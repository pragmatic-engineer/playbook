// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh` places the binary and one PATH marker per shell, hands the
//! right flags to `playbook init`, and copes with a release binary older than
//! the script. Removal is `playbook uninstall`, covered by tests/uninstall.rs.
//! The script is sourced from `git archive HEAD` so untracked build output
//! cannot leak in.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{archive_head, both, install_stubs, repo_root, sha256_hex, write_exec, Work};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASE_BODY: &str = r#"{"tag_name": "v1.2.3"}"#;
const ASSET: &str = "playbook-1.2.3-x86_64-unknown-linux-musl";
const GOOD_ASSET: &str = "#!/usr/bin/env bash\necho \"playbook 1.2.3\"\n";
const MARKER: &str = "# playbook binary";

/// Install through the stubbed network path (`--no-setup --skip-plugin`). The
/// stub serves no source tarball, so the script dies after the binary and PATH
/// work, which is all these scenarios look at.
fn binary_install(work: &Work, stubs: &Path, home: &Path, bindir: &Path, shell: &str) {
    let sums = format!("{}  {ASSET}", sha256_hex(GOOD_ASSET.as_bytes()));
    let path = format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _ = work;
    let _ = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--no-setup", "--skip-plugin"])
        .env("PATH", path)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("HOME", home)
        .env("PLAYBOOK_BIN_DIR", bindir)
        .env("SHELL", shell)
        .env("STUB_CODE", "200")
        .env("STUB_BODY", RELEASE_BODY)
        .env("STUB_ASSET_BODY", GOOD_ASSET)
        .env("STUB_SUMS_BODY", sums)
        .output();
}

fn markers(file: &Path) -> usize {
    fs::read_to_string(file)
        .map(|t| t.matches(MARKER).count())
        .unwrap_or(0)
}

struct Net {
    work: Work,
    stubs: PathBuf,
}

fn net(tag: &str) -> Net {
    let work = Work::new(tag);
    let stubs = work.dir("stubs");
    install_stubs(&stubs);
    Net { work, stubs }
}

#[test]
fn install_places_the_binary_and_exactly_one_path_marker() {
    let n = net("bp-one");
    let home = n.work.dir("home");
    let bindir = n.work.path("bindir");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/bin/bash");
    assert!(bindir.join("playbook").exists());
    assert_eq!(markers(&home.join(".bashrc")), 1);
}

#[test]
fn installing_twice_leaves_exactly_one_path_marker() {
    let n = net("bp-twice");
    let home = n.work.dir("home");
    let bindir = n.work.path("bindir");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/bin/bash");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/bin/bash");
    assert_eq!(markers(&home.join(".bashrc")), 1);
}

#[test]
fn zsh_gets_one_path_block_in_zshenv_and_nothing_in_zshrc() {
    let n = net("bp-zsh");
    let home = n.work.dir("home");
    let bindir = n.work.path("zsh-dir");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/bin/zsh");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/bin/zsh");
    let zshenv = fs::read_to_string(home.join(".zshenv")).unwrap_or_default();
    assert_eq!(markers(&home.join(".zshenv")), 1, "{zshenv}");
    assert!(!home.join(".zshrc").exists());
    assert!(
        zshenv.contains(&format!("export PATH=\"{}:$PATH\"", bindir.display())),
        "{zshenv}"
    );
}

#[test]
fn the_bash_path_block_goes_into_the_existing_login_file() {
    let n = net("bp-login");
    let home = n.work.dir("home");
    fs::write(home.join(".profile"), "# mine\n").unwrap();
    binary_install(
        &n.work,
        &n.stubs,
        &home,
        &n.work.path("login-dir"),
        "/bin/bash",
    );
    assert_eq!(markers(&home.join(".profile")), 1);
    assert!(
        !home.join(".bash_profile").exists(),
        "a shadowing login file was created"
    );
}

#[test]
fn fish_gets_a_conf_d_file_with_fish_add_path() {
    let n = net("bp-fish");
    let home = n.work.dir("home");
    let bindir = n.work.path("fish-dir");
    binary_install(&n.work, &n.stubs, &home, &bindir, "/usr/bin/fish");
    let file =
        fs::read_to_string(home.join(".config/fish/conf.d/playbook.fish")).unwrap_or_default();
    assert!(
        file.contains(&format!("fish_add_path -g \"{}\"", bindir.display())),
        "{file}"
    );
}

fn source_tree(work: &Work) -> Option<PathBuf> {
    let src = work.path("src");
    archive_head(&src)?;
    Some(src)
}

/// Run the installer from the archived tree with a stub `playbook` in the
/// bin dir (no network path).
fn local_install(
    work: &Work,
    src: &Path,
    stub_dir: &Path,
    flags: &[&str],
    tag: &str,
) -> (i32, PathBuf, String) {
    let home = work.dir(&format!("home-{tag}"));
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(flags)
        .env("PLAYBOOK_SRC", src)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("HOME", &home)
        .env("PLAYBOOK_BIN_DIR", stub_dir)
        .env("SHELL", "/bin/bash")
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), home, both(&out))
}

#[test]
fn install_succeeds_against_a_release_binary_that_predates_an_optional_init_flag() {
    let work = Work::new("bp-strict");
    let Some(src) = source_tree(&work) else {
        return;
    };
    let stub = work.dir("strict-init-bin");
    write_exec(
        &stub.join("playbook"),
        r#"case "${1:-}" in
  --version) printf 'playbook 0.10.0\n'; exit 0 ;;
  init)
    shift
    if [ "${1:-}" = "--help" ]; then
      printf 'Install or repair the local Claude Code configuration\n\nUsage: playbook init\n'
      exit 0
    fi
    if [ $# -gt 0 ]; then
      printf "error: unexpected argument '%s' found\n\nUsage: playbook init\n" "$1" >&2
      exit 2
    fi
    home="${CLAUDE_HOME:-$HOME/.claude}"
    mkdir -p "$home"
    [ -f "$home/settings.json" ] || printf '{}\n' > "$home/settings.json"
    [ -f "$home/.settings.base.json" ] || printf '{}\n' > "$home/.settings.base.json"
    printf 'settings: wired - seeded from template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: skipped - $SHELL is neither bash nor zsh\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: skipped - not installed; pass --system-prompt to opt in\n'
    exit 0
    ;;
esac
exit 0
"#,
    );
    let (rc, home, out) = local_install(&work, &src, &stub, &["--yes", "--skip-plugin"], "strict");
    assert_eq!(rc, 0, "{out}");
    assert!(home.join(".claude/settings.json").exists(), "{out}");
}

fn recording_stub(work: &Work) -> (PathBuf, PathBuf) {
    let dir = work.dir("recording-init-bin");
    let argv = work.path("init-argv.txt");
    write_exec(
        &dir.join("playbook"),
        &format!(
            r#"case "${{1:-}}" in
  --version) printf 'playbook 0.0.0-stub\n' ;;
  init)
    shift
    printf '%s\n' "$@" > "{}"
    if [ "${{1:-}}" = "--help" ]; then
      printf -- '--aliases\n--system-prompt\n'
      exit 0
    fi
    home="${{CLAUDE_HOME:-$HOME/.claude}}"
    [ -f "$home/settings.json" ] || printf '{{}}\n' > "$home/settings.json"
    [ -f "$home/.settings.base.json" ] || printf '{{}}\n' > "$home/.settings.base.json"
    printf 'settings: wired - seeded from template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: ok - installed\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: ok - installed\n'
    ;;
esac
exit 0
"#,
            argv.display()
        ),
    );
    (dir, argv)
}

#[test]
fn aliases_and_system_prompt_flags_are_forwarded_to_init() {
    let work = Work::new("bp-forward");
    let Some(src) = source_tree(&work) else {
        return;
    };
    let (stub, argv) = recording_stub(&work);
    local_install(
        &work,
        &src,
        &stub,
        &["--yes", "--aliases", "--system-prompt", "--skip-plugin"],
        "fwd",
    );
    let got = fs::read_to_string(&argv).unwrap_or_default();
    assert!(got.contains("--aliases"), "init received: '{got}'");
    assert!(got.contains("--system-prompt"), "init received: '{got}'");
}

#[test]
fn binary_only_does_not_opt_in_to_the_launcher_or_system_prompt() {
    let work = Work::new("bp-binonly");
    let Some(src) = source_tree(&work) else {
        return;
    };
    let (stub, argv) = recording_stub(&work);
    local_install(&work, &src, &stub, &["--yes", "--binary-only"], "bo");
    let got = fs::read_to_string(&argv).unwrap_or_default();
    assert!(
        !got.contains("--aliases") && !got.contains("--system-prompt"),
        "init received: '{got}'"
    );
}
