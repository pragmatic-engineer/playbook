// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Turns argv into a [`Cli`], or into the text and exit code `main` prints.

use crate::Cli;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::FromArgMatches;
use std::ffi::OsString;

/// Exit code for a usage error.
pub const EXIT_USAGE: i32 = 2;
/// Prefix of the one error line printed after a command's help.
pub const ERROR_PREFIX: &str = "error: ";
const MISSING_ARGUMENTS: &str = "the following required arguments were not provided:";

/// What `main` prints to stderr before exiting with `code`.
#[derive(Debug)]
pub struct ParseExit {
    pub code: i32,
    pub stderr: String,
}

/// Why argv did not become a [`Cli`].
enum Failure {
    /// Clap's own help or version display, printed to stdout with exit 0.
    Display(clap::Error),
    Exit(ParseExit),
}

/// The clap command tree for [`Cli`].
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// Parses `args` (including the program name).
pub fn parse(args: &[OsString]) -> Result<Cli, ParseExit> {
    parse_inner(args).map_err(|failure| match failure {
        Failure::Display(err) => ParseExit {
            code: err.exit_code(),
            stderr: err.to_string(),
        },
        Failure::Exit(exit) => exit,
    })
}

/// Parses `args`, or prints the failure and exits like `Cli::parse` does.
pub fn parse_or_exit(args: &[OsString]) -> Cli {
    match parse_inner(args) {
        Ok(cli) => cli,
        Err(Failure::Display(err)) => err.exit(),
        Err(Failure::Exit(exit)) => {
            eprint!("{}", exit.stderr);
            std::process::exit(exit.code)
        }
    }
}

fn parse_inner(args: &[OsString]) -> Result<Cli, Failure> {
    let mut cmd = command();
    let mut matches = match cmd.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(err) => return Err(classify(err, &mut cmd, args)),
    };
    Cli::from_arg_matches_mut(&mut matches).map_err(|err| classify(err, &mut cmd, args))
}

fn classify(err: clap::Error, cmd: &mut clap::Command, args: &[OsString]) -> Failure {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Failure::Display(err),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let help = deepest_command(cmd, args).render_long_help().to_string();
            Failure::Exit(ParseExit {
                code: err.exit_code(),
                stderr: help,
            })
        }
        ErrorKind::MissingRequiredArgument | ErrorKind::MissingSubcommand => {
            let help = deepest_command(cmd, args).render_long_help().to_string();
            Failure::Exit(ParseExit {
                code: EXIT_USAGE,
                stderr: format!("{help}\n{}\n", missing_line(&err)),
            })
        }
        _ => Failure::Exit(ParseExit {
            code: err.exit_code(),
            stderr: err.to_string(),
        }),
    }
}

/// Walks `args` past the program name, skipping flags, down to the last word
/// that names a subcommand.
fn deepest_command<'a>(root: &'a mut clap::Command, args: &[OsString]) -> &'a mut clap::Command {
    let mut current = root;
    for word in args.iter().skip(1).filter_map(|a| a.to_str()) {
        if word.starts_with('-') {
            continue;
        }
        if current.find_subcommand(word).is_none() {
            break;
        }
        current = current
            .find_subcommand_mut(word)
            .expect("the subcommand was just found");
    }
    current
}

fn missing_line(err: &clap::Error) -> String {
    match err.get(ContextKind::InvalidArg) {
        Some(ContextValue::Strings(names)) => {
            format!("{ERROR_PREFIX}{MISSING_ARGUMENTS} {}", names.join(" "))
        }
        _ => format!("{ERROR_PREFIX}{}", err.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Command;
    use clap::Parser;

    fn argv(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn parse_err(words: &[&str]) -> ParseExit {
        parse(&argv(words)).expect_err("the argv should not parse")
    }

    /// The long help of the command at `path`, as clap renders it.
    fn long_help(path: &[&str]) -> String {
        let mut cmd = command();
        cmd.build();
        let mut cur = &mut cmd;
        for word in path {
            cur = cur
                .find_subcommand_mut(word)
                .expect("the path names a command");
        }
        cur.render_long_help().to_string()
    }

    #[test]
    fn a_missing_positional_prints_long_help_then_one_error_line() {
        // Arrange
        let args = ["playbook", "trust"];

        // Act
        let exit = parse_err(&args);

        // Assert
        assert_eq!(exit.code, 2);
        assert!(exit.stderr.contains("Usage: playbook trust <PATH>"));
        assert!(exit.stderr.contains("Absolute path"));
        let about = command()
            .find_subcommand("trust")
            .and_then(|c| c.get_long_about().map(ToString::to_string))
            .expect("trust has a long about");
        let first_line = about.lines().next().expect("a first line");
        assert!(exit.stderr.contains(first_line));
        assert_eq!(
            exit.stderr.trim_end().lines().last(),
            Some("error: the following required arguments were not provided: <PATH>")
        );
    }

    #[test]
    fn a_missing_nested_positional_names_the_deepest_command() {
        // Arrange
        let rows: [(&[&str], &[&str]); 2] = [
            (&["playbook", "config", "get"], &["config", "get"]),
            (&["playbook", "config", "set", "KEY"], &["config", "set"]),
        ];
        for (args, path) in rows {
            // Act
            let exit = parse_err(args);

            // Assert
            assert_eq!(exit.code, 2, "{args:?}");
            assert!(exit.stderr.starts_with(&long_help(path)), "{args:?}");
        }
    }

    #[test]
    fn a_missing_required_flag_prints_that_commands_help() {
        // Arrange
        let args = ["playbook", "pr", "reply"];

        // Act
        let exit = parse_err(&args);

        // Assert
        assert_eq!(exit.code, 2);
        assert!(exit.stderr.starts_with(&long_help(&["pr", "reply"])));
    }

    #[test]
    fn a_flag_before_the_missing_positional_does_not_hide_the_command() {
        // Arrange
        let args = ["playbook", "route", "--tier", "high"];

        // Act
        let exit = parse_err(&args);

        // Assert
        assert_eq!(exit.code, 2);
        assert!(exit.stderr.starts_with(&long_help(&["route"])));
    }

    #[test]
    fn a_bare_group_still_prints_its_help_and_exits_two() {
        // Arrange
        for group in ["plan", "config"] {
            // Act
            let exit = parse_err(&["playbook", group]);

            // Assert
            assert_eq!(exit.code, 2, "{group}");
            assert!(exit.stderr.starts_with(&long_help(&[group])), "{group}");
        }
    }

    #[test]
    fn commands_with_a_bare_form_still_parse_without_arguments() {
        // Arrange
        type Check = fn(&Command) -> bool;
        let rows: [(&str, Check); 4] = [
            (
                "cc",
                |c| matches!(c, Command::Cc { sub: None, args } if args.is_empty()),
            ),
            ("effort", |c| {
                matches!(
                    c,
                    Command::Effort {
                        sub: None,
                        level: None,
                        json: false
                    }
                )
            }),
            ("usage", |c| {
                matches!(
                    c,
                    Command::Usage {
                        sub: None,
                        json: false,
                        summary: false,
                        range: None,
                        ..
                    }
                )
            }),
            ("sessions", |c| {
                matches!(
                    c,
                    Command::Sessions {
                        sub: None,
                        json: false
                    }
                )
            }),
        ];
        for (word, is_bare_form) in rows {
            // Act
            let cli = parse(&argv(&["playbook", word])).expect("a bare form parses");

            // Assert
            assert!(is_bare_form(&cli.command), "{word}: {:?}", cli.command);
        }
    }

    #[test]
    fn an_unrelated_clap_error_is_left_to_clap() {
        // Arrange
        let rows: [&[&str]; 2] = [&["playbook", "trust", "a", "b"], &["playbook", "nope"]];
        for args in rows {
            let clap_text = Cli::try_parse_from(args)
                .expect_err("clap rejects it")
                .to_string();

            // Act
            let exit = parse_err(args);

            // Assert
            assert_eq!(exit.code, 2, "{args:?}");
            assert_eq!(exit.stderr, clap_text, "{args:?}");
        }
    }
}
