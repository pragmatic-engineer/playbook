// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The `git` calls that write a message: `commit`, `tag`, `merge` and
//! `commit-tree`. Reads the message from every form git accepts and from the
//! message a commit reuses, drops an `--author` or `--trailer` that names an
//! AI, and follows the script a `git rebase --exec` runs.

use super::engine::{git_output, nested_word, Call, Findings, Plan};
use super::sources::{
    apply, feed, from_path, from_stdin, from_word, joined, read_message, resolve, value_span,
    Named, Rule, Source, Target,
};
use crate::common::attribution::{has_sign_off, is_ai_identity, problems, sanitize, trailer_shape};
use crate::common::cli_opts::{has, named, Opt, Spec};
use crate::common::repo::repo_slug;
use crate::common::session::home_dir;
use crate::common::shell::{commands, is_assignment, program_name, quote};
use crate::config;
use std::ops::Range;
use std::path::{Path, PathBuf};

// The specs list the options whose value can be a message or hides one, and
// the flags that change where a message comes from. `other_long` holds the
// rest of the subcommand's long names, as `--git-completion-helper-all` of the
// installed git lists them, so an exact name or a prefix shared with one of
// them is not taken for an option here. A test checks it against real git.
const COMMIT: Spec = Spec {
    short_values: "mFCct",
    short_attached: "Su",
    long_values: &[
        "message",
        "file",
        "reuse-message",
        "reedit-message",
        "author",
        "date",
        "template",
        "cleanup",
        "fixup",
        "squash",
        "trailer",
    ],
    long_flags: &[
        "amend",
        "no-edit",
        "edit",
        "reset-author",
        "signoff",
        "no-signoff",
        "dry-run",
    ],
    other_long: &[
        "quiet",
        "verbose",
        "status",
        "gpg-sign",
        "all",
        "include",
        "interactive",
        "patch",
        "unified",
        "inter-hunk-context",
        "only",
        "no-verify",
        "short",
        "branch",
        "ahead-behind",
        "porcelain",
        "long",
        "null",
        "no-post-rewrite",
        "untracked-files",
        "pathspec-from-file",
        "pathspec-file-nul",
        "allow-empty",
        "allow-empty-message",
        "verify",
        "post-rewrite",
        "no-quiet",
        "no-verbose",
        "no-file",
        "no-author",
        "no-date",
        "no-message",
        "no-reedit-message",
        "no-reuse-message",
        "no-fixup",
        "no-squash",
        "no-reset-author",
        "no-trailer",
        "no-template",
        "no-cleanup",
        "no-status",
        "no-gpg-sign",
        "no-all",
        "no-include",
        "no-interactive",
        "no-patch",
        "no-only",
        "no-dry-run",
        "no-short",
        "no-branch",
        "no-ahead-behind",
        "no-porcelain",
        "no-long",
        "no-null",
        "no-amend",
        "no-untracked-files",
        "no-pathspec-from-file",
        "no-pathspec-file-nul",
        "no-allow-empty",
        "no-allow-empty-message",
    ],
    abbreviate: true,
};
const MERGE: Spec = Spec {
    short_values: "mFsX",
    short_attached: "S",
    long_values: &["message", "file", "strategy", "strategy-option", "cleanup"],
    long_flags: &["no-edit", "edit", "squash", "no-commit", "signoff"],
    other_long: &[
        "stat",
        "summary",
        "compact-summary",
        "log",
        "commit",
        "ff",
        "ff-only",
        "rerere-autoupdate",
        "verify-signatures",
        "into-name",
        "verbose",
        "quiet",
        "abort",
        "quit",
        "continue",
        "allow-unrelated-histories",
        "progress",
        "gpg-sign",
        "autostash",
        "overwrite-ignore",
        "no-verify",
        "verify",
        "no-stat",
        "no-summary",
        "no-compact-summary",
        "no-log",
        "no-squash",
        "no-cleanup",
        "no-ff",
        "no-rerere-autoupdate",
        "no-verify-signatures",
        "no-strategy",
        "no-strategy-option",
        "no-message",
        "no-into-name",
        "no-verbose",
        "no-quiet",
        "no-abort",
        "no-quit",
        "no-continue",
        "no-allow-unrelated-histories",
        "no-progress",
        "no-gpg-sign",
        "no-autostash",
        "no-overwrite-ignore",
        "no-signoff",
    ],
    abbreviate: true,
};
const TAG: Spec = Spec {
    short_values: "mFuf",
    short_attached: "",
    long_values: &["message", "file", "local-user", "cleanup", "trailer"],
    long_flags: &["annotate", "sign", "force", "edit", "no-edit"],
    other_long: &[
        "list",
        "delete",
        "verify",
        "create-reflog",
        "column",
        "contains",
        "no-contains",
        "with",
        "without",
        "merged",
        "no-merged",
        "omit-empty",
        "sort",
        "points-at",
        "format",
        "color",
        "ignore-case",
        "no-annotate",
        "no-file",
        "no-trailer",
        "no-sign",
        "no-cleanup",
        "no-local-user",
        "no-force",
        "no-create-reflog",
        "no-column",
        "no-omit-empty",
        "no-sort",
        "no-points-at",
        "no-format",
        "no-color",
        "no-ignore-case",
    ],
    abbreviate: true,
};
// `git commit-tree` has no `--message` or `--file`, only the short options.
const COMMIT_TREE: Spec = Spec {
    short_values: "mFp",
    short_attached: "S",
    long_values: &[],
    long_flags: &[],
    other_long: &["gpg-sign", "no-gpg-sign"],
    abbreviate: true,
};

/// Environment variables and `-c` keys that set the author or committer.
const IDENTITY_NAME_KEYS: [&str; 4] = [
    "GIT_AUTHOR_NAME",
    "GIT_COMMITTER_NAME",
    "user.name",
    "author.name",
];
const IDENTITY_EMAIL_KEYS: [&str; 4] = [
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_EMAIL",
    "user.email",
    "author.email",
];
const REBASE: Spec = Spec {
    short_values: "x",
    short_attached: "",
    long_values: &["exec"],
    long_flags: &[],
    other_long: &[],
    abbreviate: true,
};
/// Global options that take a value in the next word.
const GLOBAL_WITH_VALUE: [&str; 6] = [
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--attr-source",
    "--config-env",
];
const MESSAGE_FILES: [&str; 2] = ["MERGE_MSG", "SQUASH_MSG"];
/// Longest hook script read when looking for a sign-off.
const MAX_SCRIPT_BYTES: u64 = 1 << 20;

/// What every subcommand handler needs.
struct Run<'a> {
    call: Call<'a>,
    /// Index of the first word after the subcommand.
    rest: usize,
}

pub fn handle(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    drop_identity_env(call, plan, findings);
    let Some((sub_at, dir)) = skip_globals(call, plan, findings) else {
        return;
    };
    let call = Call { dir: &dir, ..*call };
    let run = Run {
        call,
        rest: sub_at + 1,
    };
    match call.cmd().words[sub_at].as_str() {
        "commit" => commit(&run, plan, findings),
        "merge" => message_command(&run, &MERGE, "the merge message", plan, findings),
        "tag" => tag(&run, plan, findings),
        "commit-tree" => commit_tree(&run, plan, findings),
        "rebase" => rebase(&run, plan, findings),
        _ => {}
    }
}

/// An assignment before the program that sets an AI as the author or
/// committer is removed, so git uses the configured identity.
fn drop_identity_env(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    let cmd = call.cmd();
    for (at, word) in cmd.words[..call.at].iter().enumerate() {
        let Some((name, value)) = word.split_once('=').filter(|_| is_assignment(word)) else {
            continue;
        };
        if is_ai_value(name, value) {
            let span = &cmd.spans[at];
            plan.edits.push((span.start..span.end, String::new()));
            findings
                .notes
                .push(format!("the {name} variable (an AI identity)"));
        }
    }
}

fn is_ai_value(key: &str, value: &str) -> bool {
    (IDENTITY_NAME_KEYS.contains(&key) && is_ai_identity(value, ""))
        || (IDENTITY_EMAIL_KEYS.contains(&key) && is_ai_identity("", value))
}

/// The index of the subcommand word and the directory git runs in, past the
/// global options. A `-c` that sets an AI identity is removed.
fn skip_globals(call: &Call, plan: &mut Plan, findings: &mut Findings) -> Option<(usize, PathBuf)> {
    let cmd = call.cmd();
    let words = &cmd.words;
    let mut dir = call.dir.to_path_buf();
    let mut at = call.at + 1;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "-C" => {
                if let Some(target) = words.get(at + 1) {
                    dir = resolve(target, &dir);
                }
                at += 2;
            }
            "-c" => {
                if let Some(setting) = words.get(at + 1) {
                    drop_identity_config(call, at..at + 2, setting, plan, findings);
                }
                at += 2;
            }
            w if w.starts_with("-c") && !w.starts_with("--") => {
                drop_identity_config(call, at..at + 1, &w[2..], plan, findings);
                at += 1;
            }
            w if GLOBAL_WITH_VALUE.contains(&w) => at += 2,
            w if w.starts_with('-') => at += 1,
            _ => return Some((at, dir)),
        }
    }
    None
}

fn drop_identity_config(
    call: &Call,
    words: Range<usize>,
    setting: &str,
    plan: &mut Plan,
    findings: &mut Findings,
) {
    let Some((key, value)) = setting.split_once('=') else {
        return;
    };
    let key = key.to_lowercase();
    let known = IDENTITY_NAME_KEYS
        .iter()
        .chain(&IDENTITY_EMAIL_KEYS)
        .find(|k| k.to_lowercase() == key);
    if known.is_some_and(|k| is_ai_value(k, value)) {
        let spans = &call.cmd().spans;
        plan.edits.push((
            spans[words.start].start..spans[words.end - 1].end,
            String::new(),
        ));
        findings
            .notes
            .push(format!("the {key} setting (an AI identity)"));
    }
}

fn commit(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = COMMIT.scan(&run.call.cmd().words[run.rest..]);
    let (sources, written) = message_sources(run, &opts, "the commit message", findings);
    let mut known = (!sources.is_empty()).then(|| joined(&sources));
    apply(sources, Rule::Commit, true, plan, findings);
    drop_ai_options(run, &opts, plan, findings);
    if !written {
        known = reused_message(run, &opts, plan, findings);
    }
    sign_off(run, &opts, known.as_deref(), plan, findings);
}

/// The message a commit takes from the repository instead of the command: the
/// one `-C` or `--amend --no-edit` reuses, or the one a merge or a squash left
/// behind. It is rewritten to come from a heredoc when it carries
/// attribution. It is left alone, and reported as unread, when an earlier git
/// command in the same call may change what the repository holds by the time
/// this one runs, when the commit sits in a nested script such as a rebase
/// `--exec`, which runs later against other commits, and when `--reset-author`
/// changes the author the rewrite would copy.
fn reused_message(
    run: &Run,
    opts: &[Opt],
    plan: &mut Plan,
    findings: &mut Findings,
) -> Option<String> {
    let reused = named(opts, &["C", "c", "reuse-message", "reedit-message"]);
    let amends = has(opts, &["amend"]);
    let source = match reused.first() {
        Some(opt) => Some((Some(*opt), opt.value.clone().unwrap_or_default())),
        None if amends && has(opts, &["no-edit"]) => Some((None, "HEAD".to_string())),
        None if !amends => None,
        None => return None,
    };
    if git_ran_earlier(&run.call) {
        findings
            .unread
            .push("the reused message (an earlier git command in this call may change it)".into());
        return None;
    }
    if source.is_some() && run.call.depth > 0 {
        findings.unread.push(NESTED_REUSE.into());
        return None;
    }
    if !reused.is_empty() && has(opts, &["reset-author"]) {
        findings.unread.push(
            "the message reused with --reset-author (the rewrite would copy the author it resets)"
                .into(),
        );
        return None;
    }
    match source {
        Some((opt, rev)) => reuse(run, opt, &rev, plan, findings),
        None => stored_message(run, plan, findings),
    }
}

const NESTED_REUSE: &str =
    "the reused message (it is in a nested script, which may run against other commits)";

/// Whether a command before this one in the script runs git, or a script that
/// mentions it.
fn git_ran_earlier(call: &Call) -> bool {
    call.cmds[..call.index].iter().any(|cmd| {
        cmd.words.iter().any(|word| {
            program_name(word) == "git"
                || (word.contains(char::is_whitespace) && word.to_lowercase().contains("git"))
        })
    })
}

/// Adds `-s` unless a sign-off is already settled: asked for or refused on the
/// command line, in the message, added by the repository's own hook, or turned
/// off by `commit.signOff`. The commit then carries the Signed-off-by line the
/// DCO check needs.
fn sign_off(
    run: &Run,
    opts: &[Opt],
    known: Option<&str>,
    plan: &mut Plan,
    findings: &mut Findings,
) {
    let call = &run.call;
    let settled = has(opts, &["s", "signoff", "no-signoff", "dry-run"])
        || known.is_some_and(|text| has_sign_off(&sanitize(text).text))
        || hook_signs_off(call.dir)
        || !sign_off_enabled();
    if !settled {
        let at = call.cmd().spans[run.rest - 1].end;
        plan.edits.push((at..at, " -s".to_string()));
        findings.signed_off = true;
    }
}

/// Whether `commit.signOff` allows adding a sign-off. A config that cannot be
/// read keeps the default, on, so the DCO check still passes.
pub fn sign_off_enabled() -> bool {
    let slug = repo_slug();
    let slug = (!slug.is_empty()).then_some(slug.as_str());
    config::resolve("commit.signOff", &home_dir(), slug)
        .ok()
        .and_then(|(value, _)| value.as_bool())
        .unwrap_or(true)
}

/// Whether the repository's `prepare-commit-msg` or `commit-msg` hook, in the
/// hooks path git uses, adds a Signed-off-by line itself. A hook that only
/// checks for the line, or adds another trailer, does not.
fn hook_signs_off(dir: &Path) -> bool {
    let Some(hooks) = git_output(dir, &["rev-parse", "--git-path", "hooks"]) else {
        return false;
    };
    let hooks = resolve(hooks.trim(), dir);
    ["prepare-commit-msg", "commit-msg"].iter().any(|name| {
        runnable_script(&hooks.join(name)).is_some_and(|script| script_adds_sign_off(&script))
    })
}

/// Whether a hook script runs a command that signs off: `--signoff`, or
/// `git interpret-trailers` with a `--trailer` for the sign-off.
fn script_adds_sign_off(script: &str) -> bool {
    commands(script).iter().any(|cmd| {
        let words = &cmd.words;
        let trailers = words.iter().any(|w| w == "interpret-trailers")
            && words.iter().enumerate().any(|(i, w)| {
                let value = match w.strip_prefix("--trailer=") {
                    Some(inline) => Some(inline),
                    None if w == "--trailer" => words.get(i + 1).map(String::as_str),
                    None => None,
                };
                value.is_some_and(is_sign_off_trailer)
            });
        words.iter().any(|w| w == "--signoff") || trailers
    })
}

/// A `--trailer` value whose token is `Signed-off-by` or `sign-off`, however
/// it is spelled.
fn is_sign_off_trailer(value: &str) -> bool {
    let token = value.split([':', '=']).next().unwrap_or_default();
    let token: String = token
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    token == "signedoffby" || token == "signoff"
}

/// The text of an executable file, which git runs as a hook.
fn runnable_script(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    if !meta.is_file() || meta.len() > MAX_SCRIPT_BYTES {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn tag(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = TAG.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, "the tag message", findings);
    apply(sources, Rule::Commit, true, plan, findings);
    drop_ai_options(run, &opts, plan, findings);
}

/// The commands `-x` and `--exec` run are scripts of their own.
fn rebase(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = REBASE.scan(&run.call.cmd().words[run.rest..]);
    for opt in named(&opts, &["x", "exec"]) {
        let at = opt.value_word + run.rest;
        if opt.value.is_some() {
            nested_word(&run.call, at, opt.inline_at, plan, findings);
        }
    }
}

fn commit_tree(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let label = "the commit message";
    let opts = COMMIT_TREE.scan(&run.call.cmd().words[run.rest..]);
    let (mut sources, written) = message_sources(run, &opts, label, findings);
    if !written {
        sources = from_stdin(&run.call, label, findings);
    }
    apply(sources, Rule::Commit, true, plan, findings);
}

fn message_command(run: &Run, spec: &Spec, label: &str, plan: &mut Plan, findings: &mut Findings) {
    let opts = spec.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, label, findings);
    apply(sources, Rule::Commit, true, plan, findings);
}

/// The text of every `-m` and `-F` option, and whether there was one.
fn message_sources(
    run: &Run,
    opts: &[Opt],
    label: &str,
    findings: &mut Findings,
) -> (Vec<Source>, bool) {
    let call = &run.call;
    let mut sources = Vec::new();
    let mut written = false;
    for opt in opts {
        let value_at = opt.value_word + run.rest;
        let Some(value) = opt
            .value
            .as_deref()
            .filter(|_| value_at < call.cmd().words.len())
        else {
            continue;
        };
        match opt.name.as_str() {
            "m" | "message" => {
                written = true;
                let spans = &call.cmd().spans;
                let whole = spans[opt.word + run.rest].start..spans[value_at].end;
                let kept = kept_letters(opt, &call.cmd().words[opt.word + run.rest]);
                let mut found = from_word(call, value_at, opt.inline_at, label, findings);
                for source in &mut found {
                    if let Target::Word { whole: slot, .. } = &mut source.target {
                        *slot = Some((whole.clone(), kept.clone()));
                    }
                }
                sources.extend(found);
            }
            "F" | "file" => {
                written = true;
                let named = Named {
                    path: value,
                    dynamic: call.cmd().spans[value_at].dynamic,
                    replace: value_span(call, value_at, opt.inline_at),
                    opener: "- ",
                };
                sources.extend(from_path(call, &named, label, findings));
            }
            _ => {}
        }
    }
    (sources, written)
}

/// Removes an `--author` that names an AI and a `--trailer` that credits one.
fn drop_ai_options(run: &Run, opts: &[Opt], plan: &mut Plan, findings: &mut Findings) {
    let cmd = run.call.cmd();
    for opt in opts {
        let Some(value) = opt.value.as_deref() else {
            continue;
        };
        let value_at = opt.value_word + run.rest;
        let ai = match opt.name.as_str() {
            "author" => author_is_ai(value),
            "trailer" => trailer_is_ai(value),
            _ => false,
        };
        if ai && !cmd.spans[value_at].dynamic {
            let range = cmd.spans[opt.word + run.rest].start..cmd.spans[value_at].end;
            plan.edits.push((range, String::new()));
            findings
                .notes
                .push(format!("the --{} option (an AI)", opt.name));
        }
    }
}

/// `Name <email>`, or a bare name or email.
fn author_is_ai(value: &str) -> bool {
    match value.split_once('<') {
        Some((name, email)) => is_ai_identity(name, email.trim_end_matches('>')),
        None => is_ai_identity(value, value),
    }
}

/// `token:value` or `token=value`, as `--trailer` reads it.
fn trailer_is_ai(value: &str) -> bool {
    let Some((token, rest)) = value.split_once([':', '=']) else {
        return false;
    };
    trailer_shape(&format!("{}: {}", token.trim(), rest.trim())).is_some()
}

/// Rewrites a commit that reuses a message, from `-C` or from `--amend
/// --no-edit`, to read the sanitised message from a heredoc instead. The
/// message and the author are read now, so the rewritten call does not depend
/// on what the repository looks like when it runs.
fn reuse(
    run: &Run,
    option: Option<&Opt>,
    rev: &str,
    plan: &mut Plan,
    findings: &mut Findings,
) -> Option<String> {
    let call = &run.call;
    let message = git_output(call.dir, &["log", "-1", "--format=%B", rev, "--"])?;
    if !problems(&message).is_empty() {
        let spans = &call.cmd().spans;
        let (replace, opener) = match option {
            Some(opt) => (
                spans[opt.word + run.rest].start..spans[opt.value_word + run.rest].end,
                format!("{}-F - ", author_flags(call.dir, rev)),
            ),
            None => {
                let at = spans[run.rest - 1].end;
                (at..at, " -F - ".to_string())
            }
        };
        let label = format!("the message reused from {rev}");
        let source = feed(call, label, message.clone(), replace, opener);
        apply(vec![source], Rule::Commit, false, plan, findings);
    }
    Some(message)
}

/// `--author` and `--date` copied from `rev`, unless its author is an AI.
fn author_flags(dir: &Path, rev: &str) -> String {
    let Some(out) = git_output(dir, &["log", "-1", "--format=%an%n%ae%n%aI", rev, "--"]) else {
        return String::new();
    };
    let mut lines = out.lines();
    let (Some(name), Some(email), Some(date)) = (lines.next(), lines.next(), lines.next()) else {
        return String::new();
    };
    if is_ai_identity(name, email) {
        return String::new();
    }
    format!(
        "--author={} --date={} ",
        quote(&format!("{name} <{email}>")),
        quote(date)
    )
}

/// A commit with no message option takes its message from the file a merge
/// or a squash left behind, `MERGE_MSG` before `SQUASH_MSG`. It is read from a
/// heredoc, and the file is left as it is.
fn stored_message(run: &Run, plan: &mut Plan, findings: &mut Findings) -> Option<String> {
    let call = &run.call;
    let git_dir = git_output(call.dir, &["rev-parse", "--git-dir"])?;
    let git_dir = resolve(git_dir.trim(), call.dir);
    let name = MESSAGE_FILES
        .iter()
        .find(|name| git_dir.join(name).exists())?;
    if call.depth > 0 {
        findings
            .unread
            .push(format!("{name} (in a nested script, which may run later)"));
        return None;
    }
    let text = match read_message(&git_dir.join(name)) {
        Ok(text) => text,
        Err(why) => {
            findings.unread.push(format!("{name} ({why})"));
            return None;
        }
    };
    let at = call.cmd().spans[run.rest - 1].end;
    let source = feed(
        call,
        name.to_string(),
        text.clone(),
        at..at,
        " -F - ".to_string(),
    );
    apply(vec![source], Rule::Commit, false, plan, findings);
    Some(text)
}

/// What stays of the word of a short `-m` when the option goes: the other
/// letters of a cluster such as `-nm`, or nothing.
fn kept_letters(opt: &Opt, word: &str) -> String {
    let others = match word.strip_prefix('-').filter(|w| !w.starts_with('-')) {
        Some(cluster) if opt.name == "m" => cluster.split('m').next().unwrap_or_default(),
        _ => "",
    };
    if others.is_empty() {
        String::new()
    } else {
        format!("-{others}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::process::Command;

    /// The long option names `git <sub>` lists, read from the installed git.
    fn git_long_options(sub: &str) -> Vec<String> {
        let repo = scratch_dir("git-options");
        std::fs::create_dir_all(&repo).expect("scratch dir");
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("git runs")
        };
        run(&["init", "-q"]);
        let out = run(&[sub, "--git-completion-helper-all"]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let _ = std::fs::remove_dir_all(&repo);
        text.split_whitespace()
            .filter(|w| w.starts_with("--") && *w != "--")
            .map(|w| w[2..].trim_end_matches('=').to_string())
            .collect()
    }

    /// `spec` without the long names the installed git lacks, which an older
    /// git never had. Each one left out is printed.
    fn known_to_git(sub: &str, spec: &Spec, names: &[String]) -> Spec {
        let keep = |list: &[&'static str]| -> &'static [&'static str] {
            let (known, missing): (Vec<&str>, Vec<&str>) = list
                .iter()
                .copied()
                .partition(|name| names.iter().any(|n| n == name));
            if !missing.is_empty() {
                eprintln!("note: the installed git {sub} has no {missing:?}, so they are skipped");
            }
            Box::leak(known.into_boxed_slice())
        };
        Spec {
            short_values: spec.short_values,
            short_attached: spec.short_attached,
            long_values: keep(spec.long_values),
            long_flags: keep(spec.long_flags),
            other_long: keep(spec.other_long),
            abbreviate: spec.abbreviate,
        }
    }

    /// The option git resolves `--prefix` to: an exact name, else the only
    /// name it starts. `None` when git refuses it as ambiguous.
    fn git_resolves(names: &[String], prefix: &str) -> Option<String> {
        if names.iter().any(|n| n == prefix) {
            return Some(prefix.to_string());
        }
        let mut starts = names.iter().filter(|n| n.starts_with(prefix));
        match (starts.next(), starts.next()) {
            (Some(only), None) => Some(only.clone()),
            _ => None,
        }
    }

    #[test]
    fn every_abbreviation_git_accepts_resolves_the_way_git_does() {
        for (sub, spec) in [
            ("commit", COMMIT),
            ("merge", MERGE),
            ("tag", TAG),
            ("commit-tree", COMMIT_TREE),
        ] {
            let names = git_long_options(sub);
            assert!(!names.is_empty(), "git {sub} lists its options");
            let spec = known_to_git(sub, &spec, &names);
            let read: Vec<&str> = spec
                .long_values
                .iter()
                .chain(spec.long_flags)
                .copied()
                .collect();
            for name in &names {
                for end in 1..=name.len() {
                    let prefix = &name[..end];
                    let Some(expected) = git_resolves(&names, prefix) else {
                        continue;
                    };
                    let got = spec.scan(&[format!("--{prefix}")]);

                    let ours = got[0].name.as_str();
                    if read.contains(&expected.as_str()) {
                        assert_eq!(ours, expected, "git {sub} --{prefix}");
                    } else {
                        assert!(
                            !read.contains(&ours),
                            "git {sub} --{prefix} is {expected}, not {ours}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_specs_only_name_options_git_has() {
        for (sub, spec) in [
            ("commit", COMMIT),
            ("merge", MERGE),
            ("tag", TAG),
            ("commit-tree", COMMIT_TREE),
        ] {
            let names = git_long_options(sub);
            assert!(!names.is_empty(), "git {sub} lists its options");
            let ours = spec
                .long_values
                .iter()
                .chain(spec.long_flags)
                .chain(spec.other_long);
            for name in ours {
                if !names.iter().any(|n| n == name) {
                    eprintln!("note: the installed git {sub} has no --{name}, so it is skipped");
                }
            }
        }
    }

    #[test]
    fn a_trailing_cluster_letter_is_kept_when_its_message_goes() {
        let message = Opt {
            name: "m".to_string(),
            value: None,
            word: 0,
            value_word: 1,
            inline_at: None,
        };

        assert_eq!(kept_letters(&message, "-m"), "");
        assert_eq!(kept_letters(&message, "-nm"), "-n");
        assert_eq!(kept_letters(&message, "-anm"), "-an");
        assert_eq!(kept_letters(&message, "--message"), "");
    }
}
