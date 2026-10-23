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

// The `playbook` and `_playbook` literals below must change with this name.
const BIN_NAME: &str = "playbook";

// The header clap_complete (pinned with `=`) writes on the first zsh line.
const ZSH_CLAP_HEADER: &str = "#compdef playbook\n";
const ZSH_FIRST_LINE: &str = "#compdef playbook pb\n";
const PB_GUARD_ZSH: &str = "if (( $+functions[compdef] )) && [[ ${commands[pb]} -ef ${commands[playbook]} ]]; then\n    compdef _playbook pb\nfi\n";
const PB_GUARD_BASH: &str = "if [[ $(command -v pb) -ef $(command -v playbook) ]]; then\n    complete -F _playbook -o bashdefault -o default pb\nfi\n";
const PB_GUARD_FISH: &str = "if command -sq pb; and command -sq playbook; and test (command -s pb) -ef (command -s playbook)\n    complete -c pb -w playbook\nend\n";

/// The completion script for `shell`. `pb` is registered only where it is
/// the same file as `playbook`, and a false test leaves the exit status 0.
///
/// Fails when the zsh script does not start with the header this crate
/// rewrites, so a `clap_complete` change cannot drop real code silently.
pub fn render(shell: CompletionShell) -> Result<String, String> {
    let mut cmd = crate::parse::command();
    let mut out = Vec::new();
    clap_complete::generate(shell.to_clap(), &mut cmd, BIN_NAME, &mut out);
    let script = String::from_utf8_lossy(&out).into_owned();
    match shell {
        CompletionShell::Bash => Ok(format!("{script}{PB_GUARD_BASH}")),
        CompletionShell::Zsh => {
            let body = script
                .strip_prefix(ZSH_CLAP_HEADER)
                .ok_or_else(|| format!("the zsh script does not start with {ZSH_CLAP_HEADER:?}"))?;
            Ok(format!("{ZSH_FIRST_LINE}{body}{PB_GUARD_ZSH}"))
        }
        CompletionShell::Fish => Ok(format!("{script}{PB_GUARD_FISH}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_complete_zsh_output_starts_with_the_pinned_header() {
        // Arrange
        let mut cmd = crate::parse::command();
        let mut out = Vec::new();

        // Act
        clap_complete::generate(clap_complete::Shell::Zsh, &mut cmd, BIN_NAME, &mut out);

        // Assert
        assert!(String::from_utf8_lossy(&out).starts_with(ZSH_CLAP_HEADER));
    }
}
