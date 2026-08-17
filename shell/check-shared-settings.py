#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# check-shared-settings.py: validate the shipped settings.shared.json template
# against the tracked permissions.shared.json and the repo layout. Confirms the
# permissions block matches, no model is pinned, the prompt defaults are set, no personal
# keys leaked in, and every hook command resolves to a file inside the repo.
#
# Run:  python3 shell/check-shared-settings.py TEMPLATE PERMISSIONS REPO_ROOT
# Exit: 0 if the template is valid, non-zero (message on stderr) otherwise.

import sys
import os
import json


def die(msg):
    print(f"check-shared-settings: {msg}", file=sys.stderr)
    sys.exit(1)


def main():
    if len(sys.argv) != 4:
        die("usage: check-shared-settings.py TEMPLATE PERMISSIONS REPO_ROOT")

    template_path = sys.argv[1]
    permissions_path = sys.argv[2]
    repo_root = sys.argv[3]

    if not template_path or not permissions_path or not repo_root:
        die("usage: check-shared-settings.py TEMPLATE PERMISSIONS REPO_ROOT")

    if not os.access(template_path, os.R_OK):
        die(f"template not readable: {template_path}")
    if not os.access(permissions_path, os.R_OK):
        die(f"permissions not readable: {permissions_path}")
    if not os.path.isdir(repo_root):
        die(f"repo root is not a directory: {repo_root}")

    try:
        with open(template_path, encoding="utf-8") as fh:
            template = json.load(fh)
    except (json.JSONDecodeError, OSError):
        die(f"template is not valid JSON: {template_path}")

    try:
        with open(permissions_path, encoding="utf-8") as fh:
            permissions = json.load(fh)
    except (json.JSONDecodeError, OSError):
        die(f"permissions is not valid JSON: {permissions_path}")

    # The permissions file must itself be a JSON object.
    if not isinstance(permissions, dict):
        die(f"permissions file is not a JSON object: {permissions_path}")

    # .permissions must exist, be an object, and deep-equal the permissions file.
    if not isinstance(template.get("permissions"), dict):
        die(f".permissions is missing or not an object in {template_path}")

    if template["permissions"] != permissions:
        die(f".permissions in template does not deep-equal {permissions_path}")

    # The seed must NOT pin a model.
    if "model" in template:
        die(f".model must not ship in {template_path} (the harness or user picks the model)")

    # Shipped defaults.
    if template.get("skipAutoPermissionPrompt") is not False:
        die(f".skipAutoPermissionPrompt must be false in {template_path}")

    # Personal keys must never ship in the public template.
    for key in ("effortLevel", "theme", "preferredNotifChannel", "prefersReducedMotion"):
        if key in template:
            die(f"personal key must be absent from template: {key}")

    # Every hook command must resolve to a file inside the repo (rtk is external).
    hooks = template.get("hooks") or {}
    for event_entries in hooks.values():
        if not isinstance(event_entries, list):
            continue
        for entry in event_entries:
            if not isinstance(entry, dict):
                continue
            for hook in (entry.get("hooks") or []):
                if not isinstance(hook, dict):
                    continue
                cmd = hook.get("command")
                if not isinstance(cmd, str):
                    continue
                # Strip an optional "bash " wrapper.
                if cmd.startswith("bash "):
                    cmd = cmd[5:]
                # rtk and playbook are external tool wrappers resolved on
                # PATH, not repo files.
                if cmd.startswith("rtk") or cmd.startswith("playbook"):
                    continue
                # Resolve either ~/.claude/ or literal $HOME/.claude/ to a repo-relative path.
                rel = cmd
                if rel.startswith("~/.claude/"):
                    rel = rel[len("~/.claude/"):]
                elif rel.startswith("$HOME/.claude/"):
                    rel = rel[len("$HOME/.claude/"):]
                full_path = os.path.join(repo_root, rel)
                if not os.path.exists(full_path):
                    die(
                        f"hook command path not found under repo root:"
                        f" '{cmd}' (looked for {full_path})"
                    )

    print(f"check-shared-settings: OK ({template_path})")


if __name__ == "__main__":
    main()
