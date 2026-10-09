// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Hot paths that run before clap builds its command tree.
//!
//! Claude Code runs `playbook hook <name>` on every tool call and
//! `playbook statusline` on every redraw, and the whole clap tree costs 2 to
//! 3 ms to build. These forms are recognised from the raw arguments. Anything
//! else, including a wrong name, an extra argument or a flag, returns `None`
//! and falls through to clap, so `--help` and every error message are unchanged.

use crate::HookName;
use clap::ValueEnum;
use std::ffi::OsString;

/// A command recognised without clap.
#[derive(Debug, Clone, Copy)]
pub enum Fast {
    Hook(HookName),
    Statusline,
    Version,
}

/// The fast command for `args` (the full argv, program name first), if any.
pub fn classify(args: &[OsString]) -> Option<Fast> {
    let first = args.get(1)?.to_str()?;
    match (first, args.len()) {
        ("hook", 3) => {
            let name = args.get(2)?.to_str()?;
            HookName::from_str(name, false).ok().map(Fast::Hook)
        }
        ("statusline", 2) => Some(Fast::Statusline),
        ("--version" | "-V", 2) => Some(Fast::Version),
        _ => None,
    }
}

/// What `playbook --version` prints, the same text clap renders.
pub fn version_text() -> String {
    format!("playbook {}\n", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_three_hot_forms_are_recognised() {
        assert!(matches!(
            classify(&argv(&["playbook", "hook", "session-init"])),
            Some(Fast::Hook(HookName::SessionInit))
        ));
        assert!(matches!(
            classify(&argv(&["playbook", "statusline"])),
            Some(Fast::Statusline)
        ));
        assert!(matches!(
            classify(&argv(&["playbook", "--version"])),
            Some(Fast::Version)
        ));
        assert!(matches!(
            classify(&argv(&["playbook", "-V"])),
            Some(Fast::Version)
        ));
    }

    #[test]
    fn anything_else_falls_through_to_clap() {
        for parts in [
            &["playbook"][..],
            &["playbook", "hook"],
            &["playbook", "hook", "nope"],
            &["playbook", "hook", "session-init", "extra"],
            &["playbook", "hook", "--help"],
            &["playbook", "hook", "Session-Init"],
            &["playbook", "statusline", "--help"],
            &["playbook", "--version", "x"],
            &["playbook", "--help"],
            &["playbook", "config", "list"],
        ] {
            assert!(classify(&argv(parts)).is_none(), "{parts:?}");
        }
    }

    #[test]
    fn a_non_utf8_argument_falls_through() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let args = vec![
                OsString::from("playbook"),
                OsString::from("hook"),
                OsString::from_vec(vec![0xff, 0xfe]),
            ];
            assert!(classify(&args).is_none());
        }
    }
}
