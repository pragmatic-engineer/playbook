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

mod alloc;
mod limits;
mod watchdog;

#[global_allocator]
static ALLOCATOR: alloc::CountingAllocator = alloc::CountingAllocator;

const DEFAULT_MAX_MEMORY_MB: u64 = 1024;
const MAX_MEMORY_MB_CLAMP: u64 = 4096;
const BYTES_PER_MB: u64 = 1 << 20;
const CONVERT_THREADS: usize = 1;

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
    Convert {
        input: String,
        output: String,
        limits: Limits,
    },
}

/// The per-run resource limits taken from the command line.
struct Limits {
    max_input_bytes: u64,
    max_memory_mb: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: limits::HARD_MAX_INPUT_BYTES,
            max_memory_mb: DEFAULT_MAX_MEMORY_MB,
        }
    }
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

impl From<limits::AdmitError> for Failure {
    fn from(err: limits::AdmitError) -> Self {
        match err {
            limits::AdmitError::Unreadable(message) => Self::failed(message),
            limits::AdmitError::TooLarge(refusal) => refusal.into(),
        }
    }
}

impl From<limits::Refusal> for Failure {
    fn from(refusal: limits::Refusal) -> Self {
        Self {
            code: EXIT_RESOURCE_LIMIT,
            message: refusal.0,
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
        Command::Convert {
            input,
            output,
            limits,
        } => convert(&input, &output, &limits)?,
    }
    Ok(())
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, Failure> {
    let mut input = None;
    let mut output = None;
    let mut limits = Limits::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            FLAG_VERSION => return Ok(Command::PrintVersion),
            FLAG_CONVERTER_VERSION => return Ok(Command::PrintConverterVersion),
            FLAG_OUTPUT => output = Some(flag_value(&mut args, FLAG_OUTPUT)?),
            FLAG_MAX_INPUT_BYTES => {
                limits.max_input_bytes = number_flag(&mut args, FLAG_MAX_INPUT_BYTES)?;
            }
            FLAG_MAX_MEMORY_MB => {
                limits.max_memory_mb =
                    number_flag(&mut args, FLAG_MAX_MEMORY_MB)?.min(MAX_MEMORY_MB_CLAMP);
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
        (Some(input), Some(output)) => Ok(Command::Convert {
            input,
            output,
            limits,
        }),
        (None, _) => Err(Failure::usage("missing input path")),
        (_, None) => Err(Failure::usage("missing -o output path")),
    }
}

fn flag_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, Failure> {
    args.next()
        .ok_or_else(|| Failure::usage(format!("{flag} needs a value")))
}

fn number_flag(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<u64, Failure> {
    flag_value(args, flag)?
        .parse()
        .map_err(|_| Failure::usage(format!("{flag} needs a whole number")))
}

/// Runs the safeguards of ADR 0051 in order, then converts.
fn convert(input: &str, output: &str, limits: &Limits) -> Result<(), Failure> {
    let cap_bytes = usize::try_from(limits.max_memory_mb * BYTES_PER_MB).unwrap_or(usize::MAX);
    watchdog::spawn(cap_bytes).map_err(|_| Failure::failed("cannot start the memory watchdog"))?;
    alloc::set_heap_cap(cap_bytes);
    // lopdf parallelises through rayon's global pool, so one thread bounds PDF memory.
    rayon::ThreadPoolBuilder::new()
        .num_threads(CONVERT_THREADS)
        .build_global()
        .map_err(|_| Failure::failed("cannot start the worker pool"))?;

    let path = Path::new(input);
    limits::admit(path, limits.max_input_bytes)?;
    limits::prescan_zip(path)?;
    let bytes = fs::read(path).map_err(|err| io_failure("cannot read input", &err))?;
    let markdown = anydoc::to_markdown_bytes(&bytes, Format::from_path(path))?;
    fs::write(output, markdown).map_err(|err| io_failure("cannot write output", &err))
}

fn io_failure(what: &str, err: &io::Error) -> Failure {
    Failure::failed(format!("{what}: {}", err.kind()))
}
