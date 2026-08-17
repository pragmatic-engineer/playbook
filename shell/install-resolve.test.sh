#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-resolve.test.sh: hermetic tests for install.sh's resolve_tarball_url.
#
# The function picks what the installer downloads, and its risky branch is the
# failure one: if it cannot reach the GitHub release API it must NOT quietly
# install the main branch, because that swaps a tagged release for whatever
# in-progress work is on the branch. Only a genuine 404 ("this repo published
# no release") may fall back to main.
#
# Hermetic via a stub `curl` placed first on PATH, so the real function runs
# unmodified against chosen HTTP status codes. No production test seam exists
# for this and none is added: the stub exercises the shipped code path.
#
# Run:  bash shell/install-resolve.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL="$SCRIPT_DIR/../install.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Stub curl. Answers the releases API with a chosen status code and body, and
# fails every other URL, so the run always stops at the download step. That is
# deliberate: the download failure message echoes the resolved URL, which is
# exactly what these scenarios need to inspect.
STUB_BIN="$WORK/bin"
mkdir -p "$STUB_BIN"
# The stub emulates BOTH call shapes on purpose, so it is a fair judge of the
# old implementation as well as the new one:
#   with -o FILE -w '%{http_code}'  -> body to FILE, status code to stdout
#   with -f and no -o               -> body to stdout, and -f means a status of
#                                      400 or more exits non-zero with no body
# Without the second shape the stub would fail the old code for using a
# different flag set rather than for the defect under test, and the mutation
# check would prove nothing.
cat > "$STUB_BIN/curl" <<'STUB'
#!/usr/bin/env bash
out=""; url=""; fail_on_http_error=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*) case "$1" in *f*) fail_on_http_error=1 ;; esac; shift ;;
    *)  url="$1"; shift ;;
  esac
done
case "$url" in
  *api.github.com/repos/*/releases/latest)
    if [ "${STUB_TRANSPORT_FAIL:-0}" = "1" ]; then exit 7; fi
    code="${STUB_CODE:-200}"
    if [ "$fail_on_http_error" = "1" ] && [ "$code" -ge 400 ]; then exit 22; fi
    if [ -n "$out" ]; then
      printf '%s' "${STUB_BODY:-}" > "$out"
      printf '%s' "$code"
    else
      printf '%s' "${STUB_BODY:-}"
    fi
    exit 0 ;;
  *) exit 22 ;;
esac
STUB
chmod +x "$STUB_BIN/curl"

# Run the installer through the network path with the stub in front. Returns
# combined output; the caller asserts on it. HOME/CLAUDE_HOME are scratch so a
# scenario that got further than expected still cannot touch the real ~/.claude.
run_resolve() {
  local home; home="$(mktemp -d "$WORK/h.XXXXXX")"
  PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home" HOME="$home" \
    bash "$INSTALL" --no-setup 2>&1
}

RELEASE_BODY='{"tag_name": "v1.2.3", "name": "release"}'

# 1. A published release resolves to that tag's tarball.
s_release() {
  local out
  out="$(STUB_CODE=200 STUB_BODY="$RELEASE_BODY" run_resolve)"
  [[ "$out" == *"refs/tags/v1.2.3"* ]]
}

# 2. A repo with no releases (404) may fall back to main, and says so honestly.
s_no_release() {
  local out
  out="$(STUB_CODE=404 STUB_BODY='{"message":"Not Found"}' run_resolve)"
  [[ "$out" == *"refs/heads/main"* && "$out" == *"published no release"* ]]
}

# 3. Rate limited (403). The regression: this used to be indistinguishable from
#    a 404 and silently installed main. It must abort instead.
s_rate_limited() {
  local out
  out="$(STUB_CODE=403 STUB_BODY='{"message":"API rate limit exceeded"}' run_resolve)"
  [[ "$out" != *"refs/heads/main"* && "$out" == *"HTTP 403"* ]]
}

# 4. Transport failure (no network). Same rule: abort, never main.
s_transport_fail() {
  local out
  out="$(STUB_TRANSPORT_FAIL=1 run_resolve)"
  [[ "$out" != *"refs/heads/main"* && "$out" == *"HTTP 000"* ]]
}

# 5. A 5xx must abort too, so the branch is a real allowlist and not a
#    hardcoded pair of special cases.
s_server_error() {
  local out
  out="$(STUB_CODE=503 STUB_BODY='' run_resolve)"
  [[ "$out" != *"refs/heads/main"* && "$out" == *"HTTP 503"* ]]
}

# 6. A 200 carrying no tag_name is malformed. Refuse rather than guess, and in
#    particular do not treat it as "no release" and install main.
s_200_no_tag() {
  local out
  out="$(STUB_CODE=200 STUB_BODY='{"message":"weird"}' run_resolve)"
  [[ "$out" != *"refs/heads/main"* && "$out" == *"no tag_name"* ]]
}

# 7. PLAYBOOK_REF pins a ref and skips the API entirely, so a rate limit cannot
#    block an install that already names what it wants.
s_ref_pin() {
  local out home
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  out="$(PATH="$STUB_BIN:$PATH" PLAYBOOK_REF=v9.9.9 STUB_CODE=403 \
        CLAUDE_HOME="$home" HOME="$home" bash "$INSTALL" --no-setup 2>&1)"
  [[ "$out" == *"tar.gz/v9.9.9"* && "$out" != *"HTTP 403"* ]]
}

for s in \
  "published release resolves to its tag:s_release" \
  "no release (404) falls back to main:s_no_release" \
  "rate limit (403) aborts, never main:s_rate_limited" \
  "transport failure aborts, never main:s_transport_fail" \
  "server error (503) aborts, never main:s_server_error" \
  "200 without tag_name refuses to guess:s_200_no_tag" \
  "PLAYBOOK_REF pin skips the API:s_ref_pin" \
; do
  name="${s%%:*}"; fn="${s##*:}"
  if "$fn"; then pass "$name"; else fail "$name"; fi
done

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
