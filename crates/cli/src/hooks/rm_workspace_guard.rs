// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ports the retired shell original: denies an `rm` whose target sits outside
//! the safe roots, `~/.claude/**`, playbook's own folders (`roots::footprint`,
//! whole) and install files, and the scratch trees (contents only, never the
//! tree root itself or a parent of it): the OS temp and cache folders listed
//! in `roots`, the shared `.worktrees` folder next to the repo (or next to the
//! main checkout when run inside a linked worktree), `WORKTREE_BASE_DIR` when
//! set, and the `hooks.rmGuard.allowedRoots` config list. Playbook data that
//! cannot be made again (`roots::protected`) stays blocked whatever root
//! covers it, and so do the built-in deny set in
//! `roots::DENY` (system folders, credentials, personal folders, shell
//! config, Claude Code's settings and transcripts) and, outside the OS temp
//! folders and the folders playbook makes worktrees in, a repo or worktree
//! root and its `.git`. A denial names the rule that matched
//! and, for a plain folder in home, the exact config command that allows it.
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
//! repo's; a script file run by an interpreter, and a `>|` redirect, are not
//! read.
//!
//! Best-effort protection against an accidental `rm`, NOT a security boundary.
//! Anything it cannot resolve is blocked rather than evaluated.
//!
//! The command is read into real command words by `parse`, on the shared shell
//! lexer, so only a word in command position is a deletion. Quoted text, a
//! grep pattern, a commit message, an `echo` and a heredoc read by a program
//! other than a shell are data. `find -delete`, `find -exec rm` and
//! `xargs rm` are judged by `find`'s start paths. The hook runs for every
//! Bash command, since a deletion need not hold an `rm` word, so a command
//! that deletes nothing returns before any git or config read, and one that
//! only overwrites reads neither.
//!
//! Relative targets are judged from the `cwd` in the payload, and after a
//! `cd` from the folder the shell changed into (see `parse`). The repo of
//! each folder the command changes into is a safe root too.
//!
//! Deliberate conservative blocks: a relative target in a folder only the
//! shell knows is unresolvable; a deletion inside, next to or fed by a command
//! substitution is unresolvable; a target holding a variable is unresolvable
//! unless `parse` can follow it (a value the command sets, `$HOME`, `$PWD`,
//! `$TMPDIR` and a few more). The variable rule fixed a FAIL-OPEN
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

mod expand;
mod interp;
mod message;
mod parse;
mod roots;

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use parse::Target;
use roots::{InstallFile, Kept, RootEnv, Scope};
use std::path::{Path, PathBuf};

pub fn run(payload: &Payload) {
    let cmd = payload.field(".tool_input.command");
    if cmd.is_empty() {
        return;
    }
    let cwd = start_dir(&payload.field(".cwd"));
    let Some((judged, roots)) = judge_command(&cmd, &cwd) else {
        return;
    };
    let denied: Vec<(String, Rule)> = judged
        .into_iter()
        .filter_map(|j| j.rule.map(|rule| (j.target, rule)))
        .collect();
    if denied.is_empty() {
        return;
    }
    emit_pre_deny(&message::denial(&denied, &roots));
}

/// What the guard does with one path a command names.
pub struct Explained {
    /// The path as written, or the label of what cannot be resolved.
    pub target: String,
    /// `delete`, `below` (a deletion that stays under the path), `overwrite`,
    /// `unresolvable` or `option`.
    pub kind: &'static str,
    /// The rule that denies it, or `None` when it is allowed.
    pub rule: Option<&'static str>,
    /// Where it resolved to, when that is outside every root.
    pub resolved: Option<PathBuf>,
}

/// The guard's verdict on `command` run in `cwd`, for `playbook hooks
/// explain-rm`: every path it names, and the denial text Claude would see.
pub fn explain(command: &str, cwd: &Path) -> (Vec<Explained>, Option<String>) {
    let Some((judged, roots)) = judge_command(command, cwd) else {
        return (Vec::new(), None);
    };
    let denied: Vec<(String, Rule)> = judged
        .iter()
        .filter_map(|j| j.rule.clone().map(|rule| (j.target.clone(), rule)))
        .collect();
    let text = (!denied.is_empty()).then(|| message::denial(&denied, &roots));
    let explained = judged
        .into_iter()
        .map(|j| Explained {
            // As in the denial: only when it differs from the path as written.
            resolved: match &j.rule {
                Some(Rule::Outside(dir)) if dir.as_os_str() != j.target.as_str() => {
                    Some(dir.clone())
                }
                _ => None,
            },
            rule: j.rule.as_ref().map(Rule::name),
            target: j.target,
            kind: j.kind,
        })
        .collect();
    (explained, text)
}

/// One judged target.
struct Judged {
    target: String,
    kind: &'static str,
    rule: Option<Rule>,
}

/// Every target `command` names, judged, with the roots used, or `None`
/// when it deletes and overwrites nothing.
fn judge_command(cmd: &str, cwd: &Path) -> Option<(Vec<Judged>, Roots)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let tmpdir = absolute("TMPDIR");
    let cargo_target_dir = absolute("CARGO_TARGET_DIR");
    // Most commands that reach the hook delete nothing, so the roots, which
    // cost git and config reads, are only worked out when something does.
    let found = parse::scan(
        cmd,
        &parse::Env {
            home: &home,
            cwd,
            cdpath: std::env::var_os("CDPATH").is_some_and(|v| !v.is_empty()),
            tmpdir: tmpdir.as_deref(),
            cargo_target_dir: cargo_target_dir.as_deref(),
        },
    );
    if found.targets.is_empty() {
        return None;
    }
    let env = RootEnv::from_process(&home);
    // An overwrite is judged by the deny set and playbook's data alone, so a
    // command that only redirects output skips the git and config reads.
    let overwrites_only = found
        .targets
        .iter()
        .all(|t| matches!(t, Target::Overwrite { .. }));
    let roots = if overwrites_only {
        Roots::for_overwrites(&home, cwd, &env)
    } else {
        Roots::new(&home, cwd, &found.dirs, &env)
    };
    let judged = judge_targets(found.targets, &roots);
    Some((judged, roots))
}

/// The folder the command runs in: the `cwd` Claude Code sends, which is
/// where its shell is, or this process's folder when that is missing or gone.
fn start_dir(sent: &str) -> PathBuf {
    let sent = Path::new(sent);
    if sent.is_absolute() && sent.is_dir() {
        return sent.to_path_buf();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Why a target is denied. Each names the rule in the denial message.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    /// A command substitution, a variable, xargs input or a relative path
    /// after `cd`: the guard cannot tell what it names.
    Unresolvable,
    /// A quote, backslash or shell syntax left inside the word.
    StrayCharacter,
    /// `~user`, or a `~` inside quotes, which the shell leaves alone.
    Tilde,
    /// A symlink chain too deep to follow, a link that cannot be read, or
    /// a working folder that is gone.
    LinkLoop,
    /// Playbook data that cannot be made again, or a folder that holds it.
    /// Every protected item the target is or holds.
    Protected(Vec<Kept>),
    /// A scratch root itself: only its contents may go.
    RootItself,
    /// A glob the guard cannot judge, such as `/tmp/*`.
    Glob,
    /// Outside every allowed root. Holds the resolved path.
    Outside(PathBuf),
    /// In the built-in deny set. Holds the row's group.
    Denied(&'static str),
    /// A repo or worktree root, `.git`, or a glob at a root that takes
    /// either, outside the temp folders.
    RepoRoot,
    /// An option that is always blocked, such as `--no-preserve-root`.
    Forbidden,
}

impl Rule {
    fn name(&self) -> &'static str {
        match self {
            Rule::Unresolvable => "unresolvable target",
            Rule::StrayCharacter => "stray quote or shell character",
            Rule::Tilde => "another user's home or a quoted ~",
            Rule::LinkLoop => "path that cannot be resolved",
            Rule::Protected(_) => "protected playbook data",
            Rule::RootItself => "scratch root itself (contents only)",
            Rule::Glob => "bare glob",
            Rule::Outside(_) => "outside the allowed roots",
            Rule::Denied(_) => "built-in deny list",
            Rule::RepoRoot => "repo root or .git",
            Rule::Forbidden => "option that is always blocked",
        }
    }
}

/// Every root the guard judges against, resolved through symlinks.
struct Roots {
    home: String,
    /// The folder the command starts in, for relative targets.
    cwd: PathBuf,
    claude_dir: PathBuf,
    /// The repo, or `PLAYBOOK_SAFE_ROOTS`: deleted whole if asked.
    safe: Vec<PathBuf>,
    /// Contents only.
    scratch: Vec<PathBuf>,
    protected: Vec<(PathBuf, Kept)>,
    /// Playbook's own folders, each deletable whole.
    footprint: Vec<PathBuf>,
    /// Where playbook makes worktrees: a linked worktree root there may go.
    worktrees: Vec<PathBuf>,
    /// The built-in deny set, resolved, with each row's scope and group.
    deny: Vec<(PathBuf, Scope, &'static str)>,
    /// The OS temp and cache folders, where a scratch clone's root and
    /// `.git` may go.
    temp: Vec<PathBuf>,
    install: Vec<InstallFile>,
    /// `hooks.rmGuard.allowedRoots` as stored, for the fix command.
    configured: Vec<String>,
}

impl Roots {
    /// `dirs` are the folders the command changes into: the repo each one
    /// sits in is a safe root too.
    /// Only what an overwrite is judged by: the deny set and playbook's data.
    /// Every allowance is empty, which an overwrite never reads.
    fn for_overwrites(home: &str, cwd: &Path, env: &RootEnv) -> Roots {
        Roots {
            claude_dir: PathBuf::new(),
            safe: Vec::new(),
            scratch: Vec::new(),
            protected: roots::protected(env)
                .into_iter()
                .map(|(p, kept)| (resolve_existing(&p), kept))
                .collect(),
            deny: roots::deny_set(env)
                .into_iter()
                .map(|(path, scope, group)| (resolve_existing(&path), scope, group))
                .collect(),
            temp: Vec::new(),
            footprint: Vec::new(),
            worktrees: Vec::new(),
            install: Vec::new(),
            configured: Vec::new(),
            home: home.to_string(),
            cwd: cwd.to_path_buf(),
        }
    }

    fn new(home: &str, cwd: &Path, dirs: &[PathBuf], env: &RootEnv) -> Roots {
        let configured = configured_values(home);
        let safe = safe_roots(home, cwd, dirs);
        let install = roots::install_files(env)
            .into_iter()
            .map(|file| match file {
                InstallFile::Exact(path) => InstallFile::Exact(resolve_parent(&path)),
                InstallFile::Prefix {
                    dir,
                    prefix,
                    suffix,
                } => InstallFile::Prefix {
                    dir: resolve_existing(&dir),
                    prefix,
                    suffix,
                },
            })
            .collect();
        Roots {
            claude_dir: resolve_existing(&PathBuf::from(home).join(".claude")),
            safe: safe.clone(),
            scratch: scratch_roots(home, cwd, env, &configured),
            protected: roots::protected(env)
                .into_iter()
                .map(|(p, kept)| (resolve_existing(&p), kept))
                .collect(),
            deny: roots::deny_set(env)
                .into_iter()
                .map(|(path, scope, group)| (resolve_existing(&path), scope, group))
                .collect(),
            temp: roots::temp_roots(env)
                .iter()
                .map(|p| resolve_existing(p))
                .collect(),
            // A footprint folder that links elsewhere would widen the
            // allowance to wherever it points, so only a target that is still
            // named for playbook is kept.
            footprint: roots::footprint(env)
                .iter()
                .map(|p| resolve_existing(p))
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n == "playbook" || n == "pragmatic-engineer")
                })
                .collect(),
            worktrees: worktree_roots(home, cwd, &safe),
            install,
            configured,
            home: home.to_string(),
            cwd: cwd.to_path_buf(),
        }
    }
}

/// `PLAYBOOK_SAFE_ROOTS` is colon-separated like `$PATH`. Unset or empty falls
/// back to the git repo root, then the cwd, plus the repo of each folder the
/// command changes into. `$HOME` is deliberately not the default, since that
/// would unblock `~/.ssh` and `~/.aws`, so a repo that is home or holds it
/// is left out.
fn safe_roots(home: &str, cwd: &Path, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let configured = std::env::var("PLAYBOOK_SAFE_ROOTS").unwrap_or_default();
    if configured.is_empty() {
        let home = resolve_existing(Path::new(home));
        // `/`, home or a parent of home would turn the allowance into
        // nearly everything, so the fallback skips them too.
        let narrow = |root: &Path| root.components().count() >= 2 && !home.starts_with(root);
        let root = resolve_existing(&git_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf()));
        let mut roots: Vec<PathBuf> = narrow(&root).then_some(root).into_iter().collect();
        for dir in dirs.iter().filter(|d| d.is_dir()) {
            let Some(repo) = crate::common::gitfacts::toplevel(dir) else {
                continue;
            };
            let repo = resolve_existing(&repo);
            if narrow(&repo) && !roots.contains(&repo) {
                roots.push(repo);
            }
        }
        return roots;
    }
    configured
        .split(':')
        .filter(|r| !r.is_empty())
        .map(|r| resolve_existing(Path::new(r)))
        .collect()
}

/// The directories whose CONTENTS may be deleted, resolved through symlinks:
/// the OS temp and cache folders, the shared `.worktrees` folder,
/// `WORKTREE_BASE_DIR` and the configured roots. The macOS per-user temp
/// folder is matched by shape in `strict_base`.
fn scratch_roots(home: &str, cwd: &Path, env: &RootEnv, configured: &[String]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = roots::temp_roots(env)
        .into_iter()
        .map(|r| resolve_existing(&r))
        .collect();
    let repo = git_repo_root(cwd);
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
    roots.extend(configured_roots(home, configured));
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

/// The folders playbook makes worktrees in: `.worktrees` next to the main
/// checkout, `WORKTREE_BASE_DIR`, and `.claude/worktrees` in each safe root.
/// One that is `/`, home or a folder above home is dropped.
fn worktree_roots(home: &str, cwd: &Path, safe: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = safe.iter().map(|r| r.join(".claude/worktrees")).collect();
    if let Some(parent) =
        main_checkout(git_repo_root(cwd).as_deref()).and_then(|m| m.parent().map(Path::to_path_buf))
    {
        roots.push(resolve_existing(&parent.join(".worktrees")));
    }
    if let Ok(base) = std::env::var("WORKTREE_BASE_DIR") {
        if base.starts_with('/') {
            roots.push(resolve_existing(Path::new(&base)));
        }
    }
    let home = resolve_existing(Path::new(home));
    roots.retain(|r| r.components().count() >= 2 && !home.starts_with(r));
    roots
}

/// Whether `dir` is the root of a linked worktree: its `.git` is a file that
/// points into a `worktrees` folder of the main repo. A main checkout and a
/// submodule are not.
fn is_linked_worktree(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(".git")).is_ok_and(|text| {
        text.strip_prefix("gitdir:")
            .is_some_and(|to| to.contains("/worktrees/") || to.contains("\\worktrees\\"))
    })
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

/// `hooks.rmGuard.allowedRoots` as stored, each without a trailing `/**`. An
/// unreadable or malformed value yields nothing, which only narrows the
/// allowance.
fn configured_values(home: &str) -> Vec<String> {
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
        .map(|t| crate::config::write::strip_root_glob(t).to_string())
        .collect()
}

/// The configured roots, `~/` expanded and resolved.
fn configured_roots(home: &str, configured: &[String]) -> Vec<PathBuf> {
    let home_path = Path::new(home);
    configured
        .iter()
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

fn git_repo_root(cwd: &Path) -> Option<PathBuf> {
    if let Some(top) = crate::common::gitfacts::toplevel(cwd) {
        return Some(top);
    }
    let args = ["rev-parse", "--show-toplevel"];
    let path = crate::common::git::trimmed(crate::common::git::run(
        Some(cwd),
        &args,
        crate::common::git::TIMEOUT,
    ))?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Resolves `.`, `..` and every symlink along the path, the way the kernel
/// would, but tolerating missing components: the target of an `rm` may not exist,
/// and a missing component can hold no link. `..` is applied AFTER a link is
/// followed, so `/tmp/link/../x` is judged where it really points. A relative
/// `path` is taken from `cwd`. `None` means the link chain is too deep to
/// trust.
fn resolve(path: &Path, cwd: &Path) -> Option<PathBuf> {
    let absolute = cwd.join(path);
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
        .or_else(|| resolve(path, Path::new("/nonexistent-relative-root")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent-unresolvable-root"))
}

/// `path` with its longest existing part spelled as the disk stores it, which
/// fixes the letter case on a case-insensitive disk. A last component that is
/// a symlink is kept as written, so the link is judged where it sits.
fn stored_case(path: &Path) -> PathBuf {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let mut rest = Vec::new();
    let mut current = path;
    if is_link {
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
    loop {
        if let Ok(found) = std::fs::canonicalize(current) {
            return rest.iter().rev().fold(found, |acc, part| acc.join(part));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `path` with its folder resolved and its own name kept, so a file that is
/// a link is matched where it sits.
fn resolve_parent(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve_existing(parent).join(name),
        _ => resolve_existing(path),
    }
}

/// The scratch root `path` is at or below, or `None`: a listed root or the
/// macOS per-user temp folder.
fn strict_base(path: &Path, roots: &Roots) -> Option<PathBuf> {
    if let Some(root) = roots.scratch.iter().find(|r| path.starts_with(r)) {
        return Some(root.clone());
    }
    roots::macos_temp_base(path)
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

/// Whether deleting `target`, a word as the shell passes it, is allowed, and
/// the rule that denies it when it is not. `tilde` says a leading `~` was
/// unquoted, so the shell expands it.
fn judge(target: &str, tilde: bool, below: bool, roots: &Roots) -> Result<(), Rule> {
    // A quote or backslash left in the word is part of the file name, which is
    // more likely a mistake than a real file. The rest are shell syntax (zsh
    // glob qualifiers, redirects) that this guard does not model.
    if target.contains(['"', '\'', '\\', '(', ')', '<', '>', '^']) {
        return Err(Rule::StrayCharacter);
    }
    // Only `~` and `~/...` are the caller's home, and never inside quotes, where
    // the shell leaves `~` alone. `~other` is somebody else's.
    let home = &roots.home;
    let expanded = if target == "~" || target.starts_with("~/") {
        if home.is_empty() || !tilde {
            return Err(Rule::Tilde);
        }
        format!("{home}{}", &target[1..])
    } else if target.starts_with('~') {
        return Err(Rule::Tilde);
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
        return Err(Rule::Glob);
    }
    let glob_at = parts.iter().position(|p| p.chars().any(is_glob_char));
    let literal_len = glob_at.unwrap_or(parts.len());
    // A `..` after a glob cannot be resolved, because the expansion may be a link.
    if glob_at.is_some_and(|g| parts[g..].contains(&"..")) {
        return Err(Rule::Glob);
    }
    let dir = resolve(
        Path::new(&join_parts(&parts[..literal_len], absolute)),
        &roots.cwd,
    )
    .ok_or(Rule::LinkLoop)?;
    // The glob, when it is the last component and has no trailing slash.
    let last_glob = glob_at
        .filter(|g| *g == parts.len() - 1 && !trailing_slash)
        .map(|g| parts[g]);
    let first_glob = glob_at.map(|g| parts[g]);

    let verdict = |dir: &Path| -> Result<(), Rule> {
        // The deny rules compare the path as the disk stores it, so a
        // different letter case on a case-insensitive disk cannot slip past.
        let stored = stored_case(dir);
        if let Some(group) = denied_by(&stored, glob_at.is_some(), last_glob, roots) {
            return Err(Rule::Denied(group));
        }
        let holds_protected = |(p, _): &&(PathBuf, Kept)| {
            if stored.starts_with(p) {
                return true;
            }
            if !p.starts_with(&stored) {
                return false;
            }
            // A glob with a named prefix next to a protected folder cannot
            // match it, so `runtime/sess-*` stays allowed.
            match last_glob {
                Some(glob) if p.parent() == Some(stored.as_path()) => {
                    let named = glob[..literal_prefix_len(glob)].to_lowercase();
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&named))
                }
                _ => true,
            }
        };
        let held: Vec<&(PathBuf, Kept)> = roots.protected.iter().filter(holds_protected).collect();
        // The denial names what is really there, or every item when none is.
        let present: Vec<&&(PathBuf, Kept)> = held
            .iter()
            .filter(|(p, _)| stored.starts_with(p) || p.symlink_metadata().is_ok())
            .collect();
        let mut kept: Vec<Kept> = if present.is_empty() {
            held.iter().map(|(_, k)| *k).collect()
        } else {
            present.iter().map(|(_, k)| *k).collect()
        };
        kept.dedup();
        if !kept.is_empty() {
            return Err(Rule::Protected(kept));
        }
        if repo_data(&stored, first_glob, last_glob, below, roots) {
            return Err(Rule::RepoRoot);
        }
        // Playbook's own folders go whole, the folder itself included.
        if roots.footprint.iter().any(|f| dir.starts_with(f)) {
            return Ok(());
        }
        if dir.starts_with(&roots.claude_dir) || roots.safe.iter().any(|r| dir.starts_with(r)) {
            return Ok(());
        }
        if (glob_at.is_none() || last_glob.is_some())
            && roots.install.iter().any(|f| f.covers(dir, last_glob))
        {
            return Ok(());
        }
        let Some(base) = strict_base(dir, roots) else {
            return Err(Rule::Outside(dir.to_path_buf()));
        };
        let Some(_) = glob_at else {
            // Contents only: the root itself and its parents stay blocked.
            return if dir == base {
                Err(Rule::RootItself)
            } else {
                Ok(())
            };
        };
        // A glob under a scratch root: the last component only, no trailing
        // slash, and directly under the root it needs a named prefix.
        match last_glob {
            Some(glob) if dir != base || literal_prefix_len(glob) >= 3 => Ok(()),
            _ => Err(Rule::Glob),
        }
    };
    verdict(&dir)?;
    // `rm link` removes the link itself, wherever it points, so the link's own
    // location has to be allowed as well as its destination. This is what stops
    // a link that sits outside every root, but points into one, from being
    // deleted by name.
    if glob_at.is_none() && !trailing_slash {
        if let Some(name) = parts.last().filter(|n| !matches!(**n, "" | "." | "..")) {
            let parent = &parts[..parts.len() - 1];
            let parent = resolve(Path::new(&join_parts(parent, absolute)), &roots.cwd)
                .ok_or(Rule::LinkLoop)?;
            return verdict(&parent.join(name));
        }
    }
    Ok(())
}

/// The deny-set group that blocks deleting `stored`, the target as the disk
/// stores it, or a folder that holds a deny-set path. With a glob, `stored`
/// is the folder the glob reads, so a row that blocks the folder itself
/// only is matched by what the glob can take.
fn denied_by(
    stored: &Path,
    glob: bool,
    last_glob: Option<&str>,
    roots: &Roots,
) -> Option<&'static str> {
    roots.deny.iter().find_map(|(path, scope, group)| {
        let direct = match scope {
            Scope::Subtree => stored.starts_with(path),
            Scope::Itself => !glob && stored == path,
            Scope::Children => stored == path || stored.parent() == Some(path.as_path()),
        };
        (direct || holds(stored, path, last_glob)).then_some(*group)
    })
}

/// Whether deleting `stored`, or what `last_glob` matches in it, takes
/// `inner` with it.
fn holds(stored: &Path, inner: &Path, last_glob: Option<&str>) -> bool {
    if inner == stored || !inner.starts_with(stored) {
        return false;
    }
    match last_glob {
        Some(glob) if inner.parent() == Some(stored) => inner
            .file_name()
            .is_some_and(|n| glob_matches(glob, &n.to_string_lossy())),
        _ => true,
    }
}

/// Whether the shell glob `pattern` (`*`, `?`, `[...]`, `{a,b}`) matches
/// `name`. A leading dot is matched only by a leading dot, as the shell does.
/// A brace group the guard cannot split is taken to match everything.
fn glob_matches(pattern: &str, name: &str) -> bool {
    if let Some(open) = pattern.find('{') {
        let Some(close) = pattern[open..].find('}').map(|c| open + c) else {
            return true;
        };
        let inner = &pattern[open + 1..close];
        if inner.contains('{') {
            return true;
        }
        return inner.split(',').any(|alt| {
            let expanded = format!("{}{alt}{}", &pattern[..open], &pattern[close + 1..]);
            glob_matches(&expanded, name)
        });
    }
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn walk(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|k| walk(&p[1..], &n[k..])),
            Some('?') => !n.is_empty() && walk(&p[1..], &n[1..]),
            Some('[') => {
                let Some(close) = p.iter().skip(1).position(|c| *c == ']').map(|i| i + 1) else {
                    return n.first() == Some(&'[') && walk(&p[1..], &n[1..]);
                };
                let Some(c) = n.first() else {
                    return false;
                };
                let set = &p[1..close];
                let (negate, set) = match set.first() {
                    Some('!' | '^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= set[i] <= *c && *c <= set[i + 2];
                        i += 3;
                    } else {
                        hit |= set[i] == *c;
                        i += 1;
                    }
                }
                hit != negate && walk(&p[close + 1..], &n[1..])
            }
            Some(c) => n.first() == Some(c) && walk(&p[1..], &n[1..]),
        }
    }
    walk(&p, &n)
}

/// Whether deleting `stored` would take a repo or worktree root or a `.git`
/// with it, outside the OS temp folders, where a scratch clone may go whole.
/// A linked worktree root in a folder playbook makes worktrees in may go.
/// A glob at a root counts when it is bare (`*`, `.*`) or can match `.git`.
/// `below` says the deletion stays under `stored`, as a `find` with a name
/// test does, so a root itself is not taken.
fn repo_data(
    stored: &Path,
    first_glob: Option<&str>,
    last_glob: Option<&str>,
    below: bool,
    roots: &Roots,
) -> bool {
    let in_temp = roots.temp.iter().any(|t| stored.starts_with(t))
        || roots::macos_temp_base(stored).is_some();
    // A linked worktree root playbook made may go; its `.git` file and a main
    // checkout in the same folder may not.
    let made_worktree = first_glob.is_none()
        && roots
            .worktrees
            .iter()
            .any(|w| stored.starts_with(w) && stored != w)
        && is_linked_worktree(stored);
    if in_temp || made_worktree {
        return false;
    }
    if stored.components().any(|c| c.as_os_str() == ".git") {
        return true;
    }
    if std::fs::symlink_metadata(stored.join(".git")).is_err() {
        return false;
    }
    match first_glob {
        None => !below,
        Some(glob) => {
            let bare = glob.chars().all(|c| matches!(c, '*' | '?' | '.'));
            (bare && last_glob.is_some()) || glob_matches(glob, ".git")
        }
    }
}

/// Every deletion and overwrite target, each as written, or as its label
/// when it cannot be resolved, with the rule that denies it, if any.
fn judge_targets(targets: Vec<Target>, roots: &Roots) -> Vec<Judged> {
    targets
        .into_iter()
        .map(|target| {
            let (target, kind, rule) = match target {
                Target::Unresolvable(label) => (label, "unresolvable", Some(Rule::Unresolvable)),
                Target::Forbidden(label) => (label, "option", Some(Rule::Forbidden)),
                Target::Path { text, tilde } => {
                    let rule = judge(&text, tilde, false, roots).err();
                    (text, "delete", rule)
                }
                Target::Below { text, tilde } => {
                    let rule = judge(&text, tilde, true, roots).err();
                    (text, "below", rule)
                }
                // An overwrite keeps the file, so only the deny set and
                // playbook's data block it.
                Target::Overwrite { text, tilde } => {
                    let rule = judge(&text, tilde, false, roots)
                        .err()
                        .filter(|r| matches!(r, Rule::Denied(_) | Rule::Protected(_)));
                    (text, "overwrite", rule)
                }
            };
            Judged { target, kind, rule }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    #[test]
    fn globs_match_the_way_the_shell_matches_names() {
        let cases = [
            ("*", "src", true),
            ("*", ".git", false),
            (".*", ".git", true),
            (".g*", ".git", true),
            ("?git", ".git", false),
            (".[a-z]*", ".git", true),
            (".[!g]*", ".git", false),
            ("*.log", "a.log", true),
            ("*.log", "a.txt", false),
            ("{a,.git}", ".git", true),
            ("{a,b}", ".git", false),
            ("x{a", ".git", true),
        ];
        for (pattern, name, want) in cases {
            assert_eq!(glob_matches(pattern, name), want, "{pattern} vs {name}");
        }
    }
}
