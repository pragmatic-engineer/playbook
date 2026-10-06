#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-binary-backup.test.sh: hermetic tests for the previous-binary backup
# that install.sh's install_release_binary keeps when it replaces an existing
# $PLAYBOOK_BIN_DIR/playbook, and for uninstall.sh removing those backups.
#
# A stub curl serves a fake release and a stub uname pins the platform, as in
# install-resolve.test.sh, so the shipped function runs unmodified. The run
# always dies after the binary is placed (the stub serves no source tarball),
# which is fine: the binary and its backup are written before that point.
#
# Run:  bash shell/install-binary-backup.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL="$SCRIPT_DIR/../install.sh"
UNINSTALL="$SCRIPT_DIR/../uninstall.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

STUB_BIN="$WORK/bin"
mkdir -p "$STUB_BIN"

# Serves the releases API, the asset, and SHA256SUMS from the STUB_* variables.
cat > "$STUB_BIN/curl" <<'STUB'
#!/usr/bin/env bash
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*) shift ;;
    *)  url="$1"; shift ;;
  esac
done
emit() { if [ -n "$out" ]; then printf '%s' "$1" > "$out"; else printf '%s' "$1"; fi; }
case "$url" in
  *api.github.com/repos/*/releases/latest)
    emit "$STUB_BODY"
    if [ -n "$out" ]; then printf '200'; fi
    exit 0 ;;
  */releases/download/*/SHA256SUMS) emit "$STUB_SUMS_BODY"; exit 0 ;;
  */releases/download/*)            emit "$STUB_ASSET_BODY"; exit 0 ;;
  *) exit 22 ;;
esac
STUB
chmod +x "$STUB_BIN/curl"

cat > "$STUB_BIN/uname" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  -s) printf 'Linux\n' ;;
  -m) printf 'x86_64\n' ;;
  *) command -p uname "$@" ;;
esac
STUB
chmod +x "$STUB_BIN/uname"

# A cp that refuses to write a backup temp file and defers to the real cp for
# everything else, so only the backup step fails.
FAILCP_BIN="$WORK/failcp"
mkdir -p "$FAILCP_BIN"
cat > "$FAILCP_BIN/cp" <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do
  case "$a" in *.playbook.bak.*) exit 1 ;; esac
done
exec /bin/cp "$@"
STUB
chmod +x "$FAILCP_BIN/cp"

_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  fi
}

# Body of a fake release binary that prints "playbook <version>". The salt
# makes two binaries of the same version differ in bytes.
fake_body() {
  printf '#!/usr/bin/env bash\necho "playbook %s"\n# %s\n' "$1" "${2:-}"
}

# Installs a fake release <version> into <bindir>. Output (stdout and stderr)
# goes to stdout. EXTRA_PATH, when set, is put ahead of the stubs.
install_ver() {
  local home="$1" bindir="$2" ver="$3" salt="${4:-}" body hash
  body="$(fake_body "$ver" "$salt")"
  hash="$(_sha256 "$body")"
  PATH="${EXTRA_PATH:+$EXTRA_PATH:}$STUB_BIN:$PATH" \
    CLAUDE_HOME="$home/.claude" HOME="$home" \
    PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
    STUB_BODY="{\"tag_name\": \"v$ver\"}" \
    STUB_ASSET_BODY="$body" \
    STUB_SUMS_BODY="$hash  playbook-$ver-x86_64-unknown-linux-musl" \
    bash "$INSTALL" --no-setup --skip-plugin 2>&1
}

new_case() { CASE_HOME="$(mktemp -d "$WORK/h.XXXXXX")"; CASE_BIN="$CASE_HOME/bin"; }

count_baks() { find "$1" -maxdepth 1 -name 'playbook.*.bak' | wc -l | tr -d ' '; }

# Gives the newest backup a distinct, increasing mtime so ordering by age is
# deterministic even when installs run within the same second.
age_baks() {
  local n="$1" f
  for f in "$CASE_BIN"/playbook.*.bak; do
    [ -e "$f" ] || continue
    [ "$(find "$f" -newermt "2021-01-01" | wc -l)" -eq 0 ] && continue
    touch -t "$(printf '20200101%02d00' "$n")" "$f"
  done
}

# 1. A first install has nothing to back up.
s_first_install_no_backup() {
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.1.0 >/dev/null
  [ -x "$CASE_BIN/playbook" ] && [ "$(count_baks "$CASE_BIN")" -eq 0 ]
}

# 2. Reinstalling over an older version keeps the old binary byte for byte,
#    mode 0755, named after its version, and installs the new one. The user is
#    told where it is and how to roll back.
s_reinstall_creates_backup() {
  local out
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.1.0 >/dev/null
  cp "$CASE_BIN/playbook" "$WORK/old-snapshot"
  out="$(install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3)"
  local bak="$CASE_BIN/playbook.1.1.0.bak"
  [ -f "$bak" ] && cmp -s "$bak" "$WORK/old-snapshot" \
    && find "$bak" -maxdepth 0 -perm 755 | grep -q . \
    && [ "$("$CASE_BIN/playbook" --version)" = "playbook 1.2.3" ] \
    && [[ "$out" == *"mv -f $bak $CASE_BIN/playbook"* ]]
}

# 3. A backup of the same version that already exists is kept untouched, not
#    replaced by a different file.
s_existing_backup_kept() {
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.1.0 >/dev/null
  printf 'KEEP' > "$CASE_BIN/playbook.1.1.0.bak"
  install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3 >/dev/null
  [ "$(cat "$CASE_BIN/playbook.1.1.0.bak")" = "KEEP" ] \
    && [ "$("$CASE_BIN/playbook" --version)" = "playbook 1.2.3" ]
}

# 4. Only the newest three backups are kept.
s_prunes_to_newest_three() {
  local n vers=(1.0.0 1.1.0 1.2.0 1.3.0 1.4.0) i
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" "${vers[0]}" >/dev/null
  for i in 1 2 3 4; do
    install_ver "$CASE_HOME" "$CASE_BIN" "${vers[$i]}" >/dev/null
    age_baks "$i"
  done
  n="$(count_baks "$CASE_BIN")"
  [ "$n" -eq 3 ] && [ ! -e "$CASE_BIN/playbook.1.0.0.bak" ] \
    && [ -f "$CASE_BIN/playbook.1.1.0.bak" ] \
    && [ -f "$CASE_BIN/playbook.1.2.0.bak" ] \
    && [ -f "$CASE_BIN/playbook.1.3.0.bak" ]
}

# 5. An old binary that cannot report its version still gets backed up, under
#    an unknown- name.
s_unrunnable_gets_unknown_backup() {
  local bak
  new_case
  mkdir -p "$CASE_BIN"
  printf '#!/bin/sh\nexit 1\n' > "$CASE_BIN/playbook"
  chmod 0755 "$CASE_BIN/playbook"
  cp "$CASE_BIN/playbook" "$WORK/broken-snapshot"
  install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3 >/dev/null
  bak="$(find "$CASE_BIN" -maxdepth 1 -name 'playbook.unknown-*.bak' | head -1)"
  [ -n "$bak" ] && cmp -s "$bak" "$WORK/broken-snapshot" \
    && [ "$("$CASE_BIN/playbook" --version)" = "playbook 1.2.3" ]
}

# 6. A backup that fails to write only warns. The new binary is still
#    installed and no half-written file is left behind.
s_failed_backup_still_installs() {
  local out
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.1.0 >/dev/null
  out="$(EXTRA_PATH="$FAILCP_BIN" install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3)"
  [[ "$out" == *"warning"*"backup"* && "$out" == *"Installed playbook 1.2.3"* ]] \
    && [ "$("$CASE_BIN/playbook" --version)" = "playbook 1.2.3" ] \
    && [ "$(count_baks "$CASE_BIN")" -eq 0 ] \
    && [ -z "$(find "$CASE_BIN" -maxdepth 1 -name '.playbook.bak.*')" ]
}

# 7. Reinstalling the identical binary makes no new backup.
s_identical_reinstall_no_backup() {
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3 >/dev/null
  install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3 >/dev/null
  [ "$(count_baks "$CASE_BIN")" -eq 0 ]
}

# 8. Uninstall removes the backups along with the binary.
s_uninstall_removes_backups() {
  new_case
  install_ver "$CASE_HOME" "$CASE_BIN" 1.1.0 >/dev/null
  install_ver "$CASE_HOME" "$CASE_BIN" 1.2.3 >/dev/null
  [ "$(count_baks "$CASE_BIN")" -eq 1 ] || return 1
  CLAUDE_HOME="$CASE_HOME/.claude" HOME="$CASE_HOME" PLAYBOOK_BIN_DIR="$CASE_BIN" \
    bash "$UNINSTALL" --yes --force >/dev/null 2>&1
  [ ! -e "$CASE_BIN/playbook" ] && [ "$(count_baks "$CASE_BIN")" -eq 0 ]
}

for s in \
  "first install leaves no backup:s_first_install_no_backup" \
  "reinstall keeps the old binary as a versioned backup:s_reinstall_creates_backup" \
  "an existing same-version backup is kept:s_existing_backup_kept" \
  "only the newest three backups are kept:s_prunes_to_newest_three" \
  "an unrunnable old binary gets an unknown backup:s_unrunnable_gets_unknown_backup" \
  "a failing backup warns and still installs:s_failed_backup_still_installs" \
  "an identical reinstall makes no backup:s_identical_reinstall_no_backup" \
  "uninstall removes the backups:s_uninstall_removes_backups" \
; do
  name="${s%%:*}"; fn="${s##*:}"
  if "$fn"; then pass "$name"; else fail "$name"; fi
done

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
