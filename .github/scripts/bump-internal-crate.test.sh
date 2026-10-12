#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# Behavioural tests for bump-internal-crate.sh. gh and cargo are replaced by
# stubs on PATH, so nothing touches the network or a real lockfile.
# shellcheck shell=bash
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
script="$here/bump-internal-crate.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0

mkstubs() { # $1 latest tag, $2 asset list
  mkdir -p "$work/bin"
  cat >"$work/bin/gh" <<STUB
#!/usr/bin/env bash
case "\$*" in
  *tagName*) echo "$1" ;;
  *assets*) printf '%s\n' $2 ;;
  *) exit 1 ;;
esac
STUB
  cat >"$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
echo "$*" >>"$BUMP_ROOT/cargo.log"
STUB
  chmod +x "$work/bin/gh" "$work/bin/cargo"
}

fixture() {
  rm -rf "$work/root"
  mkdir -p "$work/root"
  cat >"$work/root/Cargo.toml" <<'TOML'
[dependencies]
libc = "=0.2.189"
playbook-telemetry = { git = "https://github.com/pragmatic-engineer/playbook-telemetry", tag = "v0.1.3" }
other = { git = "https://github.com/pragmatic-engineer/other", tag = "v0.1.3" }
TOML
  export BUMP_ROOT="$work/root"
}

run() {
  PATH="$work/bin:$PATH" bash "$script" "$@" >"$work/out" 2>&1
  echo $?
}

check() { # name expected actual
  if [ "$2" = "$3" ]; then
    echo "ok   $1"
  else
    echo "FAIL $1 (want $2, got $3)"
    cat "$work/out"
    fails=$((fails + 1))
  fi
}

T=pragmatic-engineer/playbook-telemetry

fixture; mkstubs v0.1.4 "SHA256SUMS a.tar.gz"
check "bumps when latest and checksummed" 0 "$(run playbook-telemetry $T v0.1.4)"
check "rewrote the tag" 1 "$(grep -c 'playbook-telemetry.*tag = "v0.1.4"' "$work/root/Cargo.toml")"
check "left other crate alone" 1 "$(grep -c 'other.*tag = "v0.1.3"' "$work/root/Cargo.toml")"
check "ran cargo update for that crate only" "update -p playbook-telemetry" "$(cat "$work/root/cargo.log")"

fixture; mkstubs v0.1.3 "SHA256SUMS"
check "already current exits 0" 0 "$(run playbook-telemetry $T v0.1.3)"
check "already current does not run cargo" 0 "$(find "$work/root" -name cargo.log | wc -l | tr -d ' ')"

fixture; mkstubs v0.1.5 "SHA256SUMS"
check "refuses a tag that is not the latest" 1 "$(run playbook-telemetry $T v0.1.4)"
check "stale tag leaves Cargo.toml" 1 "$(grep -c 'playbook-telemetry.*v0.1.3' "$work/root/Cargo.toml")"

fixture; mkstubs v0.1.4 "a.tar.gz"
check "refuses a release without SHA256SUMS" 1 "$(run playbook-telemetry $T v0.1.4)"

fixture; mkstubs v0.1.4 "SHA256SUMS"
check "refuses an unknown crate" 1 "$(run missing $T v0.1.4)"
check "refuses a mismatched repo" 1 "$(run playbook-telemetry pragmatic-engineer/other-repo v0.1.4)"
check "rejects a floating tag" 2 "$(run playbook-telemetry $T latest)"
check "rejects missing args" 2 "$(run playbook-telemetry)"

[ "$fails" -eq 0 ] || { echo "$fails failure(s)"; exit 1; }
echo "all passed"
