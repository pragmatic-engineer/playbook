// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ports the retired shell original: denies an `rm` whose target sits outside
//! the safe roots, `~/.claude/**`, and the scratch trees (contents only, never
//! the tree root itself or a parent of it): `/tmp`, `/private/tmp`, the macOS
//! per-user temp folder `/private/var/folders/*/*/T`, `~/.cache`, the shared
//! `.worktrees` folder next to the repo (or next to the main checkout when run
//! inside a linked worktree), `WORKTREE_BASE_DIR` when set, and the
//! `hooks.rmGuard.allowedRoots` config list.
//!
//! Targets are judged after resolving symlinks and `..` against the real
//! filesystem, so a link inside an allowed root that leaves it is denied. A
//! glob directly under a scratch root (`/tmp/*`) is denied because it would take
//! out other programs' files; a named prefix of at least three literal
//! characters (`/tmp/pb-*`) is allowed. A glob anywhere else under a scratch
//! root is denied because its expansions cannot be resolved. Under the repo and
//! the configured safe roots a glob is judged as before.
//!
//! ACCEPTED RISKS, pinned here: a check-then-use race (a link created between
//! the guard and the `rm`) cannot be closed by a pre-execution hook; the
//! `.worktrees` allowance covers every repo's folder there, not only this
//! repo's; `rmdir`, `unlink` and `find -delete` are not seen, as before.
//!
//! Best-effort protection against an accidental `rm`, NOT a security boundary.
//! It only sees `rm`, so `find -delete`, `unlink` and `>` truncation pass, and
//! anything it cannot resolve is blocked rather than evaluated.
//!
//! Four deliberate conservative blocks. Three are carried over unchanged: a
//! `cd` anywhere makes relative targets unresolvable, a command substitution
//! could expand to anything, and a quoted path containing a space still splits
//! into two tokens and is judged as two paths. The fourth was added later: a
//! target containing `$` or a backtick is unresolvable, because it expands at
//! runtime to a path the guard never sees. That one fixed a FAIL-OPEN rather
//! than tightening an existing block, so it is the one to be careful about
//! reverting.
//!
//! A heredoc body is DATA, not commands, so a mention inside one is prose
//! unless it starts a line. That was previously an accepted false positive,
//! revised on 2026-08-23 because commit messages written through a heredoc were
//! blocking real work. A body line that STARTS with a deletion is still in
//! command position and still blocks, which is what keeps `bash <<EOF` honest.
//!
//! **ACCEPTED MISS**, pinned so it is not re-reported as a vulnerability later:
//! a command name that is obfuscated or built at runtime is not resolved, so it
//! is not recognised as a deletion. Escaped and quote-split spellings, and names
//! produced by command or parameter substitution, all fall in this class.
//! Closing it needs word expansion, which for the substitution cases cannot be
//! done statically at all. This is deliberate and in scope for a guard that
//! exists to catch an ACCIDENT: no agent writes an obfuscated command name by
//! accident, and the ordinary forms it would write, including `sudo`, `xargs`,
//! `/bin/rm`, env prefixes and multi-line commands, are all still caught.
//!
//! **One documented divergence: JSON formatting.** This is the only guard that
//! never sourced `lib/common.sh`; it piped through `jq -n`, so its output was
//! pretty-printed while every other hook emits compact JSON. The port uses the
//! shared `emit_pre_deny`, making the bytes differ but the parsed object
//! identical, verified across all 22 scenarios. Reproducing jq's indentation
//! would mean a second emitter used by exactly one hook, for a consumer that
//! parses the JSON either way.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use std::path::{Path, PathBuf};

/// Placeholder reported when `rm` is reached through `$(...)` or a backtick.
const SUBSTITUTION_LABEL: &str = "<command substitution>";

const SEPARATORS: [&str; 5] = [";", "&&", "||", "|", "&"];

/// Scratch space allowed regardless of `PLAYBOOK_SAFE_ROOTS`, the same standing
/// exemption `~/.claude` already has.
///
/// BOTH spellings are listed because `canon` is lexical and never resolves a
/// symlink: on macOS `/tmp` is a symlink to `/private/tmp`, so listing one would
/// allow `/tmp/x` while still blocking the identical `/private/tmp/x`.
///
/// The temp root ITSELF stays blocked, unlike a configured safe root, which may
/// be deleted whole. `rm -rf /tmp` takes out sockets and runtime state that live
/// processes depend on, which is the class of accident this guard exists to
/// stop, and no ordinary cleanup needs it.
const TEMP_ROOTS: [&str; 2] = ["/tmp", "/private/tmp"];

pub fn run(payload: &Payload) {
    let cmd = payload.field(".tool_input.command");
    if cmd.is_empty() {
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let claude_dir = resolve_existing(&PathBuf::from(&home).join(".claude"));
    let safe_roots = safe_roots();
    let scratch_roots = scratch_roots(&home);

    let outside = offending_targets(&cmd, &home, &claude_dir, &safe_roots, &scratch_roots);
    if outside.is_empty() {
        return;
    }

    let roots = if safe_roots.is_empty() {
        "(no safe roots configured)".to_string()
    } else {
        safe_roots
            .iter()
            .map(|r| format!("{}/**", r.display()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    // Comma with no space: the shell built this with `IFS=', '` and `${arr[*]}`,
    // which joins on the FIRST character of IFS only. Matching bash's quirk
    // rather than the prettier form keeps the message byte-identical.
    let scratch = scratch_roots
        .iter()
        .map(|r| format!("{}/*", r.display()))
        .chain(std::iter::once(MACOS_TEMP_LABEL.to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    let state_root = crate::common::paths::playbook_root();
    let state_root = state_root.to_string_lossy();
    let hint = if outside.iter().any(|t| t.ends_with(".lock")) {
        " A leftover `.lock` directory clears itself on the next write once it is 10 seconds old, so do not rm it."
    } else if outside.iter().any(|t| t.starts_with(state_root.as_ref())) {
        " This path is playbook's own state: to clear plan scratch files use `playbook plan cleanup <slug>` (or `playbook plan cleanup --stale` for old leftovers), do not rm them."
    } else if outside.iter().any(|t| t.contains("/.worktrees/")) {
        " This path looks like a worktree leftover: use `git worktree remove --force <path>` for a registered one, `git worktree prune` for a deleted one, and an empty leftover folder is cleared by the next worktree create."
    } else {
        ""
    };
    emit_pre_deny(&format!(
        "rm blocked: {} is outside {roots}, ~/.claude/** and the scratch roots {scratch} (contents only, symlinks resolved, no bare glob such as /tmp/* but a named prefix such as /tmp/pb-* is fine). Allowed: rm inside the repo or those roots. Extra roots go in the `hooks.rmGuard.allowedRoots` config key. For scratch cleanup, create and delete a repo-local temp dir (for example `mktemp -d ./.tmp-XXXX`), or ask the user to delete the path. If the label is `<command substitution>`, a `$VAR` or a relative path after `cd`, the guard cannot resolve it: run the rm as its own call with a literal absolute path.{hint}",
        outside.join(",")
    ));
}

/// `PLAYBOOK_SAFE_ROOTS` is colon-separated like `$PATH`. Unset or empty falls
/// back to the git repo root, then the cwd. `$HOME` is deliberately not the
/// default, since that would unblock `~/.ssh` and `~/.aws`.
fn safe_roots() -> Vec<PathBuf> {
    let configured = std::env::var("PLAYBOOK_SAFE_ROOTS").unwrap_or_default();
    if configured.is_empty() {
        let root = git_repo_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        return vec![resolve_existing(&root)];
    }
    configured
        .split(':')
        .filter(|r| !r.is_empty())
        .map(|r| resolve_existing(Path::new(r)))
        .collect()
}

/// Label for the macOS per-user temp folder in the denial message.
const MACOS_TEMP_LABEL: &str = "/private/var/folders/*/*/T/*";

/// The directories whose CONTENTS may be deleted, resolved through symlinks.
/// The macOS per-user temp folder is matched by shape in `strict_base`, because
/// its name is a hash.
fn scratch_roots(home: &str) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = TEMP_ROOTS
        .iter()
        .map(|r| resolve_existing(Path::new(r)))
        .collect();
    if !home.is_empty() {
        // Derived from HOME, deliberately NOT from `XDG_CACHE_HOME`, which could
        // be set to `/`.
        roots.push(resolve_existing(&Path::new(home).join(".cache")));
    }
    let repo = git_repo_root();
    if let Some(parent) =
        main_checkout(repo.as_deref()).and_then(|m| m.parent().map(Path::to_path_buf))
    {
        roots.push(resolve_existing(&parent.join(".worktrees")));
    }
    if let Ok(base) = std::env::var("WORKTREE_BASE_DIR") {
        if base.starts_with('/') {
            roots.push(resolve_existing(Path::new(&base)));
        }
    }
    roots.extend(configured_roots(home));
    roots.sort();
    roots.dedup();
    // A root that is `/`, the home directory or a parent of it would turn
    // "contents only" into "nearly everything", so it is dropped, whatever
    // produced it.
    let home_real = resolve_existing(Path::new(home));
    roots
        .retain(|r| r.components().count() >= 2 && !(home_real.starts_with(r) && !home.is_empty()));
    roots
}

/// The main checkout of the repo, which is the repo itself outside a linked
/// worktree. `.worktrees` is a sibling of THIS one.
fn main_checkout(repo: Option<&Path>) -> Option<PathBuf> {
    let repo = repo?;
    let porcelain = crate::common::git::raw(repo, &["worktree", "list", "--porcelain"])?;
    crate::cc::worktree::main_worktree(&porcelain)
        .map(PathBuf::from)
        .or_else(|| Some(repo.to_path_buf()))
}

/// `hooks.rmGuard.allowedRoots`, `~/` expanded. An unreadable or malformed
/// value yields nothing, which only narrows the allowance.
fn configured_roots(home: &str) -> Vec<PathBuf> {
    if home.is_empty() {
        return Vec::new();
    }
    let home_path = Path::new(home);
    let slug = crate::common::mode::hook_slug(home_path);
    let Ok((value, _)) =
        crate::config::resolve("hooks.rmGuard.allowedRoots", home_path, slug.as_deref())
    else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|t| !t.contains(['*', '?', '[', '{', '$', '`']))
        .filter_map(|t| match t.strip_prefix("~/") {
            Some(rest) => Some(home_path.join(rest)),
            None => t.starts_with('/').then(|| PathBuf::from(t)),
        })
        .map(|p| resolve_existing(&p))
        .collect()
}

fn git_repo_root() -> Option<PathBuf> {
    if let Some(top) = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::common::gitfacts::toplevel(&cwd))
    {
        return Some(top);
    }
    let args = ["rev-parse", "--show-toplevel"];
    let path = crate::common::git::trimmed(crate::common::git::run(
        None,
        &args,
        crate::common::git::TIMEOUT,
    ))?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Resolves `.`, `..` and every symlink along the path, the way the kernel
/// would, but tolerating missing components: the target of an `rm` may not exist,
/// and a missing component can hold no link. `..` is applied AFTER a link is
/// followed, so `/tmp/link/../x` is judged where it really points. `None` means
/// the link chain is too deep to trust.
fn resolve(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut pending: std::collections::VecDeque<String> = absolute
        .to_string_lossy()
        .split('/')
        .map(str::to_string)
        .collect();
    let mut stack: Vec<String> = Vec::new();
    let mut follows = 0usize;
    while let Some(seg) = pending.pop_front() {
        match seg.as_str() {
            "" | "." => continue,
            ".." => {
                stack.pop();
                continue;
            }
            _ => stack.push(seg),
        }
        let here = PathBuf::from(format!("/{}", stack.join("/")));
        let is_link = std::fs::symlink_metadata(&here).is_ok_and(|m| m.file_type().is_symlink());
        if !is_link {
            continue;
        }
        follows += 1;
        if follows > 40 {
            return None;
        }
        let target = std::fs::read_link(&here).ok()?;
        stack.pop();
        if target.is_absolute() {
            stack.clear();
        }
        for part in target.to_string_lossy().rsplit('/') {
            pending.push_front(part.to_string());
        }
    }
    Some(PathBuf::from(format!("/{}", stack.join("/"))))
}

/// `resolve` for a root, where an unresolvable chain falls back to a path that
/// matches nothing real, so a loop cannot widen the allowance.
fn resolve_existing(path: &Path) -> PathBuf {
    resolve(path).unwrap_or_else(|| PathBuf::from("/nonexistent-unresolvable-root"))
}

/// The scratch root `path` is at or below, or `None`. The macOS per-user temp
/// folder is matched by shape: `/private/var/folders/<xx>/<hash>/T`.
fn strict_base(path: &Path, scratch_roots: &[PathBuf]) -> Option<PathBuf> {
    if let Some(root) = scratch_roots.iter().find(|r| path.starts_with(r)) {
        return Some(root.clone());
    }
    let parts: Vec<_> = path.components().collect();
    let temp_shape =
        parts.len() >= 7 && path.starts_with("/private/var/folders") && parts[6].as_os_str() == "T";
    temp_shape.then(|| parts[..7].iter().collect())
}

fn is_glob_char(c: char) -> bool {
    matches!(c, '*' | '?' | '[' | '{')
}

/// Literal characters before the first glob character of one path component.
fn literal_prefix_len(component: &str) -> usize {
    component.chars().take_while(|c| !is_glob_char(*c)).count()
}

fn is_allowed(
    target: &str,
    home: &str,
    claude_dir: &Path,
    safe_roots: &[PathBuf],
    scratch_roots: &[PathBuf],
) -> bool {
    // One pair of surrounding quotes is shell syntax; any other quote or a
    // backslash means the shell would see a different word than this one.
    let target = ["\"", "'"]
        .iter()
        .find_map(|q| target.strip_prefix(q).and_then(|t| t.strip_suffix(q)))
        .unwrap_or(target);
    if target.contains(['"', '\'', '\\']) {
        return false;
    }
    // Only `~` and `~/...` are the caller's home. `~other` is somebody else's.
    let expanded = if target == "~" || target.starts_with("~/") {
        if home.is_empty() {
            return false;
        }
        format!("{home}{}", &target[1..])
    } else if target.starts_with('~') {
        return false;
    } else {
        target.to_string()
    };

    let trailing_slash = expanded.len() > 1 && expanded.ends_with('/');
    let parts: Vec<&str> = expanded.split('/').collect();
    let glob_at = parts.iter().position(|p| p.chars().any(is_glob_char));
    let literal_len = glob_at.unwrap_or(parts.len());
    // A `..` after a glob cannot be resolved, because the expansion may be a link.
    if glob_at.is_some_and(|g| parts[g..].contains(&"..")) {
        return false;
    }
    let literal = parts[..literal_len].join("/");
    let literal = if literal.is_empty() && expanded.starts_with('/') {
        "/".to_string()
    } else {
        literal
    };
    let Some(dir) = resolve(Path::new(&literal)) else {
        return false;
    };

    if dir.starts_with(claude_dir) || safe_roots.iter().any(|root| dir.starts_with(root)) {
        return true;
    }
    let Some(base) = strict_base(&dir, scratch_roots) else {
        return false;
    };
    let Some(g) = glob_at else {
        // Contents only: the root itself and its parents stay blocked.
        return dir != base;
    };
    // A glob under a scratch root: the last component only, no trailing slash,
    // and directly under the root it needs a named prefix.
    let last = g == parts.len() - 1 && !trailing_slash;
    last && (dir != base || literal_prefix_len(parts[g]) >= 3)
}

/// The delimiter of a heredoc opened on this line, if any.
///
/// The delimiter must start with a letter or `_`, which is what keeps an
/// arithmetic shift like `$((1 << 2))` from being read as a heredoc.
fn heredoc_delimiter(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '<' if !in_single && !in_double && chars.get(i + 1) == Some(&'<') => {
                let mut j = i + 2;
                if chars.get(j) == Some(&'-') {
                    j += 1;
                }
                while chars.get(j) == Some(&' ') {
                    j += 1;
                }
                let quote = match chars.get(j) {
                    Some(&q @ ('\'' | '"')) => {
                        j += 1;
                        Some(q)
                    }
                    _ => None,
                };
                let start = j;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let word: String = chars[start..j].iter().collect();
                let named = word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_');
                let closed = quote.is_none_or(|q| chars.get(j) == Some(&q));
                if named && closed {
                    return Some(word);
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Marks which lines are heredoc BODY, which is data rather than commands.
///
/// Heredoc mode is only entered when the terminator actually appears later. An
/// unterminated `<<` would otherwise turn the rest of the command into data and
/// hide a deletion, which is the wrong way for this to fail.
fn heredoc_body_lines(cmd: &str) -> Vec<bool> {
    let lines: Vec<&str> = cmd.lines().collect();
    let mut body = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if let Some(delim) = heredoc_delimiter(lines[i]) {
            if let Some(end) = lines
                .iter()
                .enumerate()
                .skip(i + 1)
                .find(|(_, l)| l.trim() == delim)
                .map(|(j, _)| j)
            {
                for flag in body.iter_mut().take(end).skip(i + 1) {
                    *flag = true;
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    body
}

/// Splits on spaces, keeping quote characters in the token text, and reports
/// whether each token is DATA rather than a command: either inside shell quotes
/// or inside a heredoc body. A newline becomes a `;` token, so the existing
/// command-position rule still sees the start of each body line.
///
/// Splitting continues INSIDE quotes, which is deliberate: identical tokens mean
/// target judging is byte-for-byte unchanged, and it is what still catches
/// `sh -c "cd /x && rm -rf /etc"`.
///
/// An unbalanced quote leaves the remaining tokens marked as data. That can only
/// make the guard demand a separator before believing a deletion, never fewer
/// checks on the unquoted path, so the failure direction is unchanged.
fn tokenize(cmd: &str) -> Vec<(String, bool)> {
    let body = heredoc_body_lines(cmd);
    let mut tokens = Vec::new();
    let (mut in_single, mut in_double) = (false, false);

    for (index, line) in cmd.lines().enumerate() {
        if index > 0 {
            // A newline resets in-rm state exactly as `;` does.
            tokens.push((";".to_string(), false));
        }
        let is_body = body.get(index).copied().unwrap_or(false);
        let mut current = String::new();
        let mut current_data = false;

        for ch in line.replace('\t', " ").chars() {
            if ch == ' ' {
                if !current.is_empty() {
                    tokens.push((std::mem::take(&mut current), current_data));
                }
                continue;
            }
            if current.is_empty() {
                current_data = is_body || in_single || in_double;
            }
            current.push(ch);
            // Quote state is NOT tracked inside a heredoc body: the body is
            // literal text, and an ordinary apostrophe in prose would otherwise
            // flip the state and mis-mark every token after it.
            if !is_body {
                match ch {
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    _ => {}
                }
            }
        }
        if !current.is_empty() {
            tokens.push((current, current_data));
        }
    }
    tokens
}

/// Walks the command left to right tracking whether the cursor sits inside an
/// `rm`'s argument list, which is what lets `rm a && ls b` judge only `a`.
fn offending_targets(
    cmd: &str,
    home: &str,
    claude_dir: &Path,
    safe_roots: &[PathBuf],
    scratch_roots: &[PathBuf],
) -> Vec<String> {
    let mut outside = Vec::new();
    let mut in_rm = false;
    let mut saw_cd = false;
    let mut saw_rm = false;
    // A substitution only matters when it shares a simple command with an `rm`.
    let mut segment_has_rm = false;
    let mut segment_has_substitution = false;
    let mut substitution_in_rm_command = false;
    // Open `$(` groups and an open backtick span. A command inside one is part
    // of a substitution whose separators must not hide it.
    let mut paren_depth = 0usize;
    let mut in_backtick = false;
    // The very start of a command is a command position, same as after a `;`.
    let mut prev_was_separator = true;

    for (token, is_data) in tokenize(cmd) {
        let whole = token.as_str();
        let mut token = whole;
        // A substitution opens a command position: `$(cd` is `cd` and
        // `x=$(rm` is `rm`.
        if let Some(at) = token.rfind("$(") {
            token = &token[at + 2..];
            prev_was_separator = true;
        } else if let Some(at) = token.rfind('`') {
            token = &token[at + 1..];
            prev_was_separator = true;
        }
        paren_depth += whole.matches("$(").count();
        in_backtick ^= whole.matches('`').count() % 2 == 1;
        let inside_substitution = paren_depth > 0 || in_backtick;
        paren_depth = paren_depth.saturating_sub(whole.matches(')').count());

        // A word that is DATA, meaning inside quotes or inside a heredoc body,
        // is prose unless a separator put it in command position. `-m "fix: stop
        // using rm"` is a message and a commit message mentioning `rm` is prose,
        // while `sh -c "cd /x && rm -rf /etc"` and a heredoc line that STARTS
        // with `rm` really do delete. Outside those regions nothing changes, so
        // `sudo rm`, `xargs rm` and `find -exec rm` are all still caught.
        let is_command = !is_data || prev_was_separator;

        if is_command && (token == "cd" || token.ends_with("/cd")) {
            saw_cd = true;
            prev_was_separator = false;
            continue;
        }
        if is_command && (token == "rm" || token.ends_with("/rm")) {
            in_rm = true;
            saw_rm = true;
            segment_has_rm = true;
            substitution_in_rm_command |= segment_has_substitution || inside_substitution;
            prev_was_separator = false;
            continue;
        }
        if SEPARATORS.contains(&token) {
            in_rm = false;
            segment_has_rm = false;
            segment_has_substitution = false;
            prev_was_separator = true;
            continue;
        }
        prev_was_separator = false;
        if whole.contains("$(") || whole.contains('`') {
            segment_has_substitution = true;
            substitution_in_rm_command |= segment_has_rm;
        }
        if !in_rm || token.starts_with('-') {
            continue;
        }
        // A target carrying `$` or a backtick expands at runtime to a path the
        // guard cannot see, so it is unresolvable in the same way a relative
        // target after a `cd` is.
        //
        // This one closes a FAIL-OPEN, which is why it is worth the false
        // positives it adds. `canon` treats a leading `$` as a relative path and
        // joins it to the cwd, so `rm -rf "$HOME/.cache/x"` resolved to
        // `<repo>/$HOME/.cache/x`, landed inside a safe root, and was ALLOWED.
        // The shell then expanded it to a real path outside the workspace, which
        // is precisely the accident this guard exists to prevent, and agents
        // write `$VAR` paths as a matter of course.
        //
        // The cost is that `rm -rf "$REPO/target"` now blocks too, even though
        // it would have expanded to somewhere allowed. That is the correct
        // direction to be wrong in: the caller can retry with a literal path.
        let unresolvable = whole.contains('$')
            || whole.contains('`')
            || (saw_cd && !token.starts_with('/') && !token.starts_with('~'));
        if unresolvable || !is_allowed(token, home, claude_dir, safe_roots, scratch_roots) {
            outside.push(token.to_string());
        }
    }

    if saw_rm && substitution_in_rm_command {
        outside.push(SUBSTITUTION_LABEL.to_string());
    }
    outside
}
