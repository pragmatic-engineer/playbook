#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# check-manifest.sh: guard the tracked-file allowlist so runtime state or a
# re-tracked personal settings.json cannot leak into the public repo. Every
# tracked path must be an allowlisted top-level file or live under an
# allowlisted top-level directory.
#
# Run:  bash shell/check-manifest.sh [REPO_ROOT]
# Exit: 0 if every tracked file is allowlisted, non-zero (offenders on stderr)
#       otherwise.
set -u

die() { echo "check-manifest: $*" >&2; exit 1; }

command -v jq >/dev/null 2>&1 || die "jq is required but was not found on PATH"

REPO_ROOT="${1:-}"
if [[ -z "$REPO_ROOT" ]]; then
  REPO_ROOT="$(git -C . rev-parse --show-toplevel 2>/dev/null)" \
    || die "not inside a git repository and no REPO_ROOT argument given"
fi
[[ -n "$REPO_ROOT" ]] || die "could not determine repo root"
[[ -d "$REPO_ROOT" ]] || die "repo root is not a directory: $REPO_ROOT"
git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
  || die "not a git repository: $REPO_ROOT"

# Allowlisted top-level files and directories.
ALLOW_FILES=(.gitignore README.md LICENSE Brewfile Makefile install.sh uninstall.sh \
  settings.shared.json permissions.shared.json statusline.sh ruff.toml \
  CODE_OF_CONDUCT.md CONTRIBUTING.md SECURITY.md Cargo.toml Cargo.lock)
ALLOW_DIRS=(prompts skills commands agents hooks shell docs output-styles .github .claude-plugin src)

in_list() {
  local needle="$1"; shift
  local item
  for item in "$@"; do
    [[ "$item" == "$needle" ]] && return 0
  done
  return 1
}

offenders=()
total=0
while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  total=$(( total + 1 ))
  # Belt-and-suspenders: the owner's personal settings.json must never track.
  if [[ "$path" == "settings.json" ]]; then
    offenders+=("$path  (personal settings.json must not be tracked)")
    continue
  fi
  if [[ "$path" == */* ]]; then
    top="${path%%/*}"
    in_list "$top" "${ALLOW_DIRS[@]}" || offenders+=("$path")
  else
    in_list "$path" "${ALLOW_FILES[@]}" || offenders+=("$path")
  fi
done < <(git -C "$REPO_ROOT" ls-files)

if (( ${#offenders[@]} > 0 )); then
  {
    echo "check-manifest: ${#offenders[@]} tracked file(s) outside the allowlist:"
    for o in "${offenders[@]}"; do
      echo "  $o"
    done
  } >&2
  exit 1
fi

echo "check-manifest: OK ($total tracked files, all allowlisted)"
