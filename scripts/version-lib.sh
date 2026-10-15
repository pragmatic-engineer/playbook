# shellcheck shell=bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# version-lib.sh: the version rules that bump-version.sh and release-plan.sh
# share (ADR 0038). Source it; it defines no behavior of its own.

# A plain MAJOR.MINOR.PATCH, with no leading zeros.
# shellcheck disable=SC2034
PLAIN_VERSION='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

# True when $1 is strictly greater than $2 (both MAJOR.MINOR.PATCH).
version_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}
