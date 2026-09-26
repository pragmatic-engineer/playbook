// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

use clap::Parser;
use playbook::common::payload::Payload;
use playbook::init::run::{InitPaths, StepStatus};
use playbook::init::shim::ShellKind;
use playbook::{
    agents, cc, common, config, doctor, gate, hooks, init, manifest, settings, worktree,
    AgentsCommand, CcCommand, Cli, Command, ConfigCommand, DoctorCommand, GateCommand,
    ManifestCommand, MemoryCommand, SettingsCommand, WorktreeCommand,
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
            MemoryCommand::Rebuild => match hooks::rebuild_memory_graph::rebuild_now() {
                Ok(()) => println!("memory: memory.graph.json rebuilt"),
                Err(err) => {
                    eprintln!("memory rebuild: {err}");
                    std::process::exit(1);
                }
            },
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
        Command::Config { sub } => {
            let home = common::home_dir();
            let slug = common::repo_slug();
            let repo_slug = if slug.is_empty() {
                None
            } else {
                Some(slug.as_str())
            };
            match sub {
                ConfigCommand::Get { key } => match config::resolve(&key, &home, repo_slug) {
                    Ok((value, source)) => {
                        println!(
                            "{key}: {} (source: {})",
                            format_config_value(&value),
                            source_label(source)
                        );
                    }
                    Err(err) => {
                        eprintln!("config get: {err}");
                        std::process::exit(1);
                    }
                },
                ConfigCommand::Set {
                    key,
                    value,
                    org,
                    global,
                } => {
                    if org && global {
                        eprintln!("config set: --org and --global are mutually exclusive");
                        std::process::exit(1);
                    }
                    let tier = if global {
                        config::write::Tier::Global
                    } else if org {
                        config::write::Tier::Org
                    } else {
                        config::write::Tier::Repo
                    };
                    let value_json = match parse_config_value(&key, &value) {
                        Ok(value_json) => value_json,
                        Err(err) => {
                            eprintln!("config set: {err}");
                            std::process::exit(1);
                        }
                    };
                    match config::write::set(tier, &key, value_json, &home, repo_slug) {
                        Ok(()) => println!("config set: {key} updated"),
                        Err(err) => {
                            eprintln!("config set: {err}");
                            std::process::exit(1);
                        }
                    }
                }
                ConfigCommand::List => {
                    for &key in config::keys::KNOWN_KEYS {
                        match config::resolve(key, &home, repo_slug) {
                            Ok((value, source)) => {
                                println!(
                                    "{key}: {} (source: {})",
                                    format_config_value(&value),
                                    source_label(source)
                                );
                            }
                            Err(err) => {
                                eprintln!("config list: {err}");
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
        }
        Command::Path { kind } => match common::repo_scoped_dir(common::RepoScope::Worktree) {
            Some(base) => {
                if let Some(repo_root) = playbook::manifest::check::toplevel() {
                    if let Err(err) =
                        playbook::gate::db::migrate_legacy_repo_local(&repo_root, &base)
                    {
                        eprintln!("playbook path: {err}");
                        std::process::exit(1);
                    }
                }
                println!("{}", base.join(kind.dir_name()).display());
            }
            None => {
                eprintln!(
                    "playbook path: could not resolve a worktree-scoped storage location; \
                     this repo needs a git 'origin' remote and a resolvable worktree \
                     toplevel, refusing to fall back to a repo-local path"
                );
                std::process::exit(1);
            }
        },
        Command::Worktree { sub } => {
            let home = common::home_dir();
            let slug = common::repo_slug();
            let repo_slug = if slug.is_empty() {
                None
            } else {
                Some(slug.as_str())
            };
            let repo_root = std::env::current_dir().unwrap_or_default();
            match sub {
                WorktreeCommand::Sweep { dry_run } => {
                    let now_epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    match worktree::sweep(&repo_root, &home, repo_slug, dry_run, now_epoch) {
                        Ok(report) => {
                            for line in report {
                                println!("{line}");
                            }
                        }
                        Err(err) => {
                            eprintln!("worktree sweep: {err}");
                            std::process::exit(1);
                        }
                    }
                }
                // `remove` lands in a later Segment; a bare success here
                // would let a caller wire it up believing it already works.
                WorktreeCommand::Remove { path: _ } => {
                    eprintln!("worktree remove: not implemented yet");
                    std::process::exit(1);
                }
            }
        }
        Command::Doctor { sub } => match sub {
            DoctorCommand::PluginVersion { path } => {
                println!("{}", doctor::field::plugin_version(&path));
            }
            DoctorCommand::StatuslineCommand { path } => {
                println!("{}", doctor::field::statusline_command(&path));
            }
            DoctorCommand::HookCommands { path } => {
                for command in doctor::field::hook_commands(&path) {
                    println!("{command}");
                }
            }
        },
    }
}

/// Lowercase tier label for `config get`/`config list` output, e.g. "repo"
/// rather than the raw `Source::Repo` debug casing.
fn source_label(source: config::Source) -> &'static str {
    match source {
        config::Source::Repo => "repo",
        config::Source::Org => "org",
        config::Source::Global => "global",
        config::Source::Default => "default",
    }
}

/// Render a resolved config value the way a shell script or a human reading
/// `playbook config get`/`list` output expects: a bare `deep`, not the
/// JSON-quoted `"deep"` a raw `Value`'s `Display` impl would print.
fn format_config_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Parse `config set`'s raw `value` string into the JSON type `key` expects,
/// using `config::keys::default_value` to learn that type generically so an
/// unknown key or a wrong-type value is rejected before `write::set` ever
/// runs and touches a file. A boolean key accepts `true`/`false`
/// case-insensitively; a numeric key accepts a non-negative integer; any
/// other known key is passed through as a string, leaving an out-of-enum
/// value for `write::set`'s own check to reject.
fn parse_config_value(key: &str, value: &str) -> Result<serde_json::Value, config::ConfigError> {
    let default = config::keys::default_value(key)
        .ok_or_else(|| config::ConfigError::UnknownKey(key.to_string()))?;
    match default {
        serde_json::Value::Bool(_) => match value.to_ascii_lowercase().as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(config::ConfigError::WrongType {
                key: key.to_string(),
                expected: "boolean",
            }),
        },
        serde_json::Value::Number(_) => match value.parse::<u64>() {
            Ok(parsed) => Ok(serde_json::Value::Number(parsed.into())),
            Err(_) => Err(config::ConfigError::InvalidNumber {
                key: key.to_string(),
                value: value.to_string(),
            }),
        },
        _ => Ok(serde_json::Value::String(value.to_string())),
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
