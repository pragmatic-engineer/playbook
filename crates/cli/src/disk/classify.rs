// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Which Bash commands write a lot to disk (ADR 0047 decision 3). The
//! disk-guard hook denies them when free space is below `disk.blockFreeGb`.
//!
//! Only a word in command position counts, read with the shared shell
//! lexer, so a build named in a commit message or an `echo` is data. Wrappers
//! (`env`, `sudo`, `timeout`), runners (`nohup`, `xargs`) and `sh -c` are
//! looked through. Everything not listed here is allowed, which keeps reads,
//! `rm`, `cargo clean`, `git worktree remove`, `df`, `du` and every
//! `playbook` command open for recovery.

use crate::common::shell::{
    self, program_index, program_name, runner_value_options, shell_script_at, skip_options, SHELLS,
};

const MAX_DEPTH: usize = 3;

/// The first build-class command in `command`, as a short label such as
/// `cargo build`, or `None` when it has none.
pub fn build_command(command: &str) -> Option<String> {
    build_in(command, 0)
}

fn build_in(command: &str, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    shell::simple_commands(command)
        .iter()
        .find_map(|words| build_in_words(words, depth))
}

fn build_in_words(words: &[String], depth: usize) -> Option<String> {
    let at = program_index(words)?;
    let program = program_name(&words[at]);
    let args = &words[at + 1..];
    if let Some(value_options) = runner_value_options(&program) {
        let next = skip_options(args, 0, value_options);
        return build_in_words(&args[next..], depth);
    }
    if SHELLS.contains(&program.as_str()) {
        let script = shell_script_at(args)?;
        return build_in(&args[script], depth + 1);
    }
    if program == "eval" {
        return build_in(&args.join(" "), depth + 1);
    }
    classify(&program, args)
}

/// The first argument that is not an option, from `from`.
fn first_word(args: &[String], from: usize, takes_value: &[&str]) -> Option<(usize, String)> {
    let at = skip_options(args, from, takes_value);
    args.get(at).map(|w| (at, w.to_ascii_lowercase()))
}

fn label(program: &str, sub: &str) -> Option<String> {
    Some(format!("{program} {sub}"))
}

/// Whether `args` only ask a tool about itself, which writes nothing.
fn info_only(args: &[String]) -> bool {
    args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--version" | "-v" | "-V" | "--help" | "-h" | "-n" | "--dry-run" | "--just-print"
        )
    })
}

fn classify(program: &str, args: &[String]) -> Option<String> {
    match program {
        "cargo" => {
            let from = usize::from(args.first().is_some_and(|w| w.starts_with('+')));
            let (_, sub) = first_word(args, from, &["-C", "-Z", "--config", "--color"])?;
            let builds = [
                "build", "b", "test", "t", "check", "c", "clippy", "nextest", "run", "r", "bench",
                "doc", "d", "install", "rustc", "fix",
            ];
            builds
                .contains(&sub.as_str())
                .then(|| format!("cargo {sub}"))
        }
        "rustc" => {
            let info = args.iter().any(|a| {
                matches!(a.as_str(), "--version" | "-V" | "-vV" | "--help" | "-h")
                    || a.starts_with("--print")
            });
            (!info).then(|| "rustc".to_string())
        }
        "npm" | "pnpm" | "yarn" => js(program, args),
        "pip" | "pip3" => pip(program, args),
        "python" | "python3" => {
            let module = args.iter().position(|a| a == "-m")?;
            let rest = &args[module + 1..];
            let name = rest.first()?;
            (name == "pip" || name == "pip3")
                .then(|| pip(name, &rest[1..]))
                .flatten()
        }
        "uv" => {
            let (at, sub) = first_word(args, 0, &[])?;
            match sub.as_str() {
                "pip" => pip("uv pip", &args[at + 1..]),
                "sync" | "build" => label("uv", &sub),
                _ => None,
            }
        }
        "docker" | "podman" => {
            let (at, sub) = first_word(args, 0, &["-H", "--host", "--context", "-c"])?;
            match sub.as_str() {
                "build" => label(program, "build"),
                "buildx" | "image" | "compose" => {
                    let (_, next) = first_word(args, at + 1, &["-f", "--file", "-p"])?;
                    (next == "build").then(|| format!("{program} {sub} build"))
                }
                _ => None,
            }
        }
        "brew" => {
            let (_, sub) = first_word(args, 0, &[])?;
            ["install", "upgrade", "reinstall"]
                .contains(&sub.as_str())
                .then(|| format!("brew {sub}"))
        }
        "git" => {
            let (at, sub) = first_word(args, 0, &["-C", "-c", "--git-dir", "--work-tree"])?;
            match sub.as_str() {
                "clone" => label("git", "clone"),
                "worktree" => {
                    let (_, next) = first_word(args, at + 1, &[])?;
                    (next == "add").then(|| "git worktree add".to_string())
                }
                _ => None,
            }
        }
        "make" | "gmake" => {
            if info_only(args) {
                return None;
            }
            let targets: Vec<&String> = {
                let mut found = Vec::new();
                let mut i = 0;
                while i < args.len() {
                    let a = &args[i];
                    // `-j` takes a value only when a number follows.
                    let numeric = |w: Option<&String>| {
                        w.is_some_and(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()))
                    };
                    let takes_value = ["-C", "-f", "-o", "-W", "-I"].contains(&a.as_str())
                        || (a == "-j" && numeric(args.get(i + 1)));
                    if takes_value {
                        i += 2;
                        continue;
                    }
                    if !a.starts_with('-') && !a.contains('=') {
                        found.push(a);
                    }
                    i += 1;
                }
                found
            };
            let only_clean = !targets.is_empty()
                && targets
                    .iter()
                    .all(|t| matches!(t.as_str(), "clean" | "distclean" | "mrproper" | "help"));
            (!only_clean).then(|| program.to_string())
        }
        "just" => {
            // Bare `just` runs the default recipe, which usually lists the
            // others. A named recipe can run anything, so it counts as a
            // build unless it only cleans.
            let lists = |a: &String| {
                matches!(
                    a.as_str(),
                    "--list" | "-l" | "--summary" | "--show" | "-s" | "--evaluate"
                )
            };
            if info_only(args) || args.iter().any(lists) {
                return None;
            }
            let value_options = ["-f", "--justfile", "-d", "--working-directory", "--set"];
            let (_, recipe) = first_word(args, 0, &value_options)?;
            (!matches!(recipe.as_str(), "clean" | "help" | "default"))
                .then(|| format!("just {recipe}"))
        }
        "go" => {
            let (_, sub) = first_word(args, 0, &[])?;
            ["build", "test", "install"]
                .contains(&sub.as_str())
                .then(|| format!("go {sub}"))
        }
        _ => None,
    }
}

fn js(program: &str, args: &[String]) -> Option<String> {
    let Some((at, sub)) = first_word(
        args,
        0,
        &["--prefix", "-C", "--dir", "--filter", "-F", "--cwd"],
    ) else {
        // Bare `yarn` installs; `yarn --version` only asks.
        return (program == "yarn" && !info_only(args)).then(|| "yarn".to_string());
    };
    let installs = [
        "install", "i", "ci", "add", "build", "rebuild", "update", "up",
    ];
    if installs.contains(&sub.as_str()) {
        return Some(format!("{program} {sub}"));
    }
    if sub == "run" || sub == "run-script" {
        let (_, script) = first_word(args, at + 1, &[])?;
        return script
            .contains("build")
            .then(|| format!("{program} run {script}"));
    }
    None
}

fn pip(program: &str, args: &[String]) -> Option<String> {
    let (_, sub) = first_word(args, 0, &[])?;
    (sub == "install").then(|| format!("{program} install"))
}

#[cfg(test)]
mod tests {
    use super::build_command;

    /// Commands that write a lot and are denied under the floor.
    const BUILDS: &[(&str, &str)] = &[
        ("cargo build", "cargo build"),
        ("cargo build --release -p playbook", "cargo build"),
        ("cargo b", "cargo b"),
        ("cargo test -p playbook --lib disk::", "cargo test"),
        ("cargo t", "cargo t"),
        ("cargo check --all-targets", "cargo check"),
        ("cargo clippy --all-targets -- -D warnings", "cargo clippy"),
        ("cargo nextest run --workspace", "cargo nextest"),
        ("cargo run -- disk status", "cargo run"),
        ("cargo +nightly build", "cargo build"),
        ("cargo -Z unstable-options build", "cargo build"),
        ("cargo install cargo-nextest --locked", "cargo install"),
        ("cargo bench", "cargo bench"),
        ("cargo doc --no-deps", "cargo doc"),
        ("CARGO_TARGET_DIR=/tmp/t cargo build", "cargo build"),
        ("env CARGO_INCREMENTAL=0 cargo test", "cargo test"),
        ("cd crates/cli && cargo test", "cargo test"),
        ("cargo fmt --all && cargo clippy -q", "cargo clippy"),
        ("timeout 600 cargo test", "cargo test"),
        ("nohup cargo build &", "cargo build"),
        ("sudo make install", "make"),
        ("bash -c 'cargo build'", "cargo build"),
        ("sh -lc \"cd x && cargo check\"", "cargo check"),
        ("eval cargo build", "cargo build"),
        ("rustc src/main.rs", "rustc"),
        ("npm install", "npm install"),
        ("npm i", "npm i"),
        ("npm ci", "npm ci"),
        ("npm run build", "npm run build"),
        ("npm --prefix web run build:prod", "npm run build:prod"),
        ("pnpm install --frozen-lockfile", "pnpm install"),
        ("pnpm add react", "pnpm add"),
        ("pnpm build", "pnpm build"),
        ("yarn", "yarn"),
        ("yarn install", "yarn install"),
        ("yarn build", "yarn build"),
        ("pip install -r requirements.txt", "pip install"),
        ("pip3 install requests", "pip3 install"),
        ("python -m pip install -e .", "pip install"),
        ("python3 -m pip install x", "pip install"),
        ("uv pip install x", "uv pip install"),
        ("docker build -t app .", "docker build"),
        ("docker buildx build .", "docker buildx build"),
        ("docker compose build", "docker compose build"),
        ("brew install ripgrep", "brew install"),
        ("brew upgrade", "brew upgrade"),
        ("git clone https://github.com/a/b", "git clone"),
        ("git -C repo worktree add ../wt", "git worktree add"),
        (
            "git worktree add -b feat ../.worktrees/feat",
            "git worktree add",
        ),
        ("make", "make"),
        ("make -j8 all", "make"),
        ("go build ./...", "go build"),
        ("just build", "just build"),
        ("just test -p playbook", "just test"),
        ("make -j clean all", "make"),
        ("go test ./...", "go test"),
    ];

    /// Commands that stay allowed at any free space, so the model can
    /// recover.
    const ALLOWED: &[&str] = &[
        "rm -rf /Users/me/code/.worktrees/cargo-old",
        "rm -rf target",
        "cargo clean",
        "cargo clean -p playbook",
        "CARGO_TARGET_DIR=/w/.worktrees/cargo-a cargo clean",
        "cargo fmt --all",
        "cargo metadata --format-version 1",
        "cargo tree",
        "cargo --version",
        "rustc --version",
        "rustc --print sysroot",
        "playbook disk clean",
        "playbook disk clean --dry-run",
        "playbook disk status",
        "playbook disk release",
        "playbook worktree sweep",
        "playbook deliver --dir x",
        "df -g /Users/me",
        "df -h",
        "du -sh .worktrees/*",
        "ls -la",
        "cat Cargo.toml",
        "git status",
        "git diff --stat",
        "git log --oneline -5",
        "git worktree list",
        "git worktree remove ../wt",
        "git worktree prune",
        "git branch -d feat",
        "git push",
        "git commit -m 'run cargo build after npm install'",
        "echo cargo build",
        "grep -rn 'cargo test' docs",
        "gh pr view 1",
        "npm test",
        "npm run lint",
        "npm --version",
        "pip list",
        "pip uninstall x",
        "docker ps",
        "docker image prune -f",
        "docker system df",
        "brew list",
        "brew cleanup",
        "make clean",
        "make --version",
        "make -n all",
        "make -j clean",
        "yarn --version",
        "just",
        "just --list",
        "just clean",
        "make distclean",
        "go vet ./...",
        "go clean -cache",
        "find /tmp -name 'pb-*' -mmin +60",
        "cat <<EOF\ncargo build\nEOF",
        "sccache --show-stats",
        "tmutil listlocalsnapshots /",
    ];

    #[test]
    fn build_commands_are_found() {
        for (command, want) in BUILDS {
            assert_eq!(build_command(command).as_deref(), Some(*want), "{command}");
        }
    }

    #[test]
    fn recovery_and_read_commands_stay_allowed() {
        for command in ALLOWED {
            assert_eq!(build_command(command), None, "{command}");
        }
    }

    #[test]
    fn the_table_covers_at_least_sixty_commands() {
        assert!(BUILDS.len() + ALLOWED.len() >= 60);
    }
}
