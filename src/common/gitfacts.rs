// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Git facts read straight from `.git`, with no `git` child process.
//!
//! A spawn costs 6 to 8 ms on a 2.7 ms floor, and hooks run on every tool
//! call. Every function here returns `Some` only when the answer is certain
//! and the same one `git` would print. `None` means "ask git": an unusual
//! layout, an unreadable file, or anything this module does not model. A
//! caller falls back to spawning `git`, so behaviour never regresses.
//!
//! Not modelled, so they fall back: bare repos, `GIT_DIR` and the other
//! `GIT_*` location variables, `core.worktree`, `include` and `includeIf`,
//! `url.*.insteadOf`, the reftable ref format, `extensions.worktreeConfig`,
//! legacy `remotes/` files, a repo on another mount than the start directory,
//! a repo owned by someone else (git's `safe.directory` check), and a start
//! directory inside a `.git` folder.

use std::fs;
use std::path::{Path, PathBuf};

/// Env vars that move or rewrite what git sees. Any one set means ask git.
const UNSUPPORTED_ENV: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_NAMESPACE",
];

/// A repository found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// The directory holding `HEAD` (`.git`, or `.git/worktrees/<name>`).
    pub git_dir: PathBuf,
    /// The directory holding `config`, `refs` and `packed-refs`.
    pub common_dir: PathBuf,
    /// The work tree root, as `git rev-parse --show-toplevel` prints it.
    pub toplevel: PathBuf,
}

/// Where HEAD points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    Detached(String),
}

/// Find the repo that contains `dir`. `None` when it is not certain.
fn discover(dir: &Path) -> Option<Repo> {
    if UNSUPPORTED_ENV
        .iter()
        .any(|v| std::env::var_os(v).is_some())
    {
        return None;
    }
    let start = fs::canonicalize(dir).ok()?;
    if start.components().any(|c| c.as_os_str() == ".git") {
        return None;
    }
    let start_dev = device(&start)?;
    for candidate in start.ancestors() {
        if device(candidate)? != start_dev {
            return None;
        }
        let dot_git = candidate.join(".git");
        let Ok(meta) = fs::symlink_metadata(&dot_git) else {
            // A bare repo or an odd layout would be found by git here; only
            // go on when the folder has no repo markers of its own.
            if candidate.join("HEAD").exists() && candidate.join("objects").exists() {
                return None;
            }
            continue;
        };
        if !owned_by_me(&dot_git) {
            return None;
        }
        let git_dir = if meta.is_dir() {
            dot_git
        } else if meta.is_file() {
            let text = fs::read_to_string(&dot_git).ok()?;
            let target = text.strip_prefix("gitdir:")?.trim();
            if target.is_empty() || text.lines().count() != 1 {
                return None;
            }
            let path = Path::new(target);
            let joined = if path.is_absolute() {
                path.to_path_buf()
            } else {
                candidate.join(path)
            };
            fs::canonicalize(joined).ok()?
        } else {
            return None;
        };
        if !git_dir.join("HEAD").is_file() {
            return None;
        }
        let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
            Ok(text) => {
                let rel = text.trim();
                if rel.is_empty() {
                    return None;
                }
                let path = Path::new(rel);
                let joined = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    git_dir.join(path)
                };
                fs::canonicalize(joined).ok()?
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
            Err(_) => return None,
        };
        if !common_dir.join("config").is_file() {
            return None;
        }
        let repo = Repo {
            git_dir,
            common_dir,
            toplevel: candidate.to_path_buf(),
        };
        return config_is_plain(&repo).then_some(repo);
    }
    None
}

/// The work tree root, as `git rev-parse --show-toplevel` prints it.
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    discover(dir).map(|r| r.toplevel)
}

/// HEAD of the repo containing `dir`.
pub fn head(dir: &Path) -> Option<Head> {
    head_of(&discover(dir)?)
}

/// What `git branch --show-current` prints: the branch name, `None` when
/// HEAD is detached or the answer is not certain.
pub fn current_branch(dir: &Path) -> Option<Option<String>> {
    match head(dir)? {
        Head::Branch(name) => Some(Some(name)),
        Head::Detached(_) => Some(None),
    }
}

/// What `git rev-parse HEAD` prints. `None` for an unborn branch and for
/// anything not certain.
pub fn head_sha(dir: &Path) -> Option<String> {
    let repo = discover(dir)?;
    match head_of(&repo)? {
        Head::Detached(sha) => Some(sha),
        Head::Branch(name) => branch_sha(&repo, &name),
    }
}

/// The single `remote.origin.url` as written in config, or `None` when there
/// is no origin, more than one URL, or any rewrite rule could apply.
pub fn origin_url(dir: &Path) -> Option<String> {
    let repo = discover(dir)?;
    if repo.common_dir.join("remotes").join("origin").exists()
        || repo.common_dir.join("branches").join("origin").exists()
    {
        return None;
    }
    let text = fs::read_to_string(repo.common_dir.join("config")).ok()?;
    let parsed = parse_config(&text);
    if parsed.unsure || !global_configs_plain() {
        return None;
    }
    match parsed.origin_urls.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// What `git config --get remote.origin.url` prints: the last `url` of the
/// origin remote in the repo config. `None` when nothing is certain,
/// including when the repo has no origin (a global config could supply one).
/// Unlike `origin_url`, no URL rewrite applies to this read.
pub fn config_origin_url(dir: &Path) -> Option<String> {
    let repo = discover(dir)?;
    let text = fs::read_to_string(repo.common_dir.join("config")).ok()?;
    let parsed = parse_config(&text);
    if parsed.unsure {
        return None;
    }
    parsed.origin_urls.last().cloned()
}

/// What `git rev-parse --abbrev-ref HEAD` prints: the branch name, or `HEAD`
/// when detached. `None` for an unborn branch (git fails there), for a branch
/// name that a tag shares (git prints `heads/<name>`), and when not certain.
pub fn abbrev_head(dir: &Path) -> Option<String> {
    let repo = discover(dir)?;
    match head_of(&repo)? {
        Head::Detached(_) => Some("HEAD".to_string()),
        Head::Branch(name) => {
            branch_sha(&repo, &name)?;
            (!ref_exists(&repo, &format!("refs/tags/{name}"))).then_some(name)
        }
    }
}

fn ref_exists(repo: &Repo, name: &str) -> bool {
    if repo.common_dir.join(name).exists() {
        return true;
    }
    fs::read_to_string(repo.common_dir.join("packed-refs"))
        .map(|packed| {
            packed
                .lines()
                .any(|l| l.split_once(' ').is_some_and(|(_, r)| r == name))
        })
        .unwrap_or(false)
}

fn head_of(repo: &Repo) -> Option<Head> {
    let text = fs::read_to_string(repo.git_dir.join("HEAD")).ok()?;
    let line = text.trim_end_matches(['\n', '\r']);
    if let Some(target) = line.strip_prefix("ref: ") {
        let name = target.strip_prefix("refs/heads/")?;
        if name.is_empty() || name == ".invalid" || name.contains(char::is_whitespace) {
            return None;
        }
        return Some(Head::Branch(name.to_string()));
    }
    is_hex_sha(line).then(|| Head::Detached(line.to_string()))
}

fn branch_sha(repo: &Repo, name: &str) -> Option<String> {
    let loose = repo.common_dir.join("refs").join("heads").join(name);
    if let Ok(text) = fs::read_to_string(&loose) {
        let sha = text.trim();
        return is_hex_sha(sha).then(|| sha.to_string());
    }
    let packed = fs::read_to_string(repo.common_dir.join("packed-refs")).ok()?;
    let wanted = format!("refs/heads/{name}");
    for line in packed.lines() {
        if line.starts_with(['#', '^']) {
            continue;
        }
        let (sha, reference) = line.split_once(' ')?;
        if reference == wanted {
            return is_hex_sha(sha).then(|| sha.to_string());
        }
    }
    None
}

fn is_hex_sha(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(unix)]
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device(_: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn owned_by_me(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    fs::symlink_metadata(path).is_ok_and(|m| m.uid() == me)
}

#[cfg(not(unix))]
fn owned_by_me(_: &Path) -> bool {
    false
}

/// The repo config must hold nothing this module does not model.
fn config_is_plain(repo: &Repo) -> bool {
    let Ok(text) = fs::read_to_string(repo.common_dir.join("config")) else {
        return false;
    };
    !parse_config(&text).unsure
}

#[derive(Debug, Default)]
struct ParsedConfig {
    origin_urls: Vec<String>,
    /// Something is present that changes git's answer and is not modelled.
    unsure: bool,
}

/// A line-based reader for the parts of git config this module relies on.
/// Anything it cannot read with certainty sets `unsure`.
fn parse_config(text: &str) -> ParsedConfig {
    let mut out = ParsedConfig::default();
    let mut section = String::new();
    let mut subsection: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(close) = rest.find(']') else {
                out.unsure = true;
                return out;
            };
            let tail = rest[close + 1..].trim();
            if !tail.is_empty() && !tail.starts_with('#') && !tail.starts_with(';') {
                out.unsure = true;
                return out;
            }
            let inner = &rest[..close];
            if let Some(quote) = inner.find('"') {
                section = inner[..quote].trim().to_ascii_lowercase();
                let sub = inner[quote + 1..].trim_end_matches('"');
                if sub.contains(['\\', '"']) {
                    out.unsure = true;
                    return out;
                }
                subsection = Some(sub.to_string());
            } else if inner.contains('.') {
                // The old dotted form lowercases the subsection: not modelled.
                let name = inner.split('.').next().unwrap_or("").to_ascii_lowercase();
                if matches!(name.as_str(), "remote" | "url" | "include" | "includeif") {
                    out.unsure = true;
                    return out;
                }
                section = name;
                subsection = Some(String::new());
            } else {
                section = inner.trim().to_ascii_lowercase();
                subsection = None;
            }
            if section == "include" || section == "includeif" {
                out.unsure = true;
                return out;
            }
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), Some(v.trim())),
            None => (line.to_ascii_lowercase(), None),
        };
        match (section.as_str(), key.as_str()) {
            ("core", "worktree") => out.unsure = true,
            ("core", "bare") => {
                let off = value.is_some_and(|v| {
                    matches!(
                        v.to_ascii_lowercase().as_str(),
                        "false" | "no" | "off" | "0"
                    )
                });
                if !off {
                    out.unsure = true;
                }
            }
            ("extensions", "refstorage" | "worktreeconfig") => out.unsure = true,
            ("url", "insteadof" | "pushinsteadof") => out.unsure = true,
            ("remote", "url") if subsection.as_deref() == Some("origin") => {
                let Some(value) = value else {
                    out.unsure = true;
                    return out;
                };
                if value.is_empty() || value.contains(['"', '\\', '#', ';']) {
                    out.unsure = true;
                    return out;
                }
                out.origin_urls.push(value.to_string());
            }
            _ => {}
        }
        if key == "insteadof" || key == "pushinsteadof" {
            out.unsure = true;
        }
    }
    out
}

/// The user and system config files can rewrite URLs or include more files.
fn global_configs_plain() -> bool {
    if std::env::var_os("GIT_CONFIG_NOSYSTEM").is_none() {
        let system = std::env::var_os("GIT_CONFIG_SYSTEM")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/gitconfig"));
        if !plain_file(&system) {
            return false;
        }
    }
    if let Some(global) = std::env::var_os("GIT_CONFIG_GLOBAL") {
        return plain_file(Path::new(&global));
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    plain_file(&home.join(".gitconfig")) && plain_file(&xdg.join("git").join("config"))
}

/// A missing file is plain. An unreadable one is not.
fn plain_file(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Ok(text) => !parse_config(&text).unsure,
        Err(err) => err.kind() == std::io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::{lock_cwd, scratch_dir};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_string()
    }

    fn repo(tag: &str) -> PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).expect("create dir");
        let dir = fs::canonicalize(dir).expect("canonical dir");
        git(&dir, &["init", "-q", "-b", "main"]);
        dir
    }

    fn commit(dir: &Path, name: &str) {
        fs::write(dir.join(name), name).expect("write file");
        git(dir, &["add", name]);
        git(dir, &["commit", "-q", "-m", name]);
    }

    /// Run `f` with a clean git environment: the host's own global config
    /// must not change what git or this module see.
    fn isolated<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_cwd();
        let keep: Vec<(&str, Option<std::ffi::OsString>)> =
            ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"]
                .iter()
                .map(|k| (*k, std::env::var_os(k)))
                .collect();
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        let out = f();
        for (k, v) in keep {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        out
    }

    /// The disk answers equal git's answers for the same directory.
    fn assert_matches_git(dir: &Path) {
        let top = git(dir, &["rev-parse", "--show-toplevel"]);
        assert_eq!(toplevel(dir), Some(PathBuf::from(&top)), "toplevel {dir:?}");
        let branch = git(dir, &["branch", "--show-current"]);
        let got = current_branch(dir).expect("branch is certain");
        assert_eq!(got.unwrap_or_default(), branch, "branch {dir:?}");
    }

    #[test]
    fn a_fresh_repo_with_no_commits_matches_git() {
        isolated(|| {
            let dir = repo("gf-fresh");
            assert_matches_git(&dir);
            // Unborn branch: `git rev-parse HEAD` fails, so the disk says none.
            assert_eq!(head_sha(&dir), None);
        });
    }

    #[test]
    fn a_branch_with_commits_matches_git() {
        isolated(|| {
            let dir = repo("gf-branch");
            commit(&dir, "a");
            git(&dir, &["checkout", "-q", "-b", "feat/x"]);
            assert_matches_git(&dir);
            assert_eq!(head_sha(&dir), Some(git(&dir, &["rev-parse", "HEAD"])));
        });
    }

    #[test]
    fn a_subdirectory_resolves_to_the_same_toplevel() {
        isolated(|| {
            let dir = repo("gf-sub");
            commit(&dir, "a");
            let sub = dir.join("x").join("y");
            fs::create_dir_all(&sub).expect("subdir");
            assert_matches_git(&sub);
        });
    }

    #[test]
    fn a_detached_head_matches_git() {
        isolated(|| {
            let dir = repo("gf-detached");
            commit(&dir, "a");
            commit(&dir, "b");
            git(&dir, &["checkout", "-q", "--detach", "HEAD~1"]);
            assert_matches_git(&dir);
            assert_eq!(current_branch(&dir), Some(None));
            assert_eq!(head_sha(&dir), Some(git(&dir, &["rev-parse", "HEAD"])));
        });
    }

    #[test]
    fn abbrev_head_matches_git() {
        isolated(|| {
            let dir = repo("gf-abbrev");
            // Unborn: git fails, so the disk steps aside.
            assert_eq!(abbrev_head(&dir), None);
            commit(&dir, "a");
            assert_eq!(
                abbrev_head(&dir),
                Some(git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]))
            );
            git(&dir, &["checkout", "-q", "--detach"]);
            assert_eq!(
                abbrev_head(&dir),
                Some(git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]))
            );
            git(&dir, &["checkout", "-q", "main"]);
            // A tag that shares the branch name makes git print `heads/main`.
            git(&dir, &["tag", "main"]);
            assert_eq!(abbrev_head(&dir), None);
            assert_eq!(
                git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                "heads/main"
            );
        });
    }

    #[test]
    fn packed_refs_are_read() {
        isolated(|| {
            let dir = repo("gf-packed");
            commit(&dir, "a");
            git(&dir, &["pack-refs", "--all", "--prune"]);
            assert!(!dir.join(".git/refs/heads/main").exists());
            assert_eq!(head_sha(&dir), Some(git(&dir, &["rev-parse", "HEAD"])));
        });
    }

    #[test]
    fn a_linked_worktree_matches_git() {
        isolated(|| {
            let dir = repo("gf-wt");
            commit(&dir, "a");
            let wt = dir.parent().expect("parent").join("gf-wt-linked");
            let _ = fs::remove_dir_all(&wt);
            git(
                &dir,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "side",
                    wt.to_str().expect("utf8"),
                ],
            );
            let wt = fs::canonicalize(&wt).expect("canonical worktree");
            assert_matches_git(&wt);
            assert_eq!(current_branch(&wt), Some(Some("side".to_string())));
            assert_eq!(head_sha(&wt), Some(git(&wt, &["rev-parse", "HEAD"])));
            assert_eq!(
                discover(&wt).expect("worktree repo").common_dir,
                dir.join(".git")
            );
        });
    }

    #[test]
    fn origin_urls_match_git_for_common_shapes() {
        isolated(|| {
            for (i, url) in [
                "https://github.com/acme/widgets.git",
                "git@github.com:acme/widgets.git",
                "ssh://git@github.com/acme/widgets.git/",
                "/srv/git/widgets.git",
                "file:///srv/git/widgets",
                "https://user:p%40ss@example.com/a/b",
            ]
            .iter()
            .enumerate()
            {
                let dir = repo(&format!("gf-url-{i}"));
                git(&dir, &["remote", "add", "origin", url]);
                assert_eq!(
                    origin_url(&dir),
                    Some(git(&dir, &["remote", "get-url", "origin"]))
                );
                assert_eq!(
                    origin_url(&dir),
                    Some(git(&dir, &["config", "--get", "remote.origin.url"]))
                );
            }
        });
    }

    #[test]
    fn no_origin_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-noorigin");
            assert_eq!(origin_url(&dir), None);
        });
    }

    #[test]
    fn a_second_origin_url_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-twourls");
            git(&dir, &["remote", "add", "origin", "https://a.example/x/y"]);
            git(
                &dir,
                &[
                    "config",
                    "--add",
                    "remote.origin.url",
                    "https://b.example/x/y",
                ],
            );
            assert_eq!(origin_url(&dir), None);
        });
    }

    #[test]
    fn insteadof_in_the_repo_config_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-instead");
            git(&dir, &["remote", "add", "origin", "gh:acme/widgets"]);
            git(
                &dir,
                &["config", "url.https://github.com/.insteadOf", "gh:"],
            );
            assert_eq!(origin_url(&dir), None);
            assert_eq!(
                git(&dir, &["remote", "get-url", "origin"]),
                "https://github.com/acme/widgets"
            );
        });
    }

    #[test]
    fn insteadof_in_the_global_config_falls_back_to_git() {
        let _guard = lock_cwd();
        let dir = repo("gf-ginstead");
        git(&dir, &["remote", "add", "origin", "gh:acme/widgets"]);
        let global = scratch_dir("gf-global-cfg");
        fs::create_dir_all(&global).expect("global dir");
        let file = global.join("gitconfig");
        fs::write(&file, "[url \"https://github.com/\"]\n\tinsteadOf = gh:\n").expect("write");
        let before = std::env::var_os("GIT_CONFIG_GLOBAL");
        std::env::set_var("GIT_CONFIG_GLOBAL", &file);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        let got = origin_url(&dir);
        std::env::remove_var("GIT_CONFIG_NOSYSTEM");
        match before {
            Some(v) => std::env::set_var("GIT_CONFIG_GLOBAL", v),
            None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
        }
        assert_eq!(got, None);
    }

    #[test]
    fn an_include_in_the_repo_config_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-include");
            git(&dir, &["remote", "add", "origin", "https://a.example/x/y"]);
            git(&dir, &["config", "include.path", "../other"]);
            assert_eq!(origin_url(&dir), None);
        });
    }

    #[test]
    fn a_bare_repo_falls_back_to_git() {
        isolated(|| {
            let dir = scratch_dir("gf-bare");
            fs::create_dir_all(&dir).expect("dir");
            git(&dir, &["init", "-q", "--bare", "-b", "main"]);
            assert_eq!(discover(&dir), None);
        });
    }

    #[test]
    fn core_worktree_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-corewt");
            git(&dir, &["config", "core.worktree", "/somewhere"]);
            assert_eq!(discover(&dir), None);
        });
    }

    #[test]
    fn a_submodule_falls_back_to_git() {
        isolated(|| {
            let inner = repo("gf-sub-inner");
            commit(&inner, "a");
            let outer = repo("gf-sub-outer");
            commit(&outer, "o");
            let status = Command::new("git")
                .arg("-C")
                .arg(&outer)
                .args(["-c", "protocol.file.allow=always", "submodule", "add", "-q"])
                .arg(&inner)
                .arg("child")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .status()
                .expect("run git");
            assert!(status.success());
            // core.worktree is set inside the submodule's own git dir.
            assert_eq!(discover(&outer.join("child")), None);
            // The outer repo is still read from disk.
            assert_matches_git(&outer);
        });
    }

    #[test]
    fn a_non_repo_directory_is_not_decided_here() {
        isolated(|| {
            let dir = scratch_dir("gf-none");
            fs::create_dir_all(&dir).expect("dir");
            assert_eq!(discover(&dir), None);
        });
    }

    #[test]
    fn git_dir_env_falls_back_to_git() {
        let _guard = lock_cwd();
        let dir = repo("gf-env");
        std::env::set_var("GIT_DIR", dir.join(".git"));
        let got = discover(&dir);
        std::env::remove_var("GIT_DIR");
        assert_eq!(got, None);
    }

    #[test]
    fn a_folder_named_dot_git_falls_back_to_git() {
        isolated(|| {
            let dir = repo("gf-indot");
            assert_eq!(discover(&dir.join(".git")), None);
            assert_eq!(discover(&dir.join(".git").join("refs")), None);
        });
    }

    #[test]
    fn config_parsing_handles_odd_but_plain_files() {
        let p = parse_config(
            "# comment\n[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n[remote \"origin\"]\n\turl = https://h/o/r\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n",
        );
        assert!(!p.unsure);
        assert_eq!(p.origin_urls, ["https://h/o/r"]);
        // The key is case-insensitive, the subsection is not.
        let p = parse_config("[Remote \"origin\"]\n\tURL = https://h/o/r\n[remote \"Origin\"]\n\turl = https://x/y/z\n");
        assert_eq!(p.origin_urls, ["https://h/o/r"]);
        for odd in [
            "[remote \"origin\"]\n\turl = \"quoted\"\n",
            "[remote \"origin\"]\n\turl = a \\\n b\n",
            "[remote \"origin\"]\n\turl = a # note\n",
            "[remote.origin]\n\turl = a\n",
            "[core] bare = true\n",
            "[core]\n\tbare = true\n",
            "[core]\n\tworktree = /x\n",
            "[extensions]\n\trefStorage = reftable\n",
            "[includeIf \"gitdir:~/\"]\n\tpath = x\n",
        ] {
            assert!(parse_config(odd).unsure, "{odd:?}");
        }
    }
}
