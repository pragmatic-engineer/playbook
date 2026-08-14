#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# SessionStart hook:
#   1. Prepare per-session runtime dir + zero counters.
#   2. On resume, warn when settings/hooks have drifted since session creation.
#   3. Inject the project memory slice, an auto-learn nudge, a skills primer,
#      and the async discipline reminder as SessionStart additionalContext.
#
# config_hash and the memory slice are computed by shelling out to the shared
# bash helpers (hooks/lib/config-hash.sh, shell/memory-context.sh) so there is
# a single source of truth across the bash launcher and this python hook.

import json
import os
import re
import subprocess
import sys

HOOK_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HOOK_DIR, "lib"))
import common as c  # noqa: E402

HOME = os.environ.get("HOME") or os.path.expanduser("~")


def _run(cmd, **kw):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=15, **kw)
        return r
    except Exception:
        return None


def config_hash():
    path = os.path.join(HOOK_DIR, "lib", "config-hash.sh")
    r = _run(["bash", "-c", '. "$1"; config_hash', "_", path])
    return r.stdout.strip() if r and r.returncode == 0 else ""


def git_toplevel():
    r = _run(["git", "--no-optional-locks", "rev-parse", "--show-toplevel"])
    return r.stdout.strip() if r and r.returncode == 0 else ""


def git_branch():
    r = _run(["git", "--no-optional-locks", "rev-parse", "--abbrev-ref", "HEAD"])
    return r.stdout.strip() if r and r.returncode == 0 else ""


def slugify(s):
    return re.sub(r"[^A-Za-z0-9_.-]", "_", s)


def fm_field(path, field):
    """Value of a field in the first frontmatter block (mirrors the awk in the
    bash: only when line 1 is '---', up to the closing '---', a line starting
    with 'field:' has its prefix and surrounding quotes stripped)."""
    try:
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().split("\n")
    except Exception:
        return ""
    if not lines or lines[0] != "---":
        return ""
    for line in lines[1:]:
        if line == "---":
            break
        if line.startswith(field + ":"):
            val = line[len(field) + 1:]
            val = re.sub(r"^[ \t]*", "", val)
            val = re.sub(r'^"', "", val)
            val = re.sub(r'"$', "", val)
            return val
    return ""


def one_line(text):
    s = text.replace("\n", " ").replace("\t", " ")
    if len(s) > 150:
        return s[:147] + "..."
    return s


def main() -> int:
    dir_ = c.session_dir()
    if dir_:
        for name in ("search-count", "tool-count", "edit-count", "edits.jsonl", "seen-reads"):
            try:
                open(os.path.join(dir_, name), "w").close()
            except Exception:
                pass
        try:
            import time
            with open(os.path.join(dir_, "start-ts"), "w") as f:
                f.write(str(int(time.time())))
        except Exception:
            pass

    # Clear the statusline PR/CI cache for the current repo+branch so the first
    # render of each session fetches fresh data rather than reusing stale cache.
    sl_cache = os.environ.get(
        "STATUSLINE_CACHE_DIR",
        os.path.join(os.environ.get("XDG_CACHE_HOME", os.path.join(HOME, ".cache")), "statusline"),
    )
    sl_branch = git_branch()
    if sl_branch and sl_branch != "HEAD":
        sl_slug = slugify("%s::%s" % (os.getcwd(), sl_branch))
        for pref in ("pr", "ci"):
            try:
                os.remove(os.path.join(sl_cache, "%s-%s.json" % (pref, sl_slug)))
            except OSError:
                pass

    source = c.field(".source")

    system_message = ""
    extra_context = ""

    # Config hash: warn on resume if settings have drifted.
    current_hash = config_hash()
    hash_file = os.path.join(dir_, "config-hash") if dir_ else ""

    if source == "resume" and current_hash and hash_file:
        prev_hash = ""
        try:
            with open(hash_file) as f:
                prev_hash = f.read().strip()
        except Exception:
            prev_hash = ""
        if prev_hash and prev_hash != current_hash:
            system_message = (
                "⚠ Claude config (settings.json + hooks) has drifted since this "
                "session was created. Plugins, output style, model default, and new "
                "hooks will NOT take effect on this resumed session: they're frozen at "
                "the original startup snapshot. To apply current config: exit and run "
                "`cc fresh` (or `claude` without --resume)."
            )
            extra_context = (
                "The user resumed this session, but the config hash has changed since "
                "session creation. The harness has the OLD settings loaded. If the user "
                "asks about why a recent settings change isn't showing up, point them to "
                "'cc fresh' or starting a new `claude` invocation."
            )

    # Always refresh the stored hash on startup (the new baseline going forward).
    if current_hash and source == "startup" and hash_file:
        try:
            with open(hash_file, "w") as f:
                f.write(current_hash)
        except Exception:
            pass

    # Project memory slice.
    repo_root = git_toplevel()
    mem_slug = c.repo_slug()
    mem_body = ""
    mem_preamble = ""
    if repo_root and mem_slug:
        mem_script = os.path.join(HOOK_DIR, "..", "shell", "memory-context.sh")
        if os.path.isfile(mem_script):
            r = _run(["bash", mem_script, "--repo", mem_slug])
            if r and r.returncode == 0:
                mem_body = r.stdout.strip("\n")
        if mem_body:
            mem_preamble = (
                "Project memory for this repo (%s), stored in the central memory store "
                "at ~/.claude/memory/%s/. A scoped slice: facts in scope, their typed "
                "edges, and an anchor index mapping code paths to the facts that describe "
                "them. Fact bodies are read on demand." % (mem_slug, mem_slug)
            )
        else:
            legacy = os.path.join(HOME, ".claude", "memory", mem_slug, "MEMORY.md")
            if os.path.isfile(legacy):
                try:
                    with open(legacy, encoding="utf-8") as f:
                        mem_body = f.read(16000)
                except Exception:
                    mem_body = ""
                if mem_body:
                    mem_preamble = (
                        "Project memory for this repo (%s), stored in the central memory "
                        "store at ~/.claude/memory/%s/. These facts apply only in this "
                        "repo; read the referenced fact files on demand. Index:" % (mem_slug, mem_slug)
                    )
    if mem_body:
        mem_ctx = mem_preamble + "\n" + mem_body
        extra_context = extra_context + "\n\n" + mem_ctx if extra_context else mem_ctx

    # Auto-learn nudge.
    if os.environ.get("AUTO_LEARN_NUDGE", "1") != "0" and repo_root:
        qdir = os.path.join(c.RUNTIME_ROOT, "to-learn")
        _prune_old(qdir, int(os.environ.get("AUTO_LEARN_MAX_AGE_DAYS", "14")))
        learn_flag = os.path.join(qdir, slugify(repo_root) + ".json")
        if os.path.isfile(learn_flag):
            edits = "some"
            try:
                with open(learn_flag) as f:
                    edits = json.load(f).get("edits", 0)
            except Exception:
                edits = "some"
            nudge = (
                "A previous session in this repo made %s edits, so project memory may be "
                "stale. Consider running /playbook:learn-project to refresh it, or /playbook:learn-project "
                "--stage to queue candidate facts for review." % edits
            )
            extra_context = extra_context + "\n\n" + nudge if extra_context else nudge
            try:
                os.remove(learn_flag)
            except OSError:
                pass

    # Skills & commands primer.
    if os.environ.get("SKILLS_PRIMER", "1") != "0":
        skill_lines = _catalog(os.path.join(HOME, ".claude", "skills"), "skill")
        cmd_lines = _catalog(os.path.join(HOME, ".claude", "commands"), "command")
        if skill_lines or cmd_lines:
            toolkit = (
                "Your toolkit. Before substantive work, check whether one of these fits "
                "and use it instead of ad-hoc steps: plan a feature with /playbook:scope, execute a "
                "ready plan with /playbook:implement, record a decision with /playbook:adr, commit and push "
                "with /playbook:commit-and-push, open a PR with /playbook:create-pull-request, review a PR "
                "with /playbook:quick-review or /playbook:deep-review, debug a failure with the "
                "systematic-debugging skill. Invoke skills via the Skill tool, commands as "
                "slash commands. Full catalog (name: what it is for):"
            )
            if skill_lines:
                toolkit += "\n\nSkills:\n" + skill_lines
            if cmd_lines:
                toolkit += "\nCommands:\n" + cmd_lines
            extra_context = extra_context + "\n\n" + toolkit if extra_context else toolkit

    # Async & deferred-tool discipline.
    if os.environ.get("ASYNC_DISCIPLINE", "1") != "0":
        async_ctx = (
            "Async and deferred-tool discipline. (1) Deferred tools are surfaced by name "
            "only (e.g. Monitor, TaskCreate, TaskStop, TaskUpdate, ScheduleWakeup): their "
            "schemas are NOT loaded, so calling them with guessed parameters fails "
            "validation. Before calling any tool that is not already in your active tool "
            "list, load it first with ToolSearch (query \"select:NAME\"), then call it; "
            "never guess its parameters. (2) Don't run a command in the background when "
            "the next step needs its result (installs, builds, typechecks): run it in the "
            "foreground with an extended timeout (up to 600000ms). A backgrounded job "
            "re-invokes you only when it exits, and shell state (including `wait`) does not "
            "persist across Bash calls, so there is nothing to poll."
        )
        extra_context = extra_context + "\n\n" + async_ctx if extra_context else async_ctx

    # Emit a single SessionStart payload if there is anything to say.
    if system_message or extra_context:
        out = {}
        if system_message:
            out["systemMessage"] = system_message
        if extra_context:
            out["hookSpecificOutput"] = {
                "hookEventName": "SessionStart",
                "additionalContext": extra_context,
            }
        print(json.dumps(out, separators=(",", ":"), ensure_ascii=False))
    return 0


def _prune_old(qdir, max_age_days):
    try:
        if not os.path.isdir(qdir):
            return
        import time
        cutoff = time.time() - max_age_days * 86400
        for fn in os.listdir(qdir):
            if not fn.endswith(".json"):
                continue
            fp = os.path.join(qdir, fn)
            try:
                if os.path.isfile(fp) and os.path.getmtime(fp) < cutoff:
                    os.remove(fp)
            except OSError:
                pass
    except Exception:
        pass


def _catalog(root, kind):
    """Build '- name: one-line-description' lines for skills or commands."""
    lines = []
    try:
        if kind == "skill":
            if not os.path.isdir(root):
                return ""
            # Match the bash glob order: it sorts the expanded '*/SKILL.md'
            # paths, so the '/' after each name is part of the sort key.
            for entry in sorted(os.listdir(root), key=lambda e: e + "/SKILL.md"):
                sf = os.path.join(root, entry, "SKILL.md")
                if not os.path.isfile(sf):
                    continue
                nm = fm_field(sf, "name") or entry
                lines.append("- %s: %s" % (nm, one_line(fm_field(sf, "description"))))
        else:
            if not os.path.isdir(root):
                return ""
            for entry in sorted(os.listdir(root)):
                if not entry.endswith(".md"):
                    continue
                cf = os.path.join(root, entry)
                if not os.path.isfile(cf):
                    continue
                base = entry[:-3]
                lines.append("- /%s: %s" % (base, one_line(fm_field(cf, "description"))))
    except Exception:
        return ""
    return ("\n".join(lines) + "\n") if lines else ""


if __name__ == "__main__":
    sys.exit(main())
