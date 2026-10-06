// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

use clap::Parser;
use playbook::common::payload::Payload;
use playbook::init::run::{InitPaths, StepStatus};
use playbook::init::shim::ShellKind;
use playbook::{
    agents, cc, check, ci, common, config, doctor, gate, handoff, hooks, init, json, manifest,
    mode, pr, settings, trust, usage, worktree, AgentsCommand, CcCommand, CheckCommand, Cli,
    Command, ConfigCommand, DashboardCommand, DoctorCommand, GateCommand, HandoffCommand,
    JsonCommand, ManifestCommand, MemoryCommand, ModeArg, ModeCommand, PrCommand, SettingsCommand,
    UsageCommand, WorktreeCommand,
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
        Command::Usage { sub } => {
            let paths = usage::run::Paths::real();
            let result = match sub {
                Some(UsageCommand::Ingest) => usage::run::run_ingest(&paths),
                Some(UsageCommand::Dashboard { serve: true, .. }) => {
                    usage::dashboard::serve(&paths).map(|()| String::new())
                }
                Some(UsageCommand::Dashboard {
                    sub: Some(DashboardCommand::Stop),
                    ..
                }) => usage::dashboard::run_stop(&paths),
                Some(UsageCommand::Dashboard { .. }) => {
                    let exe = std::env::current_exe().unwrap_or_default();
                    // Test seam: lets spawned-binary tests skip launching a
                    // real browser. Never set in production.
                    if std::env::var_os("PLAYBOOK_USAGE_NO_BROWSER").is_some() {
                        usage::dashboard::run_dashboard(&paths, &exe, &usage::dashboard::no_browser)
                    } else {
                        usage::dashboard::run_dashboard(
                            &paths,
                            &exe,
                            &usage::dashboard::open_in_browser,
                        )
                    }
                }
                None => usage::run::run_summary(&paths),
            };
            match result {
                Ok(output) => println!("{}", output.trim_end()),
                Err(err) => {
                    eprintln!("usage: {err}");
                    std::process::exit(1);
                }
            }
        }
        Command::Gate { sub } => match sub {
            GateCommand::Record {
                plan_slug,
                command,
                phase,
                input,
                source,
            } => match gate::record::run(&plan_slug, &command, &phase, &input, &source) {
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
                source,
            } => match gate::check::run(&plan_slug, &command, &phases, &source) {
                Ok(output) => println!("{output}"),
                Err(err) => {
                    eprintln!("gate check: {err}");
                    std::process::exit(1);
                }
            },
        },
        Command::Check { sub } => match sub {
            CheckCommand::CommitMsg { file } => match check::commit_msg(&file) {
                Ok(problems) if problems.is_empty() => {}
                Ok(problems) => {
                    for problem in problems {
                        eprintln!("commit-msg: {problem}");
                    }
                    std::process::exit(1);
                }
                Err(err) => {
                    eprintln!("commit-msg: {err}");
                    std::process::exit(1);
                }
            },
        },
        Command::Ci { json, strict, dir } => match ci::run(dir.as_deref(), json, strict) {
            Ok((text, code)) => {
                println!("{text}");
                std::process::exit(code);
            }
            Err(err) => {
                eprintln!("ci: {err}");
                std::process::exit(1);
            }
        },
        Command::Pr { sub } => {
            let gh = pr::shared::RealGhClient;
            match sub {
                PrCommand::Prepare { base, ticket, dir } => {
                    enter_dir("pr prepare", dir.as_deref());
                    match pr::prepare::run(&gh, base.as_deref(), ticket.as_deref()) {
                        Ok(output) => println!("{output}"),
                        Err(err) => {
                            eprintln!("pr prepare: {err}");
                            std::process::exit(1);
                        }
                    }
                }
                PrCommand::Create {
                    title,
                    body_file,
                    base,
                    dir,
                } => {
                    // A relative body path is relative to where the caller ran.
                    let body_file = absolute_from_cwd(&body_file);
                    enter_dir("pr create", dir.as_deref());
                    match pr::create::run(&gh, &title, &body_file, base.as_deref()) {
                        Ok(output) => println!("{output}"),
                        Err(err) => {
                            eprintln!("pr create: {err}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Command::Config { sub } => {
            let home = common::home_dir();
            let slug = common::repo_slug();
            let repo_slug = if slug.is_empty() {
                None
            } else {
                Some(slug.as_str())
            };
            match sub {
                ConfigCommand::Get { key } => match config::resolve_valid(&key, &home, repo_slug) {
                    Ok(resolved) => print_resolved("config get", &key, resolved),
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
                        match config::resolve_valid(key, &home, repo_slug) {
                            Ok(resolved) => print_resolved("config list", key, resolved),
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
                WorktreeCommand::Remove { path } => {
                    let now_epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    match worktree::remove(&repo_root, &home, repo_slug, &path, now_epoch) {
                        Ok(line) => println!("{line}"),
                        Err(err) => {
                            eprintln!("worktree remove: {err}");
                            std::process::exit(1);
                        }
                    }
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
            DoctorCommand::HookCommandsForEvent {
                path,
                event,
                guards,
            } => {
                let guard_refs: Vec<&str> = guards.iter().map(String::as_str).collect();
                for (guard, count) in
                    doctor::field::hook_commands_for_event(&path, &event, &guard_refs)
                {
                    println!("{guard}={count}");
                }
            }
            DoctorCommand::HookCommandsMatching {
                path,
                pattern,
                event,
            } => {
                let count =
                    doctor::field::hook_commands_matching(&path, event.as_deref(), &pattern);
                println!("{count}");
            }
        },
        Command::Json { sub } => {
            let input = read_stdin_to_string();
            match sub {
                JsonCommand::BucketCounts { buckets } => {
                    let names: Vec<&str> = buckets.iter().map(String::as_str).collect();
                    for (bucket, count) in json::ghjson::bucket_counts(&input, &names) {
                        println!("{bucket}={count}");
                    }
                }
                JsonCommand::ArrayLength => {
                    println!("{}", json::ghjson::array_length(&input));
                }
                JsonCommand::Field { key } => {
                    println!("{}", json::ghjson::field(&input, &key));
                }
                JsonCommand::IsValidJson => {
                    println!("{}", json::evalfixture::is_valid_json(&input));
                }
                JsonCommand::IndexedElement { index } => {
                    println!(
                        "{}",
                        json::evalfixture::indexed_element(&input, index).unwrap_or_default()
                    );
                }
                JsonCommand::StringField { key } => {
                    println!("{}", json::evalfixture::string_field(&input, &key));
                }
                JsonCommand::LensNamesJoined => {
                    println!("{}", json::evalfixture::lens_names_joined(&input));
                }
                JsonCommand::LensNames => {
                    for name in json::evalfixture::lens_names(&input) {
                        println!("{name}");
                    }
                }
                JsonCommand::IsValidJsonCompact => {
                    println!(
                        "{}",
                        json::evalfixture::is_valid_json_compact(&input).unwrap_or_default()
                    );
                }
                JsonCommand::LensFound { lens } => {
                    println!("{}", json::evalfixture::lens_found(&input, &lens));
                }
                JsonCommand::LensTier { lens } => {
                    println!("{}", json::evalfixture::lens_tier(&input, &lens));
                }
                JsonCommand::Equal { a, b, ignore_keys } => {
                    let ignore_keys: Vec<&str> = ignore_keys.iter().map(String::as_str).collect();
                    if let Err(message) = json::jsoncmp::json_equal(&a, &b, &ignore_keys) {
                        eprintln!("{message}");
                        std::process::exit(1);
                    }
                }
                JsonCommand::RemoveKeys { keys } => {
                    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                    println!("{}", json::settingsjson::remove_keys_print(&input, &keys));
                }
                JsonCommand::KeysSorted => {
                    for key in json::keylist::top_level_keys_sorted(&input) {
                        println!("{key}");
                    }
                }
                JsonCommand::AddMarkerKey { key } => {
                    println!("{}", json::settingsjson::add_marker_key_print(&input, &key));
                }
                JsonCommand::MemoryContext { graph_file, repo } => {
                    let graph_json = std::fs::read_to_string(&graph_file).unwrap_or_default();
                    let output = json::memorycontext::render_memory_context(&graph_json, &repo);
                    if !output.is_empty() {
                        println!("{output}");
                    }
                }
                JsonCommand::SessionFields { pwd } => {
                    print!("{}", json::statusline::session_fields(&input, &pwd));
                }
                JsonCommand::GraphqlCiChecks {} => {
                    print!("{}", json::statusline::graphql_ci_checks(&input));
                }
                JsonCommand::CiRollup {} => {
                    let (state, failed, running, total) = json::statusline::ci_rollup(&input);
                    println!("{state} {failed} {running} {total}");
                }
                JsonCommand::PrFields {} => {
                    print!("{}", json::statusline::pr_fields(&input));
                }
                JsonCommand::FilterSystemLines { pattern } => {
                    print!("{}", json::jsonl::filter_system_lines(&input, &pattern));
                }
                JsonCommand::RewriteSessionId { new_sid } => {
                    print!("{}", json::jsonl::rewrite_session_id(&input, &new_sid));
                }
                JsonCommand::ValidJson => {
                    if !json::validate::is_valid_json(&input) {
                        std::process::exit(1);
                    }
                }
                JsonCommand::FieldEquals { path, expected } => {
                    if !json::fieldeq::field_equals(&input, &path, &expected) {
                        std::process::exit(1);
                    }
                }
                JsonCommand::FieldLength { field } => {
                    println!("{}", json::count::field_length(&input, &field));
                }
                JsonCommand::RawStringField { key } => {
                    print!("{}", json::fields::string_field(&input, &key));
                }
                JsonCommand::CanonicalJson { del } => {
                    println!("{}", json::canon::canonical_json(&input, del.as_deref()));
                }
                JsonCommand::HookCommandsForEvent { event } => {
                    for command in json::hookevents::hook_commands_for_event(&input, &event) {
                        println!("{command}");
                    }
                }
                JsonCommand::ProjectField {
                    project_path,
                    field,
                } => {
                    println!(
                        "{}",
                        json::claudejson::project_field(&input, &project_path, &field)
                    );
                }
            }
        }
        Command::Trust { path } => {
            let _ = trust::run(&path);
        }
        Command::Handoff { sub } => {
            let result = match sub {
                HandoffCommand::Save { dir } => {
                    let mut text = String::new();
                    let _ = std::io::stdin().read_to_string(&mut text);
                    handoff::run_save(dir.as_deref(), &text)
                }
                HandoffCommand::Show { all, dir } => handoff::run_show(dir.as_deref(), all),
                HandoffCommand::Status => Ok(handoff::run_status()),
            };
            match result {
                Ok(output) => println!("{output}"),
                Err(err) => {
                    eprintln!("handoff: {err}");
                    std::process::exit(1);
                }
            }
        }
        Command::Mode { sub } => {
            let to_mode = |arg: ModeArg| match arg {
                ModeArg::Ask => common::mode::Mode::Ask,
                ModeArg::Auto => common::mode::Mode::Auto,
            };
            match sub {
                ModeCommand::Auto => exit_on_mode_error(mode::run_set(common::mode::Mode::Auto)),
                ModeCommand::Ask => exit_on_mode_error(mode::run_set(common::mode::Mode::Ask)),
                ModeCommand::Status { json, flag } => {
                    println!("{}", mode::run_status(flag.map(to_mode), json));
                }
            }
        }
    }
}

fn exit_on_mode_error(result: Result<String, String>) {
    match result {
        Ok(output) => println!("{output}"),
        Err(err) => {
            eprintln!("mode: {err}");
            std::process::exit(1);
        }
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

/// Print one `config get`/`list` line: a bare `deep`, not the JSON-quoted
/// `"deep"` a raw `Value`'s `Display` impl would print, so a shell script can
/// read it. An ignored invalid value is named in the source and warned about
/// on stderr.
fn print_resolved(
    command: &str,
    key: &str,
    (value, source, ignored): (
        serde_json::Value,
        config::Source,
        Option<config::IgnoredValue>,
    ),
) {
    let mut origin = source_label(source).to_string();
    if let Some(ignored) = ignored {
        eprintln!("{command}: warning: {}", ignored.warning);
        origin = format!("{origin}, {} value ignored", source_label(ignored.tier));
    }
    println!(
        "{key}: {} (source: {origin})",
        common::mode::value_text(&value)
    );
}

/// Parse `config set`'s raw `value` string into the JSON type `key` expects,
/// using `config::keys::default_value` to learn that type generically so an
/// unknown key or a wrong-type value is rejected before `write::set` ever
/// runs and touches a file. A boolean key accepts `true`/`false`
/// case-insensitively; a numeric key accepts any finite number, left to `write::set` to range check; any
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
        serde_json::Value::Number(_) => {
            parse_number(value).ok_or_else(|| config::ConfigError::InvalidNumber {
                key: key.to_string(),
                value: value.to_string(),
            })
        }
        _ => Ok(serde_json::Value::String(value.to_string())),
    }
}

/// A whole number stays an integer so the stored JSON has no `.0`; anything
/// else finite parses as a decimal. Range checks are left to `write::set`.
fn parse_number(value: &str) -> Option<serde_json::Value> {
    if let Ok(whole) = value.parse::<i64>() {
        return Some(whole.into());
    }
    if let Ok(whole) = value.parse::<u64>() {
        return Some(whole.into());
    }
    value
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(serde_json::Value::Number)
}

/// Reads all of stdin into a `String`, for the `json` subcommands that
/// take their JSON document piped in. Panics on a read failure: unlike
/// [`read_hook_input`], these commands have no silent-empty fallback that
/// would make sense, since an empty document is itself valid JSON input.
fn read_stdin_to_string() -> String {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("stdin should be readable");
    input
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

/// Makes a relative path absolute against the current directory.
fn absolute_from_cwd(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(p).to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// Moves into `--dir` before any git or gh call, so a shell that resets to
/// another checkout can still act on a worktree. Exits 1 on a bad path.
fn enter_dir(label: &str, dir: Option<&str>) {
    let Some(dir) = dir else { return };
    if let Err(err) = pr::shared::enter_dir(dir) {
        eprintln!("{label}: {err}");
        std::process::exit(1);
    }
}
