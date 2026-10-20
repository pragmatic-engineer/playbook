// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `pbdoc` command line contract: exit codes, the single stderr line and
//! the exact Markdown for each fixture. Every test runs the built binary in its
//! own scratch directory.

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const EXIT_MALFORMED: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_NEEDS_OCR: i32 = 3;
const EXIT_RESOURCE_LIMIT: i32 = 4;
const TRUNCATED_DOCX_BYTES: usize = 512;

const MIB: u64 = 1 << 20;
const FIVE_GIB: u64 = 5 << 30;
const HARD_MAX_INPUT_BYTES: u64 = 104_857_600;
const FLAG_MAX_INPUT: &str = "--max-input-bytes";
const LOW_INPUT_LIMIT: u64 = 1_000;
const SPARSE_ALLOCATED_LIMIT: u64 = MIB;
const BLOCK_BYTES: u64 = 512;
const MAX_ZIP_ENTRIES: usize = 10_000;
const BOMB_ENTRY_BYTES: usize = 2 << 20;
const LOW_MEMORY_MB: &str = "8";
const CSV_BYTES: usize = 20 << 20;
const CORRUPT_ENTRY_BYTES: usize = 4096;
// Local file header (30 bytes) plus the entry name word/document.xml (17 bytes).
const CORRUPT_ENTRY_HEADER_BYTES: usize = 47;
const CORRUPT_SPAN_BYTES: usize = 4;
const MEMORY_LIMIT_MESSAGE: &str = "memory limit reached";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// The `anydoc` pin in this crate's manifest, the single source of the converter version.
fn pinned_anydoc_version() -> String {
    let manifest = include_str!("../Cargo.toml");
    let line = manifest
        .lines()
        .find(|line| line.starts_with("anydoc ="))
        .expect("manifest pins anydoc");
    let (_, rest) = line.split_once("version = \"=").expect("exact pin");
    rest.split('"').next().expect("closing quote").to_owned()
}

fn scratch_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pbdoc-contract-{}-{tag}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn pbdoc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pbdoc"))
        .args(args)
        .output()
        .unwrap()
}

fn convert(input: &Path, out: &Path) -> Output {
    pbdoc(&["-o", out.to_str().unwrap(), "--", input.to_str().unwrap()])
}

fn assert_converts_to_golden(fixture_name: &str, golden_name: &str) {
    // Arrange
    let out = scratch_dir("golden").join("out.md");

    // Act
    let output = convert(&fixture(fixture_name), &out);

    // Assert
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(
        fs::read(&out).unwrap(),
        fs::read(fixture(golden_name)).unwrap()
    );
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn converts_a_csv_to_a_markdown_table() {
    // Arrange
    let out = scratch_dir("csv").join("out");
    let expected = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";

    // Act
    let output = convert(&fixture("sample.csv"), &out);

    // Assert
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(fs::read_to_string(&out).unwrap(), expected);
}

#[test]
fn converts_a_docx_with_a_heading_and_a_paragraph_to_its_golden() {
    assert_converts_to_golden("sample.docx", "sample-docx.expected.md");
}

#[test]
fn converts_an_xlsx_sheet_to_its_golden() {
    assert_converts_to_golden("sample.xlsx", "sample-xlsx.expected.md");
}

#[test]
fn converts_an_odt_to_its_golden() {
    assert_converts_to_golden("sample.odt", "sample-odt.expected.md");
}

#[test]
fn converts_an_epub_to_its_golden() {
    assert_converts_to_golden("sample.epub", "sample-epub.expected.md");
}

#[test]
fn converts_a_text_pdf_to_its_golden() {
    assert_converts_to_golden("text.pdf", "text-pdf.expected.md");
}

#[test]
fn converts_an_empty_csv_to_its_golden() {
    assert_converts_to_golden("empty.csv", "empty.expected.md");
}

#[test]
fn rejects_an_empty_docx_with_the_malformed_message() {
    // Arrange
    let out = scratch_dir("empty-docx").join("out.md");

    // Act
    let output = convert(&fixture("empty.docx"), &out);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_MALFORMED));
    assert_eq!(
        output.stderr,
        fs::read(fixture("empty-docx.expected.err")).unwrap()
    );
}

#[test]
fn rejects_a_truncated_docx_with_one_stderr_line() {
    // Arrange
    let dir = scratch_dir("truncated-docx");
    let input = dir.join("truncated.docx");
    let out = dir.join("out.md");
    let whole = fs::read(fixture("sample.docx")).unwrap();
    fs::write(&input, &whole[..TRUNCATED_DOCX_BYTES]).unwrap();

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_MALFORMED));
    let message = stderr(&output);
    assert_eq!(message.matches('\n').count(), 1, "stderr: {message:?}");
    assert_eq!(
        output.stderr,
        fs::read(fixture("truncated-docx.expected.err")).unwrap()
    );
}

#[test]
fn reports_its_own_version_as_the_last_token() {
    // Arrange
    let expected = env!("CARGO_PKG_VERSION");

    // Act
    let output = pbdoc(&["--version"]);

    // Assert
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.split_whitespace().last(), Some(expected));
}

#[test]
fn reports_the_converter_version() {
    // Arrange
    let expected = format!("anydoc {}", pinned_anydoc_version());

    // Act
    let output = pbdoc(&["--converter-version"]);

    // Assert
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        expected
    );
}

macro_rules! rejects_flag {
    ($name:ident, $($flag:expr),+) => {
        #[test]
        fn $name() {
            // Arrange
            let out = scratch_dir("flag").join("out.md");
            let input = fixture("sample.csv");
            let mut args = vec![$($flag),+, "-o", out.to_str().unwrap(), "--"];
            args.push(input.to_str().unwrap());

            // Act
            let output = pbdoc(&args);

            // Assert
            assert_eq!(output.status.code(), Some(EXIT_USAGE));
            assert!(!out.exists());
        }
    };
}

rejects_flag!(rejects_the_ocr_flag, "--ocr");
rejects_flag!(rejects_the_url_flag, "--url", "x");
rejects_flag!(rejects_the_api_key_flag, "--api-key", "k");
rejects_flag!(rejects_the_hosted_flag, "--hosted");

#[test]
fn exits_3_for_a_pdf_without_a_text_layer() {
    // Arrange
    let out = scratch_dir("scanned").join("out.md");

    // Act
    let output = convert(&fixture("scanned.pdf"), &out);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_NEEDS_OCR));
}

#[test]
fn exits_2_when_no_output_path_is_given() {
    // Arrange
    let input = fixture("sample.csv");

    // Act
    let output = pbdoc(&["--", input.to_str().unwrap()]);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_USAGE));
}

#[test]
fn a_missing_input_path_exits_1() {
    // Arrange
    let dir = scratch_dir("missing-input");
    let out = dir.join("out.md");

    // Act
    let output = convert(&dir.join("absent.docx"), &out);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_MALFORMED));
    assert!(stderr(&output).contains("cannot read input"));
    assert!(!out.exists());
}

#[test]
fn a_directory_as_input_exits_1() {
    // Arrange
    let dir = scratch_dir("dir-input");
    let out = dir.join("out.md");

    // Act
    let output = convert(&dir, &out);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_MALFORMED));
    assert!(stderr(&output).contains("not a file"));
    assert!(!out.exists());
}

/// Creates a sparse file of `len` bytes, or prints why it skips when the
/// filesystem allocated real blocks.
fn sparse_file(tag: &str, len: u64) -> Option<PathBuf> {
    let path = scratch_dir(tag).join("big.csv");
    File::create(&path).unwrap().set_len(len).unwrap();
    if fs::metadata(&path).unwrap().blocks() * BLOCK_BYTES > SPARSE_ALLOCATED_LIMIT {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must run on a filesystem with sparse files"
        );
        eprintln!("skipped: the filesystem does not make sparse files");
        return None;
    }
    Some(path)
}

fn zip_with_entries(path: &Path, entries: usize, body: &[u8], method: zip::CompressionMethod) {
    let options = zip::write::SimpleFileOptions::default().compression_method(method);
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
    for i in 0..entries {
        writer.start_file(format!("e{i}.xml"), options).unwrap();
        writer.write_all(body).unwrap();
    }
    writer.finish().unwrap();
}

fn assert_refused(output: &Output, out: &Path, reason: &str) {
    assert_eq!(
        output.status.code(),
        Some(EXIT_RESOURCE_LIMIT),
        "stderr: {}",
        stderr(output)
    );
    assert!(
        stderr(output).contains(reason),
        "stderr: {}",
        stderr(output)
    );
    assert!(!out.exists());
}

#[test]
fn refuses_a_five_gib_sparse_file_as_too_large_without_reading_it() {
    // Arrange
    let Some(input) = sparse_file("five-gib", FIVE_GIB) else {
        return;
    };
    let out = scratch_dir("five-gib-out").join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_refused(&output, &out, "too large");
}

#[test]
fn refuses_an_input_one_byte_over_the_hard_cap() {
    // Arrange
    let Some(input) = sparse_file("hard-over", HARD_MAX_INPUT_BYTES + 1) else {
        return;
    };
    let out = scratch_dir("hard-over-out").join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_refused(&output, &out, "too large");
}

#[test]
fn admits_an_input_exactly_at_the_hard_cap_past_the_size_check() {
    // Arrange
    let Some(input) = sparse_file("hard-at", HARD_MAX_INPUT_BYTES) else {
        return;
    };
    let out = scratch_dir("hard-at-out").join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert!(
        !stderr(&output).contains("too large"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_lower_max_input_bytes_flag_refuses_one_byte_over_it() {
    // Arrange
    let input = scratch_dir("flag-over").join("in.csv");
    fs::write(&input, vec![b'a'; LOW_INPUT_LIMIT as usize + 1]).unwrap();
    let out = scratch_dir("flag-over-out").join("out.md");
    let limit = LOW_INPUT_LIMIT.to_string();

    // Act
    let output = pbdoc(&[
        FLAG_MAX_INPUT,
        &limit,
        "-o",
        out.to_str().unwrap(),
        "--",
        input.to_str().unwrap(),
    ]);

    // Assert
    assert_refused(&output, &out, "too large");
}

#[test]
fn a_lower_max_input_bytes_flag_admits_a_file_exactly_at_it() {
    // Arrange
    let input = scratch_dir("flag-at").join("in.csv");
    fs::write(&input, vec![b'a'; LOW_INPUT_LIMIT as usize]).unwrap();
    let out = scratch_dir("flag-at-out").join("out.md");
    let limit = LOW_INPUT_LIMIT.to_string();

    // Act
    let output = pbdoc(&[
        FLAG_MAX_INPUT,
        &limit,
        "-o",
        out.to_str().unwrap(),
        "--",
        input.to_str().unwrap(),
    ]);

    // Assert
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
}

#[test]
fn refuses_a_docx_with_more_entries_than_the_limit() {
    // Arrange
    let input = scratch_dir("many-entries").join("bomb.docx");
    zip_with_entries(
        &input,
        MAX_ZIP_ENTRIES + 1,
        b"",
        zip::CompressionMethod::Stored,
    );
    let out = scratch_dir("many-entries-out").join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_refused(&output, &out, "entries");
}

#[test]
fn refuses_a_docx_entry_with_a_compression_ratio_over_the_limit() {
    // Arrange
    let input = scratch_dir("ratio").join("bomb.docx");
    let zeros = vec![0u8; BOMB_ENTRY_BYTES];
    zip_with_entries(&input, 1, &zeros, zip::CompressionMethod::Deflated);
    let out = scratch_dir("ratio-out").join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_refused(&output, &out, "ratio");
}

#[test]
fn stops_with_the_memory_limit_message_when_reading_exceeds_the_cap() {
    // Arrange
    let input = scratch_dir("memory").join("big.csv");
    let row = b"abcdefghijklmnopqrstuvwxyz,0123456789\n";
    let mut csv = Vec::with_capacity(CSV_BYTES + row.len());
    while csv.len() < CSV_BYTES {
        csv.extend_from_slice(row);
    }
    fs::write(&input, csv).unwrap();
    let out = scratch_dir("memory-out").join("out.md");

    // Act
    let output = pbdoc(&[
        "--max-memory-mb",
        LOW_MEMORY_MB,
        "-o",
        out.to_str().unwrap(),
        "--",
        input.to_str().unwrap(),
    ]);

    // Assert
    assert_eq!(output.status.code(), Some(EXIT_RESOURCE_LIMIT));
    assert_eq!(stderr(&output).trim_end(), MEMORY_LIMIT_MESSAGE);
    assert!(!out.exists());
}

#[test]
fn real_fixtures_still_convert_under_the_default_limits() {
    for name in ["sample.docx", "sample.xlsx", "sample.csv"] {
        // Arrange
        let out = scratch_dir("defaults").join("out.md");

        // Act
        let output = convert(&fixture(name), &out);

        // Assert
        assert_eq!(output.status.code(), Some(0), "{name}: {}", stderr(&output));
    }
}

#[test]
fn a_docx_with_a_corrupt_entry_and_an_intact_directory_exits_1_with_one_error_line() {
    // Arrange
    let dir = scratch_dir("corrupt-entry");
    let input = dir.join("bad.docx");
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut writer = zip::ZipWriter::new(File::create(&input).unwrap());
    writer.start_file("word/document.xml", options).unwrap();
    writer.write_all(&vec![b'a'; CORRUPT_ENTRY_BYTES]).unwrap();
    writer.finish().unwrap();
    let mut bytes = fs::read(&input).unwrap();
    for byte in &mut bytes[CORRUPT_ENTRY_HEADER_BYTES..][..CORRUPT_SPAN_BYTES] {
        *byte = !*byte;
    }
    fs::write(&input, bytes).unwrap();
    let out = dir.join("out.md");

    // Act
    let output = convert(&input, &out);

    // Assert
    assert_eq!(
        output.status.code(),
        Some(EXIT_MALFORMED),
        "stderr: {}",
        stderr(&output)
    );
    assert!(stderr(&output).starts_with("error: "));
    assert_eq!(stderr(&output).lines().count(), 1);
}
