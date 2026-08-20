#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-resolve.test.sh: hermetic tests for install.sh's resolve_tarball_url
# and its release-binary fetch/verify/install pipeline.
#
# The risky branch is failure: an unreachable release API must NOT quietly
# install main, which would swap a tagged release for in-progress branch work.
# Only a genuine 404 may fall back. A stub curl first on PATH drives the status
# codes, so the shipped function runs unmodified and needs no production seam.
# The same stub also serves the release asset and SHA256SUMS URLs, and a stub
# uname drives platform detection, so the checksum, smoke-test, and platform
# branches run unmodified too.
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
#
# Also serves the release asset and SHA256SUMS URLs, from STUB_ASSET_BODY and
# STUB_SUMS_BODY, so the checksum and smoke-test scenarios run the shipped
# install_release_binary unmodified. When CURL_LOG is set, every URL fetched
# is appended to it, so a scenario that must abort before any download can
# assert the log stayed empty (or absent).
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
if [ -n "${CURL_LOG:-}" ]; then printf '%s\n' "$url" >> "$CURL_LOG"; fi
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
  */releases/download/*/SHA256SUMS)
    if [ -n "$out" ]; then printf '%s' "${STUB_SUMS_BODY:-}" > "$out"
    else printf '%s' "${STUB_SUMS_BODY:-}"; fi
    exit 0 ;;
  */releases/download/*)
    if [ -n "$out" ]; then printf '%s' "${STUB_ASSET_BODY:-}" > "$out"
    else printf '%s' "${STUB_ASSET_BODY:-}"; fi
    exit 0 ;;
  *) exit 22 ;;
esac
STUB
chmod +x "$STUB_BIN/curl"

# Stub uname. Drives platform detection deterministically, independent of the
# machine actually running the suite. Falls through to the real uname (found
# via bash's -p default PATH, bypassing the stub PATH) for anything besides
# -s/-m, which install.sh never asks for but keeps the stub a faithful proxy.
cat > "$STUB_BIN/uname" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  -s) printf '%s\n' "${STUB_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${STUB_UNAME_M:-x86_64}" ;;
  *) command -p uname "$@" ;;
esac
STUB
chmod +x "$STUB_BIN/uname"

# sha256 of a string, matching exactly what the curl stub writes to disk
# (printf '%s', no trailing newline), so fixtures can produce a checksum line
# that genuinely verifies -- or deliberately does not.
_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  fi
}

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

# The scenarios below exercise install_release_binary. A resolved v1.2.3
# release (STUB_CODE=200) is common to all of them; each supplies its own
# STUB_ASSET_BODY / STUB_SUMS_BODY. None asserts the script's overall exit
# code beyond "non-zero" or "zero": every failing scenario here dies before
# the source tarball step (which the stub does not implement, by design, so
# a scenario that reaches it is itself a sign something ran too far). Each
# points CLAUDE_HOME and PLAYBOOK_BIN_DIR at scratch dirs so a scenario that
# gets further than expected cannot touch the real ~/.claude or the real
# PLAYBOOK_BIN_DIR (see the comment on run_resolve above).
ASSET_1_2_3="playbook-1.2.3-x86_64-unknown-linux-musl"
GOOD_ASSET_BODY=$'#!/usr/bin/env bash\necho "playbook 1.2.3"\n'

# 8. A downloaded asset that does not match its SHA256SUMS line: verification
#    must abort before anything is installed.
s_checksum_mismatch() {
  local home bindir zeros out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.checksum-mismatch"
  zeros="$(printf '%064d' 0)"
  out="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$GOOD_ASSET_BODY" \
        STUB_SUMS_BODY="$zeros  $ASSET_1_2_3" \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" == *"checksum mismatch"* ]] \
    && [ ! -e "$home/.claude" ] && [ ! -e "$bindir/playbook" ]
}

# 9. SHA256SUMS is an HTML error page (a 200 with the wrong body, not a real
#    HTTP error): no line matches the asset, so it must die before ever
#    invoking the checksum tool.
s_sums_html_error() {
  local home bindir out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.sums-html"
  out="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$GOOD_ASSET_BODY" \
        STUB_SUMS_BODY='<html><body>404: Not Found</body></html>' \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" == *"no checksum line"* ]] \
    && [ ! -e "$home/.claude" ] && [ ! -e "$bindir/playbook" ]
}

# 10. A 0-byte asset whose checksum genuinely matches (sha256 of empty is
#     well known): the checksum step alone would accept this, so this proves
#     the --version smoke test catches what the checksum cannot. Verified by
#     running it: an empty-but-executable file does not fail exec on its own
#     (the kernel's ENOEXEC fallback runs it as an empty shell script, which
#     "succeeds" with no output), so the catch here is the version compare
#     seeing an empty string, not the "binary did not run" branch. Either
#     branch would prove the point; asserting on what actually happens rather
#     than what seemed plausible is the point of running this at all.
s_zero_byte_matches_checksum() {
  local home bindir empty_hash out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.zero-byte"
  empty_hash="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  out="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="" \
        STUB_SUMS_BODY="$empty_hash  $ASSET_1_2_3" \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" != *"checksum mismatch"* \
     && ( "$out" == *"did not run"* || "$out" == *"version mismatch"* ) ]] \
    && [ ! -e "$bindir/playbook" ]
}

# 11. The binary runs but reports a version other than the resolved tag.
s_version_mismatch() {
  local home bindir body hash out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.version-mismatch"
  body=$'#!/usr/bin/env bash\necho "playbook 9.9.9"\n'
  hash="$(_sha256 "$body")"
  out="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$body" \
        STUB_SUMS_BODY="$hash  $ASSET_1_2_3" \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" == *"version mismatch"* ]] \
    && [ ! -e "$bindir/playbook" ]
}

# 12. A platform this installer does not publish an asset for: must name the
#     platform and point at the Windows asset by name, since a bash installer
#     can never select it.
s_unsupported_platform() {
  local home bindir out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.unsupported"
  out="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_UNAME_S=SunOS STUB_UNAME_M=sparc64 \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" == *"SunOS"* && "$out" == *"sparc64"* \
     && "$out" == *"windows-msvc.exe"* ]]
}

# 13. Neither shasum nor sha256sum on PATH: must die at preflight, before
#     curl is ever invoked. Uses a curated PATH (bash, tar, the curl stub;
#     real system dirs deliberately excluded) so a real shasum/sha256sum
#     elsewhere on the host cannot leak in and mask the scenario.
s_no_checksum_tool() {
  local home bindir dir curl_log tool t out
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.no-cksum"
  dir="$WORK/no-cksum-bin"
  mkdir -p "$dir"
  for tool in bash tar; do
    t="$(command -v "$tool" 2>/dev/null)" || continue
    case "$t" in /*) ln -sf "$t" "$dir/$tool" ;; esac
  done
  cp "$STUB_BIN/curl" "$dir/curl"
  chmod +x "$dir/curl"
  curl_log="$WORK/no-cksum-curl.log"
  rm -f "$curl_log"
  out="$(PATH="$dir" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash CURL_LOG="$curl_log" \
        bash "$INSTALL" --no-setup 2>&1)"
  local rc=$?
  [[ $rc -ne 0 && "$out" == *"shasum or sha256sum is required"* ]] \
    && [ ! -e "$curl_log" ] && [ ! -e "$home/.claude" ]
}

# 14. A full success: the binary lands at PLAYBOOK_BIN_DIR, executable, and
#     running install twice adds the PATH line to the rc file exactly once.
#     Runs the real network path twice with the same scratch dirs; the source
#     tarball step still fails both times (the stub does not implement it),
#     which is fine, since the binary and PATH work both complete first.
s_successful_install() {
  local home bindir hash out1 path_lines
  home="$(mktemp -d "$WORK/h.XXXXXX")"
  bindir="$WORK/bin.success"
  hash="$(_sha256 "$GOOD_ASSET_BODY")"

  out1="$(PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$GOOD_ASSET_BODY" \
        STUB_SUMS_BODY="$hash  $ASSET_1_2_3" \
        bash "$INSTALL" --no-setup 2>&1)"

  PATH="$STUB_BIN:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$GOOD_ASSET_BODY" \
        STUB_SUMS_BODY="$hash  $ASSET_1_2_3" \
        bash "$INSTALL" --no-setup >/dev/null 2>&1 || true

  path_lines="$(grep -cF "$bindir" "$home/.bashrc" 2>/dev/null || true)"
  path_lines="${path_lines:-0}"

  [ -x "$bindir/playbook" ] && [ "$path_lines" -eq 1 ] \
    && [[ "$out1" == *"Installed playbook 1.2.3"* ]]
}

for s in \
  "published release resolves to its tag:s_release" \
  "no release (404) falls back to main:s_no_release" \
  "rate limit (403) aborts, never main:s_rate_limited" \
  "transport failure aborts, never main:s_transport_fail" \
  "server error (503) aborts, never main:s_server_error" \
  "200 without tag_name refuses to guess:s_200_no_tag" \
  "PLAYBOOK_REF pin skips the API:s_ref_pin" \
  "corrupted download fails checksum:s_checksum_mismatch" \
  "SHA256SUMS as an HTML error page:s_sums_html_error" \
  "0-byte binary with a matching checksum:s_zero_byte_matches_checksum" \
  "binary reports a different version than the tag:s_version_mismatch" \
  "unsupported platform names itself:s_unsupported_platform" \
  "neither shasum nor sha256sum on PATH:s_no_checksum_tool" \
  "successful install places the binary and wires PATH once:s_successful_install" \
; do
  name="${s%%:*}"; fn="${s##*:}"
  if "$fn"; then pass "$name"; else fail "$name"; fi
done

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
