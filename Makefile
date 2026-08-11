# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Regenerate the tracked, conservative settings.shared.json template from the
# owner's live ~/.claude/settings.json. Run after editing settings.json or
# permissions.shared.json:
#
#   make settings.shared.json
#
# Override the source with SRC=... for testing.

SHELL := /bin/bash
SRC   ?= $(HOME)/.claude/settings.json
GEN   := shell/gen-shared-settings.py
PERMS := permissions.shared.json

.PHONY: settings.shared.json
settings.shared.json:
	@test -r "$(SRC)" || { echo "make: source settings not readable: $(SRC); nothing regenerated" >&2; exit 1; }
	@python3 "$(GEN)" "$(SRC)" "$(PERMS)" > "$@.tmp" && mv "$@.tmp" "$@"
	@echo "make: regenerated $@ from $(SRC)"
