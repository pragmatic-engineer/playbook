// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pbdoc` converts one document to Markdown offline, as playbook's bundled
//! helper (see ADR 0051). Exit codes: 0 ok, 1 failed, 2 usage, 3 needs OCR,
//! 4 resource limit. Failures print one `error: <message>` line on stderr and
//! never include document text.

use std::path::Path;
use std::process::ExitCode;
use std::{fs, io};

use anydoc::{ConvertError, Format};

const EXIT_FAILED: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_NEEDS_OCR: u8 = 3;
const EXIT_RESOURCE_LIMIT: u8 = 4;

const FLAG_OUTPUT: &str = "-o";
const FLAG_END_OF_FLAGS: &str = "--";
const FLAG_MAX_INPUT_BYTES: &str = "--max-input-bytes";
const FLAG_MAX_MEMORY_MB: &str = "--max-memory-mb";
const FLAG_VERSION: &str = "--version";
const FLAG_CONVERTER_VERSION: &str = "--converter-version";

const CONVERTER_VERSION: &str = "anydoc 0.2.4";

/// What the command line asked for.
enum Command {
    PrintVersion,
    PrintConverterVersion,
    Convert { input: String, output: String },
}

/// A failure carrying the exit code and the one stderr line to report.
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_USAGE,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_FAILED,
            message: message.into(),
        }
    }
}

impl From<ConvertError> for Failure {
    fn from(err: ConvertError) -> Self {
        let code = match err {
            ConvertError::NeedsOcr { .. } => EXIT_NEEDS_OCR,
            ConvertError::ResourceLimit { .. } => EXIT_RESOURCE_LIMIT,
            _ => EXIT_FAILED,
        };
        Self {
            code,
            message: err.to_string(),
        }
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<(), Failure> {
    match parse_args(args)? {
        Command::PrintVersion => println!("pbdoc {}", env!("CARGO_PKG_VERSION")),
        Command::PrintConverterVersion => println!("{CONVERTER_VERSION}"),
        Command::Convert { input, output } => convert(&input, &output)?,
    }
    Ok(())
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, Failure> {
    let mut input = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            FLAG_VERSION => return Ok(Command::PrintVersion),
            FLAG_CONVERTER_VERSION => return Ok(Command::PrintConverterVersion),
            FLAG_OUTPUT => output = Some(flag_value(&mut args, FLAG_OUTPUT)?),
            // Accepted so callers can pass them now; the limits are not enforced yet.
            FLAG_MAX_INPUT_BYTES | FLAG_MAX_MEMORY_MB => {
                flag_value(&mut args, &arg)?;
            }
            FLAG_END_OF_FLAGS => {
                input = Some(
                    args.next()
                        .ok_or_else(|| Failure::usage("missing input path"))?,
                );
                if args.next().is_some() {
                    return Err(Failure::usage("expected exactly one input path"));
                }
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(Failure::usage(format!("unknown flag {flag}")));
            }
            _ => return Err(Failure::usage("input path must follow --")),
        }
    }
    match (input, output) {
        (Some(input), Some(output)) => Ok(Command::Convert { input, output }),
        (None, _) => Err(Failure::usage("missing input path")),
        (_, None) => Err(Failure::usage("missing -o output path")),
    }
}

fn flag_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, Failure> {
    args.next()
        .ok_or_else(|| Failure::usage(format!("{flag} needs a value")))
}

fn convert(input: &str, output: &str) -> Result<(), Failure> {
    let bytes = fs::read(input).map_err(|err| io_failure("cannot read input", &err))?;
    let markdown = anydoc::to_markdown_bytes(&bytes, Format::from_path(Path::new(input)))?;
    fs::write(output, markdown).map_err(|err| io_failure("cannot write output", &err))
}

fn io_failure(what: &str, err: &io::Error) -> Failure {
    Failure::failed(format!("{what}: {}", err.kind()))
}
