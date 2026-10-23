// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Wires tab completion for `playbook` and `pb` into the user's shell files
//! (ADR 0054): small loader stubs in each shell's autoload directory, and for
//! zsh one marked block in `~/.zshrc`.

use crate::common::atomic::write_atomic_durable;
use crate::init::path::{self, PathShell};
use crate::init::shim::{self, RcStrip};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// Comment that opens the zsh block in `~/.zshrc`.
pub const MARKER: &str = "# playbook completions";
/// The exact lines of the zsh block, marker first.
pub const ZSH_BLOCK_LINES: [&str; 3] = [
    MARKER,
    r#"fpath=("$HOME/.local/share/zsh/site-functions" $fpath)"#,
    "if (( $+functions[compdef] )); then autoload -Uz _playbook; compdef _playbook playbook; [[ ${commands[pb]} -ef ${commands[playbook]} ]] && compdef _playbook pb; else autoload -Uz compinit && compinit; fi",
];
pub const ZSHRC: &str = ".zshrc";
pub const ZSHENV: &str = ".zshenv";
pub const ZSH_SITE_FUNCTIONS: &str = ".local/share/zsh/site-functions";
pub const BASH_COMPLETIONS: &str = ".local/share/bash-completion/completions";
pub const FISH_COMPLETIONS: &str = ".config/fish/completions";
pub const BINARY_NAME: &str = "playbook";
pub const ALIAS_NAME: &str = "pb";
pub const FISH_SUFFIX: &str = ".fish";
pub const ZSH_STUB_NAME: &str = "_playbook";
/// Mode of a `~/.zshrc` this module creates; an existing file keeps its own.
pub const RC_NEW_MODE: u32 = 0o644;
/// Mode of the directories this module creates, whatever a loose umask says.
#[cfg(unix)]
const DIR_MODE: u32 = 0o755;
pub const SKIP_DANGLING: &str = "dangling symlink";
pub const SKIP_OTHER_OWNER: &str = "owned by another user";
pub const SKIP_READ_ONLY: &str = "read-only";
pub const SKIP_NOT_OURS: &str = "not written by playbook";
pub const SKIP_UNREADABLE: &str = "unreadable";
pub const SKIP_UNKNOWN_SHELL: &str = "unknown or unset $SHELL";
pub const BACKUP_INFIX: &str = ".bak-";
/// Second line of every stub, which tells our files from a user's.
pub const STUB_MARKER: &str = "# playbook completions (written by playbook setup)";
/// zsh reads the `#compdef` tag only from the first line of a function file.
const ZSH_STUB_HEAD: &str = "#compdef playbook pb\n";
const BASH_STUB_BODY: &str = "command -v playbook >/dev/null 2>&1 || return 0\n\
source <(playbook completions bash 2>/dev/null) 2>/dev/null || return 0\n";
// The script is captured first so a broken `playbook` that prints on stdout
// and fails is never sourced.
const FISH_STUB_BODY: &str = "if command -q playbook\n\
\x20   set -l script (playbook completions fish 2>/dev/null)\n\
\x20   if test $status -eq 0; and test -n \"$script\"\n\
\x20       printf '%s\\n' $script | source\n\
\x20   end\n\
end\n";
// The generated script's own tail does not run `_playbook` under `eval`, so
// the stub calls it after the load.
const ZSH_STUB_BODY: &str =
    "if [[ $service == pb && ! ${commands[pb]} -ef ${commands[playbook]} ]]; then\n\
\x20 _default \"$@\"\n\
\x20 return\n\
fi\n\
(( $+commands[playbook] )) || return 0\n\
local script\n\
script=$(playbook completions zsh 2>/dev/null) || return 0\n\
eval \"$script\" 2>/dev/null || return 0\n\
_playbook \"$@\"\n";

/// The shell the completion files are written for.
pub type Shell = PathShell;

/// What `ensure` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Files this call created.
    pub created: Vec<PathBuf>,
    /// Files this call changed.
    pub changed: Vec<PathBuf>,
    /// Files that already held what setup would write.
    pub already: Vec<PathBuf>,
    /// Files with an edited block or stub, left alone.
    pub kept: Vec<PathBuf>,
    /// Files left alone, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// The effective user and a hook for tests.
pub struct Ctx<'a> {
    /// The uid a file must belong to for setup to rewrite it.
    pub uid: u32,
    /// Called with the rc path between the read and the write, on every path.
    pub between: &'a dyn Fn(&Path),
}

/// What doctor reports for one shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// No stub, or no way for zsh to find it.
    Missing,
    Ok,
    /// A stub path holds a file without our marker.
    StubNotOurs,
    /// The zsh block was changed by hand.
    BlockEdited,
    /// The zsh block appears more than once.
    BlockDuplicated,
}

/// Writes the completion stubs for `shell` under `home`, and for zsh the
/// block in `~/.zshrc`, as the current user. Never overwrites a file it did
/// not write.
pub fn ensure(home: &Path, shell: Shell) -> io::Result<Outcome> {
    ensure_with(
        home,
        shell,
        &Ctx {
            uid: effective_uid(),
            between: &|_| {},
        },
    )
}

#[cfg(unix)]
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Windows has no uid, and files there are not owner-checked.
#[cfg(not(unix))]
pub fn effective_uid() -> u32 {
    0
}

#[cfg(unix)]
fn owned_by(meta: &fs::Metadata, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == uid
}

#[cfg(not(unix))]
fn owned_by(_meta: &fs::Metadata, _uid: u32) -> bool {
    true
}

/// [`ensure`] with an explicit effective uid and a test hook. A file or
/// directory is created only under a `home` that uid owns.
pub fn ensure_with(home: &Path, shell: Shell, ctx: &Ctx) -> io::Result<Outcome> {
    let mut out = Outcome::default();
    let stubs = stub_paths(home, shell);
    if stubs.is_empty() {
        out.skipped
            .push((home.to_path_buf(), SKIP_UNKNOWN_SHELL.to_string()));
        return Ok(out);
    }
    let text = stub_text(shell);
    let home_owned = owned_by(&fs::metadata(home)?, ctx.uid);
    for stub in stubs {
        ensure_stub(stub, &text, home_owned, &mut out)?;
    }
    if shell == Shell::Zsh {
        ensure_zshrc(home, home_owned, ctx, &mut out)?;
    }
    Ok(out)
}

/// The stub files for `shell`, the `pb` ones included.
pub fn stub_paths(home: &Path, shell: Shell) -> Vec<PathBuf> {
    let names = |dir: &str, suffix: &str| {
        [BINARY_NAME, ALIAS_NAME]
            .map(|name| home.join(dir).join(format!("{name}{suffix}")))
            .to_vec()
    };
    match shell {
        Shell::Bash => names(BASH_COMPLETIONS, ""),
        Shell::Fish => names(FISH_COMPLETIONS, FISH_SUFFIX),
        Shell::Zsh => vec![home.join(ZSH_SITE_FUNCTIONS).join(ZSH_STUB_NAME)],
        Shell::Other => Vec::new(),
    }
}

/// The full text of the loader stub written for `shell`.
pub fn stub_text(shell: Shell) -> String {
    let (head, body) = match shell {
        Shell::Bash => ("", BASH_STUB_BODY),
        Shell::Fish => ("", FISH_STUB_BODY),
        Shell::Zsh => (ZSH_STUB_HEAD, ZSH_STUB_BODY),
        Shell::Other => return String::new(),
    };
    format!("{head}{STUB_MARKER}\n{body}")
}

fn ensure_stub(path: PathBuf, text: &str, home_owned: bool, out: &mut Outcome) -> io::Result<()> {
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound && !home_owned => {
            out.skipped.push((path, SKIP_OTHER_OWNER.to_string()));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            record_write(path.clone(), create_stub(&path, text), true, out)?;
        }
        Err(e) => return Err(e),
        Ok(_) => match fs::read(&path) {
            Ok(bytes) if has_line(&bytes, STUB_MARKER) && bytes == text.as_bytes() => {
                out.already.push(path);
            }
            Ok(bytes) if has_line(&bytes, STUB_MARKER) => out.kept.push(path),
            Ok(_) => out.skipped.push((path, SKIP_NOT_OURS.to_string())),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                out.skipped.push((path, SKIP_DANGLING.to_string()));
            }
            Err(_) => out.skipped.push((path, SKIP_UNREADABLE.to_string())),
        },
    }
    Ok(())
}

fn create_stub(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        create_dirs(dir)?;
    }
    create_file(path, text.as_bytes())
}

#[cfg(unix)]
fn create_dirs(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
}

#[cfg(not(unix))]
fn create_dirs(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// `write_atomic_durable` sets permission bits only on an existing file, so a
/// new one would carry the umask; set the mode explicitly.
#[cfg(unix)]
fn create_file(path: &Path, content: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    write_atomic_durable(path, content)?;
    fs::set_permissions(path, fs::Permissions::from_mode(RC_NEW_MODE))
}

#[cfg(not(unix))]
fn create_file(path: &Path, content: &[u8]) -> io::Result<()> {
    write_atomic_durable(path, content)
}

fn has_line(content: &[u8], wanted: &str) -> bool {
    shim::split_lines(content)
        .iter()
        .any(|l| l.trim_ascii_end() == wanted.as_bytes())
}

enum Rc {
    Missing,
    Skip(&'static str),
    Present { target: PathBuf, content: Vec<u8> },
}

fn read_rc(rc: &Path, uid: u32) -> io::Result<Rc> {
    match fs::symlink_metadata(rc) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Rc::Missing),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let target = match fs::canonicalize(rc) {
        Ok(target) => target,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Rc::Skip(SKIP_DANGLING)),
        Err(e) => return Err(e),
    };
    if !owned_by(&fs::metadata(&target)?, uid) {
        return Ok(Rc::Skip(SKIP_OTHER_OWNER));
    }
    Ok(match fs::read(&target) {
        Ok(content) => Rc::Present { target, content },
        Err(_) => Rc::Skip(SKIP_UNREADABLE),
    })
}

fn zsh_block() -> String {
    format!("{}\n", ZSH_BLOCK_LINES.join("\n"))
}

/// For each marker line in `content`, whether the block after it is exactly
/// ours.
fn marked_blocks(content: &[u8]) -> Vec<bool> {
    let lines = shim::split_lines(content);
    (0..lines.len())
        .filter(|&i| lines[i].trim_ascii_end() == MARKER.as_bytes())
        .map(|i| shim::is_marked_block(&lines, i, MARKER, &ZSH_BLOCK_LINES[1..]))
        .collect()
}

/// Whether `~/.zshrc` or `~/.zshenv` already puts the site-functions dir on
/// `fpath` by the user's own line.
fn names_site_functions(home: &Path, zshrc: &[u8]) -> bool {
    let zshenv = fs::read(home.join(ZSHENV)).unwrap_or_default();
    path::names_dir(zshrc, ZSH_SITE_FUNCTIONS) || path::names_dir(&zshenv, ZSH_SITE_FUNCTIONS)
}

fn ensure_zshrc(home: &Path, home_owned: bool, ctx: &Ctx, out: &mut Outcome) -> io::Result<()> {
    let rc = home.join(ZSHRC);
    let state = read_rc(&rc, ctx.uid)?;
    (ctx.between)(&rc);
    let existing = match state {
        Rc::Skip(why) => {
            out.skipped.push((rc, why.to_string()));
            return Ok(());
        }
        Rc::Missing if !home_owned => {
            out.skipped.push((rc, SKIP_OTHER_OWNER.to_string()));
            return Ok(());
        }
        Rc::Missing => None,
        Rc::Present { target, content } => Some((target, content)),
    };
    let created = existing.is_none();
    let content = existing.as_ref().map_or(&[][..], |(_, c)| c.as_slice());
    match marked_blocks(content).first() {
        Some(true) => out.already.push(rc),
        Some(false) => out.kept.push(rc),
        None if names_site_functions(home, content) => out.already.push(rc),
        None => {
            let result = match existing {
                None => create_file(&rc, zsh_block().as_bytes()),
                Some((target, content)) => append_block(&rc, &target, &content),
            };
            record_write(rc, result, created, out)?;
        }
    }
    Ok(())
}

/// Appends the block through an atomic replace of the real target, so a
/// symlinked rc stays a symlink. The replace would succeed on a read-only
/// file in a writable directory, so writability is checked first.
fn append_block(rc: &Path, target: &Path, original: &[u8]) -> io::Result<()> {
    if !shim::can_rewrite(rc) {
        return Err(ErrorKind::PermissionDenied.into());
    }
    let mut appended = original.to_vec();
    if !appended.is_empty() && !appended.ends_with(b"\n") {
        appended.push(b'\n');
    }
    appended.extend_from_slice(zsh_block().as_bytes());
    write_atomic_durable(target, appended)
}

fn record_write(
    path: PathBuf,
    result: io::Result<()>,
    created: bool,
    out: &mut Outcome,
) -> io::Result<()> {
    match result {
        Ok(()) if created => out.created.push(path),
        Ok(()) => out.changed.push(path),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            out.skipped.push((path, SKIP_READ_ONLY.to_string()));
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// What doctor should report for `shell` under `home`, from the files on disk.
pub fn status(home: &Path, shell: Shell) -> Status {
    let stubs = stub_paths(home, shell);
    if stubs.is_empty() {
        return Status::Missing;
    }
    let contents: Vec<Option<Vec<u8>>> = stubs.iter().map(|p| fs::read(p).ok()).collect();
    if contents.iter().any(Option::is_none) {
        return Status::Missing;
    }
    if contents.iter().flatten().any(|c| !has_line(c, STUB_MARKER)) {
        return Status::StubNotOurs;
    }
    if shell != Shell::Zsh {
        return Status::Ok;
    }
    let zshrc = fs::read(home.join(ZSHRC)).unwrap_or_default();
    match marked_blocks(&zshrc).as_slice() {
        [] if names_site_functions(home, &zshrc) => Status::Ok,
        [] => Status::Missing,
        [true] => Status::Ok,
        [false] => Status::BlockEdited,
        _ => Status::BlockDuplicated,
    }
}

/// Removes the exact zsh block from `~/.zshrc` after a backup, for uninstall.
/// A file whose real target belongs to a uid other than `uid` is reported and
/// left alone. Stub files are the caller's to remove.
pub fn strip(home: &Path, uid: u32, stamp: u64, dry_run: bool) -> Vec<RcStrip> {
    let rc = home.join(ZSHRC);
    let Ok(target) = fs::canonicalize(&rc) else {
        return Vec::new();
    };
    let Ok(old) = fs::read(&target) else {
        return Vec::new();
    };
    let Some(new) = shim::strip_marked_block(&old, MARKER, &ZSH_BLOCK_LINES[1..]) else {
        return Vec::new();
    };
    let mut entry = RcStrip {
        rc_file: rc.clone(),
        backup: None,
        unwritable: !shim::can_rewrite(&rc),
        error: None,
    };
    if !fs::metadata(&target).is_ok_and(|meta| owned_by(&meta, uid)) {
        entry.error = Some(SKIP_OTHER_OWNER.to_string());
        return vec![entry];
    }
    if !entry.unwritable && !dry_run {
        let write = backup_unless_append_only(&rc, &old, &new, stamp).and_then(|backup| {
            write_atomic_durable(&target, &new)?;
            Ok(backup)
        });
        match write {
            Ok(backup) => entry.backup = backup,
            Err(e) => entry.error = Some(e.to_string()),
        }
    }
    vec![entry]
}

/// Copies `rc` to `<rc>.bak-<stamp>` unless `new` only appends to `old`.
/// An existing backup under that name is kept and returned as is: it holds
/// the file from before an earlier step of the same run rewrote it.
pub fn backup_unless_append_only(
    rc: &Path,
    old: &[u8],
    new: &[u8],
    stamp: u64,
) -> io::Result<Option<PathBuf>> {
    if new.starts_with(old) {
        return Ok(None);
    }
    let name = rc.file_name().unwrap_or_default().to_string_lossy();
    let backup = rc.with_file_name(format!("{name}{BACKUP_INFIX}{stamp}"));
    if !backup.exists() {
        fs::copy(rc, &backup)?;
    }
    Ok(Some(backup))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::process::Command;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, SystemTime};

    const RENDEZVOUS_TIMEOUT: Duration = Duration::from_secs(5);
    const PRIVATE_MODE: u32 = 0o600;
    const READ_ONLY_MODE: u32 = 0o444;
    const MODE_BITS: u32 = 0o777;
    const OLD_MTIME_SECS: u64 = 86_400;
    const STAMP: u64 = 1_700_000_000;
    const PRIVATE_UMASK: &str = "077";
    const CHILD_HOME_VAR: &str = "PB_COMPLETION_CHILD_HOME";
    const LOOSE_UMASK: &str = "002";
    const DIR_MODE_BITS: u32 = 0o755;
    const FOREIGN_FILE: &str = "/etc/hosts";
    const CHILD_TEST: &str = "umask_child_creates_the_zshrc";
    const ROOT_SKIP: &str = "skipped: euid 0 ignores file modes";
    const CI_VAR: &str = "CI";
    const USER_LINES: &str = "export EDITOR=vim\nalias ll='ls -l'\n";
    const NO_NEWLINE: &str = "export EDITOR=vim\nalias ll='ls -l'";
    const CREATED: &str = "created";

    fn home(tag: &str) -> PathBuf {
        let dir = crate::common::test_support::scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zshrc(home: &Path) -> PathBuf {
        home.join(ZSHRC)
    }

    fn ctx_for(uid: u32) -> Ctx<'static> {
        Ctx {
            uid,
            between: &|_| {},
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & MODE_BITS
    }

    /// File text, or a marker string when unreadable, so a missing file fails an
    /// assertion instead of panicking in setup.
    fn read(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap_or_else(|e| format!("<unreadable: {e}>"))
    }

    fn marker_lines(path: &Path) -> usize {
        read(path).lines().filter(|l| *l == MARKER).count()
    }

    fn backups(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(BACKUP_INFIX))
            .collect()
    }

    fn listing(dir: &Path) -> BTreeSet<PathBuf> {
        let mut found = BTreeSet::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in fs::read_dir(&next).unwrap().filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() && !path.is_symlink() {
                    pending.push(path.clone());
                }
                found.insert(path);
            }
        }
        found
    }

    fn set_old_mtime(path: &Path) -> SystemTime {
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(OLD_MTIME_SECS);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        old
    }

    /// True when the test must stop: root ignores file modes, and in CI that
    /// is a failure rather than a skip.
    fn root_cannot_prove_modes() -> bool {
        if effective_uid() != 0 {
            return false;
        }
        assert!(
            std::env::var_os(CI_VAR).is_none(),
            "{ROOT_SKIP}, but CI is set"
        );
        println!("{ROOT_SKIP}");
        true
    }

    #[test]
    fn a_missing_zshrc_is_created_with_the_block_and_mode_0644() {
        // Arrange
        let home = home("completion-create");

        // Act
        let out = ensure(&home, Shell::Zsh).unwrap();

        // Assert
        let rc = zshrc(&home);
        assert_eq!(read(&rc), zsh_block());
        assert_eq!(mode(&rc), RC_NEW_MODE);
        assert!(backups(&home).is_empty(), "{:?}", backups(&home));
        assert!(out.created.contains(&rc), "{CREATED}: {out:?}");
    }

    #[test]
    fn an_append_only_change_takes_no_backup() {
        // Arrange
        let home = home("completion-append");
        fs::write(zshrc(&home), USER_LINES).unwrap();

        // Act
        ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert_eq!(read(zshrc(&home)), format!("{USER_LINES}{}", zsh_block()));
        assert!(backups(&home).is_empty(), "{:?}", backups(&home));
    }

    #[test]
    fn an_rc_fixture_without_a_trailing_newline_or_empty_gets_one_block_on_its_own_line() {
        // Arrange
        let rows = [
            ("with-newline", USER_LINES, false),
            ("no-newline", NO_NEWLINE, true),
            ("empty", "", true),
        ];
        for (tag, original, second_run_is_checked) in rows {
            let home = home(&format!("completion-fixture-{tag}"));
            fs::write(zshrc(&home), original).unwrap();
            let separator = if original.is_empty() || original.ends_with('\n') {
                ""
            } else {
                "\n"
            };

            // Act
            ensure(&home, Shell::Zsh).unwrap();
            let after_first = fs::read(zshrc(&home)).unwrap();
            if second_run_is_checked {
                ensure(&home, Shell::Zsh).unwrap();
            }

            // Assert
            assert_eq!(
                String::from_utf8(after_first).unwrap(),
                format!("{original}{separator}{}", zsh_block()),
                "{tag}"
            );
            assert_eq!(marker_lines(&zshrc(&home)), 1, "{tag}");
            assert!(backups(&home).is_empty(), "{tag}: {:?}", backups(&home));
        }
    }

    #[test]
    fn a_dangling_symlink_zshrc_is_skipped() {
        // Arrange
        let home = home("completion-dangling");
        let target = home.join("missing-target");
        symlink(&target, zshrc(&home)).unwrap();

        // Act
        let out = ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert!(!target.exists(), "the target was created");
        assert!(
            out.skipped
                .iter()
                .any(|(p, why)| *p == zshrc(&home) && why == SKIP_DANGLING),
            "{out:?}"
        );
    }

    #[test]
    fn a_rerun_changes_nothing() {
        // Arrange
        let home = home("completion-rerun");
        ensure(&home, Shell::Zsh).unwrap();
        let rc = zshrc(&home);
        assert!(rc.exists(), "the first run wrote no .zshrc");
        let old = set_old_mtime(&rc);
        let bytes = fs::read(&rc).unwrap();
        let files = listing(&home);

        // Act
        let out = ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert_eq!(fs::read(&rc).unwrap(), bytes);
        assert_eq!(fs::metadata(&rc).unwrap().modified().unwrap(), old);
        assert_eq!(listing(&home), files);
        assert!(out.created.is_empty() && out.changed.is_empty(), "{out:?}");
    }

    /// Two parties wait for each other, or panic naming who never came.
    struct Rendezvous {
        arrived: Mutex<Vec<&'static str>>,
        both: Condvar,
    }

    impl Rendezvous {
        fn new() -> Arc<Rendezvous> {
            Arc::new(Rendezvous {
                arrived: Mutex::new(Vec::new()),
                both: Condvar::new(),
            })
        }

        fn arrive(&self, who: &'static str) {
            let mut arrived = self.arrived.lock().unwrap();
            arrived.push(who);
            self.both.notify_all();
            let (arrived, timeout) = self
                .both
                .wait_timeout_while(arrived, RENDEZVOUS_TIMEOUT, |a| a.len() < 2)
                .unwrap();
            assert!(
                !timeout.timed_out(),
                "{who} waited {RENDEZVOUS_TIMEOUT:?}; arrived: {arrived:?}, the other party never did"
            );
        }
    }

    fn interleaved_ensure(home: &Path) {
        let rendezvous = Rendezvous::new();
        let uid = effective_uid();
        let handles: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|who| {
                let home = home.to_path_buf();
                let rendezvous = Arc::clone(&rendezvous);
                std::thread::spawn(move || {
                    let hook = move |_: &Path| rendezvous.arrive(who);
                    let ctx = Ctx {
                        uid,
                        between: &hook,
                    };
                    ensure_with(&home, Shell::Zsh, &ctx).unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("an ensure thread panicked");
        }
    }

    #[test]
    fn two_ensure_calls_forced_to_interleave_leave_exactly_one_block() {
        // Arrange
        let with_block = format!("{USER_LINES}{}", zsh_block());
        let cases = [
            ("create", None, zsh_block()),
            ("append", Some(USER_LINES.to_string()), with_block.clone()),
            ("no-op", Some(with_block.clone()), with_block),
        ];
        for (path, original, expected) in cases {
            let home = home(&format!("completion-race-{path}"));
            if let Some(original) = &original {
                fs::write(zshrc(&home), original).unwrap();
            }

            // Act
            interleaved_ensure(&home);

            // Assert
            assert_eq!(marker_lines(&zshrc(&home)), 1, "{path}");
            assert_eq!(read(zshrc(&home)), expected, "{path}");
        }
    }

    #[test]
    fn an_edited_block_is_kept_and_not_duplicated() {
        // Arrange
        let home = home("completion-edited");
        let edited = format!("{MARKER}\n# my own fpath\n{}\n", ZSH_BLOCK_LINES[2]);
        let original = format!("{USER_LINES}{edited}");
        fs::write(zshrc(&home), &original).unwrap();

        // Act
        let out = ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert_eq!(read(zshrc(&home)), original);
        assert!(out.kept.contains(&zshrc(&home)), "{out:?}");
    }

    #[test]
    fn the_atomic_rewrite_keeps_mode_0600() {
        // Arrange
        let home = home("completion-mode-0600");
        let rc = zshrc(&home);
        fs::write(&rc, USER_LINES).unwrap();
        fs::set_permissions(&rc, fs::Permissions::from_mode(PRIVATE_MODE)).unwrap();

        // Act
        ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert!(read(&rc).ends_with(&zsh_block()));
        assert_eq!(mode(&rc), PRIVATE_MODE);
    }

    /// Runs only as the child of `run_ensure_under_umask`.
    #[test]
    #[ignore = "runs only as a child process under a chosen umask"]
    fn umask_child_creates_the_zshrc() {
        let Some(home) = std::env::var_os(CHILD_HOME_VAR) else {
            return;
        };
        ensure(Path::new(&home), Shell::Zsh).unwrap();
    }

    /// Runs `ensure` for zsh in a child process with `umask`, so the test
    /// process's own umask never changes.
    fn run_ensure_under_umask(umask: &str, home: &Path) {
        let exe = std::env::current_exe().unwrap();
        let test = format!(
            "{}::{CHILD_TEST}",
            module_path!().split_once("::").map_or("", |(_, rest)| rest)
        );
        let script = format!("umask {umask}; exec \"$0\" --ignored --exact \"$1\"");
        let out = Command::new("sh")
            .env_clear()
            .env(CHILD_HOME_VAR, home)
            .args(["-c", &script])
            .arg(&exe)
            .arg(&test)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn a_new_zshrc_is_0644_under_umask_077() {
        // Arrange
        let home = home("completion-umask");

        // Act
        run_ensure_under_umask(PRIVATE_UMASK, &home);

        // Assert
        assert_eq!(read(zshrc(&home)), zsh_block());
        assert_eq!(mode(&zshrc(&home)), RC_NEW_MODE);
    }

    #[test]
    fn created_directories_are_0755_under_a_group_writable_umask() {
        // Arrange
        let home = home("completion-dir-mode");

        // Act
        run_ensure_under_umask(LOOSE_UMASK, &home);

        // Assert
        let site_functions = home.join(ZSH_SITE_FUNCTIONS);
        let chain: Vec<_> = site_functions
            .ancestors()
            .take_while(|dir| *dir != home)
            .collect();
        assert!(!chain.is_empty());
        for dir in chain {
            assert_eq!(mode(dir), DIR_MODE_BITS, "{}", dir.display());
        }
    }

    #[test]
    fn an_rc_owned_by_another_user_is_refused() {
        // Arrange
        let home = home("completion-other-owner");
        let rc = zshrc(&home);
        fs::write(&rc, USER_LINES).unwrap();
        let other = effective_uid() + 1;

        // Act
        let out = ensure_with(&home, Shell::Zsh, &ctx_for(other)).unwrap();

        // Assert
        assert_eq!(read(&rc), USER_LINES);
        assert!(
            out.skipped
                .iter()
                .any(|(p, why)| *p == rc && why == SKIP_OTHER_OWNER),
            "{out:?}"
        );
    }

    #[test]
    fn an_rc_symlinked_to_a_file_of_another_owner_is_refused_by_the_targets_owner() {
        // Arrange
        let home = home("completion-foreign-target");
        if effective_uid() == 0 {
            println!("{ROOT_SKIP}");
            return;
        }
        let target = Path::new(FOREIGN_FILE);
        assert_ne!(fs::metadata(target).unwrap().uid(), effective_uid());
        symlink(target, zshrc(&home)).unwrap();
        let before = fs::read(target).unwrap();

        // Act
        let out = ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert_eq!(fs::read(target).unwrap(), before);
        assert!(
            out.skipped
                .iter()
                .any(|(p, why)| *p == zshrc(&home) && why == SKIP_OTHER_OWNER),
            "{out:?}"
        );
    }

    #[test]
    fn nothing_is_created_under_a_home_owned_by_another_user() {
        // Arrange
        let home = home("completion-foreign-home");
        let other = effective_uid() + 1;

        // Act
        let out = ensure_with(&home, Shell::Zsh, &ctx_for(other)).unwrap();

        // Assert
        assert!(listing(&home).is_empty(), "{:?}", listing(&home));
        assert!(out.created.is_empty(), "{out:?}");
        let skipped: Vec<_> = out.skipped.iter().map(|(_, why)| why.as_str()).collect();
        assert_eq!(skipped, [SKIP_OTHER_OWNER; 2], "{out:?}");
    }

    #[test]
    fn a_symlinked_zshrc_stays_a_symlink() {
        // Arrange
        let home = home("completion-symlink");
        let target = home.join("dotfiles-zshrc");
        fs::write(&target, USER_LINES).unwrap();
        symlink(&target, zshrc(&home)).unwrap();

        // Act
        ensure_with(&home, Shell::Zsh, &ctx_for(effective_uid())).unwrap();

        // Assert
        assert!(zshrc(&home).is_symlink());
        assert_eq!(fs::read_link(zshrc(&home)).unwrap(), target);
        assert_eq!(read(&target), format!("{USER_LINES}{}", zsh_block()));
    }

    #[test]
    fn backup_unless_append_only_backs_up_only_a_non_append() {
        // Arrange
        let old = b"export A=1\n".as_slice();
        let rows: [(&str, Vec<u8>, bool); 2] = [
            ("rewrite", b"export A=2\n".to_vec(), true),
            ("append", [old, b"export B=2\n"].concat(), false),
        ];
        for (tag, new, expects_backup) in rows {
            let home = home(&format!("completion-backup-{tag}"));
            let rc = zshrc(&home);
            fs::write(&rc, old).unwrap();

            // Act
            let backup = backup_unless_append_only(&rc, old, &new, STAMP).unwrap();

            // Assert
            assert_eq!(backup.is_some(), expects_backup, "{tag}");
            match backup {
                Some(path) => {
                    assert_eq!(
                        path.file_name().unwrap().to_string_lossy(),
                        format!("{ZSHRC}{BACKUP_INFIX}{STAMP}")
                    );
                    assert_eq!(fs::read(path).unwrap(), old);
                }
                None => assert!(backups(&home).is_empty(), "{tag}"),
            }
        }
    }

    #[test]
    fn a_symlink_into_a_read_only_target_is_skipped() {
        // Arrange
        let home = home("completion-readonly-link");
        if root_cannot_prove_modes() {
            return;
        }
        let target = home.join("dotfiles-zshrc");
        fs::write(&target, USER_LINES).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(READ_ONLY_MODE)).unwrap();
        symlink(&target, zshrc(&home)).unwrap();

        // Act
        let out = ensure_with(&home, Shell::Zsh, &ctx_for(effective_uid())).unwrap();

        // Assert
        assert_eq!(read(&target), USER_LINES);
        assert!(
            out.skipped
                .iter()
                .any(|(p, why)| *p == zshrc(&home) && why == SKIP_READ_ONLY),
            "{out:?}"
        );
    }

    #[test]
    fn a_read_only_zshrc_is_reported_and_its_bytes_are_unchanged() {
        // Arrange
        let home = home("completion-readonly");
        if root_cannot_prove_modes() {
            return;
        }
        let rc = zshrc(&home);
        fs::write(&rc, USER_LINES).unwrap();
        fs::set_permissions(&rc, fs::Permissions::from_mode(READ_ONLY_MODE)).unwrap();

        // Act
        let out = ensure_with(&home, Shell::Zsh, &ctx_for(effective_uid())).unwrap();

        // Assert
        assert_eq!(read(&rc), USER_LINES);
        assert!(out.skipped.iter().any(|(p, _)| *p == rc), "{out:?}");
    }

    #[test]
    fn a_stub_path_that_is_not_ours_is_never_overwritten() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            // Arrange
            let home = home("completion-foreign-stub");
            let stubs = stub_paths(&home, shell);
            assert!(!stubs.is_empty(), "{shell:?} has no stub paths");
            let theirs = stubs[0].clone();
            fs::create_dir_all(theirs.parent().unwrap()).unwrap();
            fs::write(&theirs, "# somebody else's completion\n").unwrap();

            // Act
            let out = ensure(&home, shell).unwrap();

            // Assert
            assert_eq!(read(&theirs), "# somebody else's completion\n", "{shell:?}");
            assert!(out.skipped.iter().any(|(p, _)| *p == theirs), "{out:?}");
        }
    }

    #[test]
    fn a_zshrc_that_names_the_dir_gets_no_block() {
        // Arrange
        let home = home("completion-names-dir");
        let own = format!(
            "fpath=({} $fpath)\n",
            home.join(ZSH_SITE_FUNCTIONS).display()
        );
        fs::write(zshrc(&home), &own).unwrap();

        // Act
        ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert_eq!(read(zshrc(&home)), own);
    }

    #[test]
    fn the_block_and_stubs_leave_the_retired_launcher_check_clean() {
        // Arrange
        let home = home("completion-layer3");
        fs::write(zshrc(&home), USER_LINES).unwrap();

        // Act
        ensure(&home, Shell::Zsh).unwrap();

        // Assert
        assert!(
            shim::retired_rc_entries(&home).is_empty(),
            "{:?}",
            shim::retired_rc_entries(&home)
        );
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            assert!(
                !stub_text(shell).contains(shim::SOURCE_LINE),
                "{shell:?} stub names the retired launcher line"
            );
        }
    }

    #[test]
    fn status_follows_what_ensure_wrote_and_what_was_changed_after() {
        // Arrange
        let home = home("completion-status");
        let before = status(&home, Shell::Zsh);
        ensure(&home, Shell::Zsh).unwrap();
        let after = status(&home, Shell::Zsh);
        let rc = zshrc(&home);
        let original = read(&rc);

        // Act
        fs::write(&rc, format!("{original}{original}")).unwrap();
        let twice = status(&home, Shell::Zsh);
        fs::write(&rc, original.replace(ZSH_BLOCK_LINES[1], "# edited")).unwrap();
        let edited = status(&home, Shell::Zsh);
        fs::write(&stub_paths(&home, Shell::Zsh)[0], "# not ours\n").unwrap();
        let foreign = status(&home, Shell::Zsh);

        // Assert
        assert_eq!(before, Status::Missing);
        assert_eq!(after, Status::Ok);
        assert_eq!(twice, Status::BlockDuplicated);
        assert_eq!(edited, Status::BlockEdited);
        assert_eq!(foreign, Status::StubNotOurs);
    }

    #[test]
    fn strip_removes_only_an_exact_block_after_a_backup() {
        // Arrange
        let home = home("completion-strip");
        let rc = zshrc(&home);
        fs::write(&rc, USER_LINES).unwrap();
        ensure(&home, Shell::Zsh).unwrap();

        // Act
        let dry = strip(&home, effective_uid(), STAMP, true);
        let done = strip(&home, effective_uid(), STAMP, false);
        let again = strip(&home, effective_uid(), STAMP, false);

        // Assert
        assert_eq!(dry.len(), 1);
        assert_eq!(read(&rc), USER_LINES);
        assert_eq!(
            done[0].backup.as_deref().map(|p| read(p).contains(MARKER)),
            Some(true)
        );
        assert!(again.is_empty());
    }

    #[test]
    fn strip_leaves_an_rc_owned_by_another_user_alone() {
        // Arrange
        let home = home("completion-strip-foreign");
        let rc = zshrc(&home);
        fs::write(&rc, USER_LINES).unwrap();
        ensure(&home, Shell::Zsh).unwrap();
        let before = read(&rc);
        let other = effective_uid() + 1;

        // Act
        let out = strip(&home, other, STAMP, false);

        // Assert
        assert_eq!(read(&rc), before);
        assert!(backups(&home).is_empty(), "{:?}", backups(&home));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].error.as_deref(), Some(SKIP_OTHER_OWNER));
        assert_eq!(out[0].backup, None);
    }

    #[test]
    fn status_of_an_unknown_shell_is_missing() {
        // Arrange
        let home = home("completion-status-other");

        // Act
        let found = status(&home, Shell::Other);

        // Assert
        assert_eq!(found, Status::Missing);
    }

    #[test]
    fn the_zsh_stub_hands_a_foreign_pb_to_the_default_completer() {
        // Arrange
        let text = stub_text(Shell::Zsh);

        // Act
        let first_line = text.lines().next();
        let guard = text.find("$service == pb").expect("service guard");
        let default = text.find("_default \"$@\"").expect("default completer");
        let load = text.find("completions zsh").expect("script load");

        // Assert
        assert_eq!(first_line, Some("#compdef playbook pb"));
        assert!(text.contains("! ${commands[pb]} -ef ${commands[playbook]}"));
        assert!(guard < default && default < load, "{text}");
    }

    #[test]
    fn the_fish_stub_sources_only_a_successful_non_empty_script() {
        // Arrange
        let text = stub_text(Shell::Fish);

        // Act
        let capture = text.find("set -l script (playbook completions fish 2>/dev/null)");
        let check = text.find("test $status -eq 0; and test -n \"$script\"");
        let source = text.find("printf '%s\\n' $script | source");

        // Assert
        assert!(
            capture.is_some() && check.is_some() && source.is_some(),
            "{text}"
        );
        assert!(capture < check && check < source, "{text}");
        assert!(!text.contains("fish 2>/dev/null | source"), "{text}");
    }

    #[test]
    fn strip_keeps_an_edited_block() {
        // Arrange
        let home = home("completion-strip-edited");
        let edited = format!("{MARKER}\n# my own fpath\n");
        fs::write(zshrc(&home), &edited).unwrap();

        // Act
        let out = strip(&home, effective_uid(), STAMP, false);

        // Assert
        assert!(out.is_empty());
        assert_eq!(read(zshrc(&home)), edited);
    }
}
