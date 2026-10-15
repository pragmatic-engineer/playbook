#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# release-gh.sh: the GitHub steps of a release (ADR 0039), each safe to run
# again. GH_TOKEN must hold a token for <repo>; the release workflow passes
# the igorjs app token. Every step prints key=value lines on stdout.
#
#   release-gh.sh pr <repo> <branch> <title> <body-file> <dir> <file>...
#     Opens a pull request to the default branch of <repo> with one commit
#     that sets each <file> to its content under <dir>, and turns on
#     auto-merge. <dir> is a checkout of <repo>, and the commit starts from
#     its HEAD, which must still be the head of the default branch, so the
#     commit cannot undo a change that landed after the checkout. The commit
#     is made with createCommitOnBranch, so GitHub signs it. <branch> must be
#     release/vX.Y.Z. An open pull request from <branch> in <repo> is reused
#     (auto-merge is turned on again), and a merged one is reported as
#     merged. Prints pr=<number> and state=open|merged.
#
#   release-gh.sh wait <repo> <pr> <seconds>
#     Waits until the pull request merges, then prints sha=<merge commit>.
#     Fails when it closes without a merge or the time runs out. A failed
#     poll is retried.
#
#   release-gh.sh tag <repo> <tag> <sha>
#     Creates the lightweight tag <tag> on <sha>. A tag that already points
#     at <sha>, an annotated one included, is fine, and one that points
#     elsewhere fails. Exits 3 when GitHub refuses to create it, so the
#     caller can print the manual step.
#
#   release-gh.sh release <repo> <tag> <title> <notes-file>
#     Creates the published GitHub release for <tag> as the latest release,
#     unless a release for <tag> exists. For the downstream repositories,
#     whose release has no assets. Prints url=<release page>.
set -euo pipefail

fail() { echo "FAIL $*" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

usage() {
  echo "usage: release-gh.sh pr <repo> <branch> <title> <body-file> <dir> <file>... | wait <repo> <pr> <seconds> | tag <repo> <tag> <sha> | release <repo> <tag> <title> <notes-file>" >&2
  exit 2
}

SHA='^[0-9a-f]{40}$'

# The number of the pull request from <branch> in <repo> itself, in <state>.
# A pull request from a fork with the same branch name does not count.
find_pr() {
  gh pr list --repo "$1" --head "$2" --state "$3" --json number,isCrossRepository \
    --jq '[.[] | select(.isCrossRepository | not)][0].number // empty'
}

# Turns on auto-merge. A repository with a merge queue takes no merge
# method, and one without takes squash, its only method.
auto_merge() {
  local repo="$1" number="$2" err
  if err="$(gh pr merge "$number" --repo "$repo" --auto --squash 2>&1 >/dev/null)"; then return 0; fi
  gh pr merge "$number" --repo "$repo" --auto >/dev/null \
    || fail "could not turn on auto-merge for $repo#$number: $err"
}

pr() {
  [ "$#" -ge 6 ] || usage
  local repo="$1" branch="$2" title="$3" body="$4" dir="$5"
  shift 5
  [[ "$branch" =~ ^release/v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "branch '$branch' is not release/vX.Y.Z"
  local number base head want f
  number="$(find_pr "$repo" "$branch" open)"
  if [ -n "$number" ]; then
    auto_merge "$repo" "$number"
    printf 'pr=%s\nstate=open\n' "$number"
    return 0
  fi
  number="$(find_pr "$repo" "$branch" merged)"
  if [ -n "$number" ]; then
    printf 'pr=%s\nstate=merged\n' "$number"
    return 0
  fi

  want="$(git -C "$dir" rev-parse HEAD)"
  base="$(gh api "repos/$repo" --jq .default_branch)"
  head="$(gh api "repos/$repo/git/ref/heads/$base" --jq .object.sha)"
  [ "$head" = "$want" ] || fail "$repo $base moved from $want to $head since the checkout. Run again."
  # A branch left by a failed run is dropped, so the commit starts from the
  # checkout.
  if gh api "repos/$repo/git/ref/heads/$branch" >/dev/null 2>&1; then
    gh api -X DELETE "repos/$repo/git/refs/heads/$branch" >/dev/null
  fi
  gh api -X POST "repos/$repo/git/refs" -f "ref=refs/heads/$branch" -f "sha=$head" >/dev/null

  # jq reads each file itself, so a large file never goes through argv.
  : > "$TMP/files.jsonl"
  for f in "$@"; do
    [ -f "$dir/$f" ] || fail "$dir/$f is missing"
    jq -n -c --arg path "$f" --rawfile c "$dir/$f" '{path: $path, contents: ($c | @base64)}' >> "$TMP/files.jsonl"
  done
  jq -s --arg repo "$repo" --arg branch "$branch" --arg head "$head" --arg msg "$title" \
    '{query: "mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid } } }",
      variables: {input: {
        branch: {repositoryNameWithOwner: $repo, branchName: $branch},
        message: {headline: $msg},
        expectedHeadOid: $head,
        fileChanges: {additions: .}
      }}}' "$TMP/files.jsonl" > "$TMP/commit.json"
  local oid
  oid="$(gh api graphql --input "$TMP/commit.json" --jq '.data.createCommitOnBranch.commit.oid // empty')"
  [ -n "$oid" ] || fail "GitHub did not create the commit on $repo $branch"

  number="$(gh api -X POST "repos/$repo/pulls" -f "title=$title" -f "head=$branch" -f "base=$base" \
    -F "body=@$body" --jq .number)"
  auto_merge "$repo" "$number"
  printf 'pr=%s\nstate=open\n' "$number"
}

wait_merged() {
  [ "$#" -eq 3 ] || usage
  local repo="$1" number="$2" seconds="$3" end state sha
  [[ "$seconds" =~ ^[0-9]+$ ]] || usage
  end=$(( $(date +%s) + seconds ))
  while :; do
    state="" sha=""
    read -r state sha < <(gh pr view "$number" --repo "$repo" --json state,mergeCommit \
      --jq '"\(.state) \(.mergeCommit.oid // "")"' 2>/dev/null) || true
    case "$state" in
      MERGED)
        if [[ "$sha" =~ $SHA ]]; then printf 'sha=%s\n' "$sha"; return 0; fi ;;
      CLOSED) fail "pull request $repo#$number was closed without a merge" ;;
    esac
    [ "$(date +%s)" -lt "$end" ] || fail "pull request $repo#$number did not merge within $seconds seconds"
    sleep "${RELEASE_GH_POLL:-20}"
  done
}

tag() {
  [ "$#" -eq 3 ] || usage
  local repo="$1" tag="$2" sha="$3" ref have type err
  [[ "$sha" =~ $SHA ]] || fail "'$sha' is not a commit sha"
  # gh prints the error body on stdout, so the exit status decides, and only
  # a 404 means the tag is missing.
  if ref="$(gh api "repos/$repo/git/ref/tags/$tag" 2>/dev/null)"; then
    type="$(jq -r '.object.type' <<<"$ref")"
    have="$(jq -r '.object.sha' <<<"$ref")"
    if [ "$type" = tag ]; then
      have="$(gh api "repos/$repo/git/tags/$have" --jq .object.sha)"
    fi
    [ "$have" = "$sha" ] || fail "tag $tag in $repo points at $have, not $sha. Not moving it."
    printf 'tag=%s\nstate=exists\n' "$tag"
    return 0
  fi
  [ "$(jq -r '.status // empty' <<<"$ref" 2>/dev/null)" = 404 ] \
    || fail "could not read tag $tag in $repo: $ref"
  if ! err="$(gh api -X POST "repos/$repo/git/refs" -f "ref=refs/tags/$tag" -f "sha=$sha" 2>&1 >/dev/null)"; then
    echo "GitHub refused to create $tag in $repo: $err" >&2
    exit 3
  fi
  printf 'tag=%s\nstate=created\n' "$tag"
}

release() {
  [ "$#" -eq 4 ] || usage
  local repo="$1" tag="$2" title="$3" notes="$4" url
  url="$(gh release view "$tag" --repo "$repo" --json url --jq .url 2>/dev/null || true)"
  if [ -z "$url" ]; then
    # gh prints the page of the new release. Another run may have made it
    # in the meantime, so a failed create looks again.
    url="$(gh release create "$tag" --repo "$repo" --verify-tag --latest --title "$title" --notes-file "$notes")" \
      || url="$(gh release view "$tag" --repo "$repo" --json url --jq .url)"
  fi
  printf 'url=%s\n' "$url"
}

cmd="${1:-}"
[ "$#" -gt 0 ] && shift
case "$cmd" in
  pr) pr "$@" ;;
  wait) wait_merged "$@" ;;
  tag) tag "$@" ;;
  release) release "$@" ;;
  *) usage ;;
esac
