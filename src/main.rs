// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

use clap::Parser;
use playbook::{Cli, Command};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Hook { name } => todo!("hook {name} is not implemented yet"),
        Command::Cc { sub } => todo!("cc {sub} is not implemented yet"),
        Command::Statusline => todo!("statusline is not implemented yet"),
        Command::Init => todo!("init is not implemented yet"),
    }
}
