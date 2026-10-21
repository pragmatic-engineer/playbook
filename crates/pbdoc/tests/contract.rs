// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `pbdoc` command line contract: exit codes, the single stderr line and
//! the exact Markdown for each fixture. Every test runs the built binary in its
//! own scratch directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const EXIT_MALFORMED: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_NEEDS_OCR: i32 = 3;
const TRUNCATED_DOCX_BYTES: usize = 512;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
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
    let expected = "anydoc 0.2.4";

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
