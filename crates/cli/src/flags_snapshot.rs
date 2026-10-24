// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A text snapshot of the attributes of every command and flag, without help
//! text. Rewriting help must not change any line of it.
//!
//! It lives in the library, not in a test, because an example cannot import
//! code from a test file.

use clap::{Arg, Command};
use std::fmt::Write as _;
use std::io;
use std::path::Path;

const LIST_SEPARATOR: &str = ",";
const NONE: &str = "-";

/// Renders one line per command and per argument, in tree order.
pub fn render_flags_snapshot(root: &Command) -> String {
    let mut out = String::new();
    render_command(root, root.get_name(), &mut out);
    out
}

/// Writes the snapshot to `path`, replacing the file.
pub fn write_flags_snapshot(root: &Command, path: &Path) -> io::Result<()> {
    std::fs::write(path, render_flags_snapshot(root))
}

fn list<I, S>(items: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let joined = items
        .into_iter()
        .map(|item| item.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR);
    if joined.is_empty() {
        NONE.to_string()
    } else {
        joined
    }
}

fn render_command(cmd: &Command, path: &str, out: &mut String) {
    let subcommands = list(cmd.get_subcommands().map(Command::get_name));
    let aliases = list(cmd.get_all_aliases());
    let groups = list(cmd.get_groups().map(|group| {
        let members = list(group.get_args().map(|id| id.as_str().to_string()));
        format!("{}[{members}]", group.get_id())
    }));
    let _ = writeln!(
        out,
        "command {path} subcommands={subcommands} aliases={aliases} groups={groups}"
    );
    for arg in cmd.get_arguments() {
        render_arg(cmd, arg, path, out);
    }
    for sub in cmd.get_subcommands() {
        render_command(sub, &format!("{path} {}", sub.get_name()), out);
    }
}

fn render_arg(cmd: &Command, arg: &Arg, path: &str, out: &mut String) {
    let long = arg.get_long().unwrap_or(NONE);
    let short = arg.get_short().map_or(NONE.to_string(), String::from);
    let value_names = list(
        arg.get_value_names()
            .unwrap_or_default()
            .iter()
            .map(|name| name.as_str()),
    );
    let possible = list(
        arg.get_possible_values()
            .iter()
            .map(|value| value.get_name().to_string()),
    );
    let defaults = list(
        arg.get_default_values()
            .iter()
            .map(|value| value.to_string_lossy()),
    );
    let aliases = list(arg.get_all_aliases().unwrap_or_default());
    let num_args = arg
        .get_num_args()
        .map_or(NONE.to_string(), |range| range.to_string());
    let conflicts = list(
        cmd.get_arg_conflicts_with(arg)
            .iter()
            .map(|other| other.get_id().as_str().to_string()),
    );
    let _ = writeln!(
        out,
        "arg {path} id={} long={long} short={short} value_names={value_names} \
         possible={possible} defaults={defaults} aliases={aliases} required={} hidden={} \
         global={} action={:?} num_args={num_args} parser={:?} conflicts={conflicts}",
        arg.get_id(),
        arg.is_required_set(),
        arg.is_hide_set(),
        arg.is_global_set(),
        arg.get_action(),
        arg.get_value_parser().type_id(),
    );
}
