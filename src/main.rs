// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

use clap::Parser;
use playbook::common::payload::Payload;
use playbook::init::run::{InitPaths, StepStatus};
use playbook::init::shim::ShellKind;
use playbook::{
    agents, cc, common, gate, hooks, init, manifest, settings, AgentsCommand, CcCommand, Cli,
    Command, GateCommand, ManifestCommand, MemoryCommand, SettingsCommand,
};
use std::io::{IsTerminal, Read};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Hook { name } => {
            let raw = read_hook_input();
            let payload = Payload::parse(&raw);
            hooks::dispatch(name, &payload);
        }
        // Launcher subcommands land in a later Work Unit; stub for now.
        Command::Cc { sub } => match sub {
            Some(CcCommand::Prune) => cc::retention::prune(&cc::logical_cwd()),
            Some(CcCommand::BustCache) => cc::bust_cache::bust(),
            Some(CcCommand::List) => {
                let cwd = cc::logical_cwd();
                let dir = cc::sessions::project_dir(&cc::claude_dir(), &cwd);
                print!("{}", cc::sessions::render_list(&dir, &cwd));
            }
            Some(CcCommand::Worktree { branch, env_base }) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                let code = cc::worktree_run::run(&cwd, &branch, env_base.as_deref());
                std::process::exit(code);
            }
            // The rest of the launcher lands in later Work Units; until then
            // they are no-ops so the shell dispatcher stays authoritative.
            _ => {}
        },
        // RESERVED, not planned. No ADR 0007 Work Unit ports this: `statusline.sh`
        // stays a shell script and WU-9 only places it where `settings.json`
        // points. Do not read this arm as work in flight; see the blueprint's
        // third 2026-08-17 amendment.
        Command::Statusline => {}
        // Retiring `hooks/hooks.json` and regenerating the seed into
        // binary-invoked form are the other two thirds of WU-11's atomic
        // switchover and stay untouched here; see `src/init/mod.rs`'s doc
        // comment for why running this wiring alone is still safe.
        Command::Init {
            system_prompt,
            aliases,
        } => {
            let home = common::home_dir();
            let claude_home = home.join(".claude");
            let self_root = init::self_root::resolve(
                std::env::var("CLAUDE_PLUGIN_ROOT").ok().as_deref(),
                &claude_home,
            );
            let shell_kind = std::env::var("SHELL")
                .ok()
                .and_then(|shell| ShellKind::detect(&shell));

            let paths = InitPaths {
                self_root,
                claude_home,
                home,
                shell_kind,
                system_prompt,
                aliases,
            };
            let outcome = init::run::run(&paths);
            for step in &outcome.steps {
                println!("{}", step.render());
            }
            if !outcome.ok() {
                let failed = outcome
                    .steps
                    .iter()
                    .filter(|s| s.status == StepStatus::Failed)
                    .count();
                eprintln!("init: {failed} step(s) failed; see above");
                std::process::exit(1);
            }
        }
        Command::Settings { sub } => match sub {
            SettingsCommand::Gen { src, perms } => match settings::gen::generate(&src, &perms) {
                Ok(output) => print!("{output}"),
                Err(err) => {
                    eprintln!("gen-shared-settings: {err}");
                    std::process::exit(2);
                }
            },
            // Exit 1, not 2: the python original used it and CI keys on it.
            SettingsCommand::Check {
                template,
                perms,
                repo_root,
            } => match settings::check::check(&template, &perms, &repo_root) {
                Ok(msg) => println!("{msg}"),
                Err(err) => {
                    eprintln!("check-shared-settings: {err}");
                    std::process::exit(1);
                }
            },
        },
        Command::Memory { sub } => match sub {
            MemoryCommand::Rebuild => {
                hooks::rebuild_memory_graph::rebuild_now();
                println!("memory: memory.graph.json rebuilt");
            }
        },
        Command::Manifest { sub } => match sub {
            // Exit 1, not 2: the shell original used it and both CI lanes key
            // on it, matching the settings check convention above.
            ManifestCommand::Check { repo_root } => {
                let root = match repo_root.or_else(manifest::check::toplevel) {
                    Some(root) => root,
                    None => {
                        // The shell's own wording when it had neither
                        // (check-manifest.sh:22).
                        eprintln!(
                            "check-manifest: not inside a git repository and no REPO_ROOT argument given"
                        );
                        std::process::exit(1);
                    }
                };
                match manifest::check::check(&root) {
                    Ok(msg) => println!("{msg}"),
                    Err(err) => {
                        eprintln!("check-manifest: {err}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Command::Agents { sub } => match sub {
            // Exit 1, not 2: the shell original used it and CI keys on it,
            // matching the manifest and settings checks' convention above.
            AgentsCommand::Check { agents_dir } => {
                let dir = match agents_dir.or_else(agents::check::default_dir) {
                    Some(dir) => dir,
                    None => {
                        eprintln!(
                            "check-agents: not inside a git repository and no AGENTS_DIR argument given"
                        );
                        std::process::exit(1);
                    }
                };
                match agents::check::check(&dir) {
                    Ok(msg) => println!("{msg}"),
                    Err(err) => {
                        eprintln!("check-agents: {err}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Command::Gate { sub } => match sub {
            GateCommand::Record {
                plan_slug,
                command,
                phase,
                input,
            } => match gate::record::run(&plan_slug, &command, &phase, &input) {
                Ok(()) => println!("gate record: recorded {phase} verdict for {plan_slug}"),
                Err(err) => {
                    eprintln!("gate record: {err}");
                    std::process::exit(1);
                }
            },
            GateCommand::Check {
                plan_slug,
                command,
                phases,
            } => match gate::check::run(&plan_slug, &command, &phases) {
                Ok(output) => println!("{output}"),
                Err(err) => {
                    eprintln!("gate check: {err}");
                    std::process::exit(1);
                }
            },
        },
        Command::Path { kind } => match common::repo_scoped_dir(common::RepoScope::Worktree) {
            Some(base) => println!("{}", base.join(kind.dir_name()).display()),
            None => {
                eprintln!(
                    "playbook path: could not resolve a worktree-scoped storage location; \
                     this repo needs a git 'origin' remote and a resolvable worktree \
                     toplevel, refusing to fall back to a repo-local path"
                );
                std::process::exit(1);
            }
        },
    }
}

/// Read the hook payload the same way common.py does: `HOOK_INPUT` env var
/// if set and non-empty, else all of stdin when stdin is not a tty, else
/// empty. Never panics; a read failure yields an empty payload.
fn read_hook_input() -> String {
    if let Ok(value) = std::env::var("HOOK_INPUT") {
        if !value.is_empty() {
            return value;
        }
    }
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    buf
}
