// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shell completion scripts for `playbook` and its `pb` alias.

use clap::ValueEnum;

/// The shells `playbook completions` supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl CompletionShell {
    // Not `clap_complete::Shell` itself, which also accepts elvish and powershell.
    fn to_clap(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
        }
    }
}

const BIN_NAME: &str = "playbook";

/// The first line of the fish script.
pub const FISH_FIRST_LINE: &str = "# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.";

const ZSH_FIRST_LINE: &str = "#compdef playbook pb";
const PB_GUARD_ZSH: &str = "(( $+functions[compdef] )) && compdef _playbook pb\n";
const PB_GUARD_BASH: &str = "[[ $(command -v pb) -ef $(command -v playbook) ]] && complete -F _playbook -o bashdefault -o default pb\n";
const PB_GUARD_FISH: &str = "if command -sq pb; and command -sq playbook; and test (command -s pb) -ef (command -s playbook)\n    complete -c pb -w playbook\nend\n";

/// The completion script for `shell`. `pb` is registered only where it is
/// the same file as `playbook`.
pub fn render(shell: CompletionShell) -> String {
    let mut cmd = crate::parse::command();
    let mut out = Vec::new();
    clap_complete::generate(shell.to_clap(), &mut cmd, BIN_NAME, &mut out);
    let script = String::from_utf8_lossy(&out).into_owned();
    match shell {
        CompletionShell::Bash => format!("{script}{PB_GUARD_BASH}"),
        CompletionShell::Zsh => {
            let body = script.split_once('\n').map_or("", |(_, rest)| rest);
            format!("{ZSH_FIRST_LINE}\n{body}{PB_GUARD_ZSH}")
        }
        CompletionShell::Fish => format!("{script}{PB_GUARD_FISH}"),
    }
}
