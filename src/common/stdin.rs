// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reading piped standard input without hanging on a terminal.
//! `read_to_string` on a terminal waits for Ctrl-D, and in an agent shell it
//! waits until the tool timeout. Commands that take their document on stdin
//! use [`read_piped`], which refuses a terminal with a clear error instead.

use std::io::{IsTerminal, Read};

/// The error shown when a command that needs piped input is run at a terminal.
pub const TERMINAL_ERROR: &str = "expected piped input, got a terminal";

/// Reads `reader` to the end, or fails with [`TERMINAL_ERROR`] when
/// `is_terminal` is true. Split from [`read_piped`] so the guard is testable.
pub fn read_guarded<R: Read>(mut reader: R, is_terminal: bool) -> Result<String, String> {
    if is_terminal {
        return Err(TERMINAL_ERROR.to_string());
    }
    let mut input = String::new();
    reader
        .read_to_string(&mut input)
        .map_err(|err| format!("cannot read stdin: {err}"))?;
    Ok(input)
}

/// All of standard input, or an error when it is a terminal or unreadable.
pub fn read_piped() -> Result<String, String> {
    let stdin = std::io::stdin();
    let is_terminal = stdin.is_terminal();
    read_guarded(stdin.lock(), is_terminal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_is_refused_without_reading() {
        struct Panics;
        impl Read for Panics {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("must not read from a terminal");
            }
        }

        let got = read_guarded(Panics, true);

        assert_eq!(got, Err(TERMINAL_ERROR.to_string()));
    }

    #[test]
    fn piped_input_is_read_whole() {
        let got = read_guarded("line one\nline two".as_bytes(), false);

        assert_eq!(got, Ok("line one\nline two".to_string()));
    }

    #[test]
    fn invalid_utf8_is_an_error_not_a_panic() {
        let got = read_guarded(&[0xff, 0xfe][..], false);

        assert!(got.is_err_and(|e| e.starts_with("cannot read stdin")));
    }
}
