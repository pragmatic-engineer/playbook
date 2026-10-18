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
//! repo's; `rmdir`, `unlink`, `shred` and `>` truncation are not seen.
//!
//! Best-effort protection against an accidental `rm`, NOT a security boundary.
//! Anything it cannot resolve is blocked rather than evaluated.
//!
//! The command is read into real command words by `parse`, on the shared shell
//! lexer, so only a word in command position is a deletion. Quoted text, a
//! grep pattern, a commit message, an `echo` and a heredoc read by a program
//! other than a shell are data. `find -delete`, `find -exec rm` and
//! `xargs rm` are judged by `find`'s start paths when the hook runs, but the
//! hook is wired with `if: Bash(rm:*)`, so Claude Code only starts it for a
//! command that holds an `rm` word.
//!
//! Deliberate conservative blocks: a `cd` before the deletion makes relative
//! targets unresolvable; a deletion inside, next to or fed by a command
//! substitution is unresolvable; a target holding a variable is unresolvable,
//! except `$HOME`, which the guard expands with its own value unless the
//! command sets or unsets it. The variable rule fixed a FAIL-OPEN
//! (`"$X/y"` used to resolve under the cwd), so it is the one to be careful
//! about reverting.
//!
//! **ACCEPTED MISS**, pinned so it is not re-reported as a vulnerability later:
//! a command name built at runtime (`$cmd`, `$(echo rm)`) is not resolved, so it
//! is not recognised as a deletion. Closing it needs word expansion, which for
//! substitutions cannot be done statically. Escaped and quote-split spellings
//! (`\rm`, `r'm'`) are caught, because the lexer removes the quoting.
//!
//! **One documented divergence: JSON formatting.** This is the only guard that
//! never sourced `lib/common.sh`; it piped through `jq -n`, so its output was
//! pretty-printed while every other hook emits compact JSON. The port uses the
//! shared `emit_pre_deny`, making the bytes differ but the parsed object
//! identical, verified across all 22 scenarios. Reproducing jq's indentation
//! would mean a second emitter used by exactly one hook, for a consumer that
//! parses the JSON either way.

mod parse;

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use parse::Target;
use std::path::{Path, PathBuf};

/// Scratch space allowed regardless of `PLAYBOOK_SAFE_ROOTS`, the same standing
/// exemption `~/.claude` already has.
///
/// Both spellings are listed for readability: on macOS `/tmp` is a symlink to
/// `/private/tmp`, and `scratch_roots` resolves and deduplicates them.
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
    // Most commands that reach the hook delete nothing, so the roots, which
    // cost git and config reads, are only worked out when something does.
    let targets = parse::targets(&cmd, &parse::Env { home: &home });
    if targets.is_empty() {
        return;
    }
    let claude_dir = resolve_existing(&PathBuf::from(&home).join(".claude"));
    let safe_roots = safe_roots();
    let scratch_roots = scratch_roots(&home);

    let outside = offending_targets(targets, &home, &claude_dir, &safe_roots, &scratch_roots);
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
        "rm blocked: {} is outside {roots}, ~/.claude/** and the scratch roots {scratch} (contents only, symlinks resolved, no bare glob such as /tmp/* but a named prefix such as /tmp/pb-* is fine). Allowed: rm inside the repo or those roots. Ask the user to add extra roots to the `hooks.rmGuard.allowedRoots` config key, since you cannot change it. A stray quote or backslash left inside a path cannot be resolved. For scratch cleanup, create and delete a repo-local temp dir (for example `mktemp -d ./.tmp-XXXX`), or ask the user to delete the path. If the label is `<command substitution>`, `<xargs input>`, a variable other than `$HOME`, or a relative path after `cd`, the guard cannot resolve it: run the rm as its own call with a literal absolute path.{hint}",
        outside
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(", ")
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
    let keep_out = [
        resolve_existing(Path::new(home)),
        resolve_existing(&Path::new(home).join(".claude")),
    ];
    roots.retain(|r| {
        r.components().count() >= 2
            && (home.is_empty() || !keep_out.iter().any(|k| k.starts_with(r)))
    });
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
        .map(crate::config::write::strip_root_glob)
        .filter(|t| !t.contains(['*', '?', '[', '{', '$', '`']))
        .filter_map(|t| match t.strip_prefix("~/") {
            // `join` of an absolute remainder replaces home, so `~//etc` would
            // name `/etc`.
            Some(rest) if rest.starts_with('/') => None,
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
        std::env::current_dir().ok()?.join(path)
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
    // `canonicalize` also normalises the letter case the filesystem stores, so a
    // root spelled `/users/me` on a case-insensitive disk compares equal to home.
    std::fs::canonicalize(path)
        .ok()
        .or_else(|| resolve(path))
        .unwrap_or_else(|| PathBuf::from("/nonexistent-unresolvable-root"))
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

/// `parts` joined back into a path that `resolve` can take. An empty join of an
/// absolute path is the root, not the working directory.
fn join_parts(parts: &[&str], absolute: bool) -> String {
    let joined = parts.join("/");
    if joined.is_empty() && absolute {
        "/".to_string()
    } else {
        joined
    }
}

/// Whether deleting `target`, a word as the shell passes it, is allowed.
/// `tilde` says a leading `~` was unquoted, so the shell expands it.
fn is_allowed(
    target: &str,
    tilde: bool,
    home: &str,
    claude_dir: &Path,
    safe_roots: &[PathBuf],
    scratch_roots: &[PathBuf],
) -> bool {
    // A quote or backslash left in the word is part of the file name, which is
    // more likely a mistake than a real file. The rest are shell syntax (zsh
    // glob qualifiers, redirects) that this guard does not model.
    if target.contains(['"', '\'', '\\', '(', ')', '<', '>', '^']) {
        return false;
    }
    // Only `~` and `~/...` are the caller's home, and never inside quotes, where
    // the shell leaves `~` alone. `~other` is somebody else's.
    let expanded = if target == "~" || target.starts_with("~/") {
        if home.is_empty() || !tilde {
            return false;
        }
        format!("{home}{}", &target[1..])
    } else if target.starts_with('~') {
        return false;
    } else {
        target.to_string()
    };

    let absolute = expanded.starts_with('/');
    let trailing_slash = expanded.len() > 1 && expanded.ends_with('/');
    let parts: Vec<&str> = expanded.split('/').collect();
    // A brace group that spans a `/` (`{a,/etc/x}`) expands to a path the
    // component walk below never sees.
    if parts
        .iter()
        .any(|p| p.matches('{').count() != p.matches('}').count())
    {
        return false;
    }
    let glob_at = parts.iter().position(|p| p.chars().any(is_glob_char));
    let literal_len = glob_at.unwrap_or(parts.len());
    // A `..` after a glob cannot be resolved, because the expansion may be a link.
    if glob_at.is_some_and(|g| parts[g..].contains(&"..")) {
        return false;
    }
    let Some(dir) = resolve(Path::new(&join_parts(&parts[..literal_len], absolute))) else {
        return false;
    };

    let verdict = |dir: &Path| -> bool {
        if dir.starts_with(claude_dir) || safe_roots.iter().any(|root| dir.starts_with(root)) {
            return true;
        }
        let Some(base) = strict_base(dir, scratch_roots) else {
            return false;
        };
        let Some(g) = glob_at else {
            // Contents only: the root itself and its parents stay blocked.
            return dir != base;
        };
        // A glob under a scratch root: the last component only, no trailing
        // slash, and directly under the root it needs a named prefix.
        let last = g == parts.len() - 1 && !trailing_slash;
        last && (dir != base || literal_prefix_len(parts[g]) >= 3)
    };
    if !verdict(&dir) {
        return false;
    }
    // `rm link` removes the link itself, wherever it points, so the link's own
    // location has to be allowed as well as its destination. This is what stops
    // a link that sits outside every root, but points into one, from being
    // deleted by name.
    if glob_at.is_none() && !trailing_slash {
        if let Some(name) = parts.last().filter(|n| !matches!(**n, "" | "." | "..")) {
            let parent = &parts[..parts.len() - 1];
            let Some(parent) = resolve(Path::new(&join_parts(parent, absolute))) else {
                return false;
            };
            return verdict(&parent.join(name));
        }
    }
    true
}

/// The deletion targets that fall outside the allowed roots, each as written,
/// or as its label when it cannot be resolved.
fn offending_targets(
    targets: Vec<Target>,
    home: &str,
    claude_dir: &Path,
    safe_roots: &[PathBuf],
    scratch_roots: &[PathBuf],
) -> Vec<String> {
    targets
        .into_iter()
        .filter_map(|target| match target {
            Target::Unresolvable(label) => Some(label),
            Target::Path { text, tilde } => {
                (!is_allowed(&text, tilde, home, claude_dir, safe_roots, scratch_roots))
                    .then_some(text)
            }
        })
        .collect()
}
