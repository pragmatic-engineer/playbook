// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook doc read <file>`: turn a PDF or Office file into Markdown with
//! the optional external `anydoc` converter, locally and under limits, and
//! print it framed as untrusted document content (ADR-0027). Nothing from the
//! document is logged or stored.

pub mod convert;
pub mod frame;
pub mod tmp;

use convert::{DocError, Limits};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const STALE_AFTER: Duration = Duration::from_secs(3600);

/// The limits and converter path from config, with built in fallbacks.
pub struct Settings {
    pub limits: Limits,
    pub anydoc_path: String,
}

fn number(home: &Path, slug: Option<&str>, key: &str) -> u64 {
    crate::config::resolve_valid(key, home, slug)
        .ok()
        .and_then(|(v, _, _)| v.as_u64())
        .or_else(|| crate::config::keys::default_value(key).and_then(|v| v.as_u64()))
        .unwrap_or(1)
}

pub fn settings(home: &Path) -> Settings {
    let slug = crate::common::repo_slug();
    let slug = (!slug.is_empty()).then_some(slug);
    let slug = slug.as_deref();
    let anydoc_path = crate::config::resolve_valid("docs.anydocPath", home, slug)
        .ok()
        .and_then(|(v, _, _)| v.as_str().map(str::to_string))
        .unwrap_or_default();
    Settings {
        limits: Limits {
            max_input_bytes: number(home, slug, "docs.maxBytes"),
            timeout: Duration::from_secs(number(home, slug, "docs.timeoutSeconds")),
            max_output_chars: number(home, slug, "docs.maxOutputChars") as usize,
            max_rss_kb: number(home, slug, "docs.maxMemoryMb") * 1024,
        },
        anydoc_path,
    }
}

/// The scratch root: `tmp` under the playbook state directory.
pub fn scratch_root(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home).join("tmp")
}

/// Run `playbook doc read`. Returns the process exit code: 0 on success, 3
/// when the document needs OCR, 1 for every other failure.
pub fn read(home: &Path, file: &Path, json: bool) -> i32 {
    let cfg = settings(home);
    let root = scratch_root(home);
    tmp::sweep_stale(&root, STALE_AFTER);
    convert::install_signal_handlers();

    let result = run(&cfg, &root, file);
    match result {
        Ok((text, c, format)) => {
            if json {
                let shown = c.markdown.chars().count();
                let v: Value = json!({
                    "status": "ok",
                    "format": format,
                    "chars": shown,
                    "total_chars": c.total_chars,
                    "tokens_estimate": frame::token_estimate(shown),
                    "truncated": c.truncated,
                    "converter_version": c.version,
                    "isolation": c.isolation.label(),
                    "not_represented": frame::limitations(format),
                    "content": text,
                });
                println!("{v}");
            } else {
                print!("{text}");
            }
            0
        }
        Err(err) => {
            let code = match err {
                DocError::NeedsOcr => 3,
                DocError::Interrupted(sig) => 128 + sig,
                _ => 1,
            };
            if json {
                println!(
                    "{}",
                    json!({"status": err.code(), "message": err.message()})
                );
            } else {
                eprintln!("doc read: {}", err.message());
            }
            code
        }
    }
}

fn run(
    cfg: &Settings,
    root: &Path,
    file: &Path,
) -> Result<(String, convert::Converted, &'static str), DocError> {
    let format = convert::format_of(file)?;
    let path_var = std::env::var_os("PATH");
    let exe = std::env::current_exe().ok();
    let anydoc = convert::find_converter(&cfg.anydoc_path, exe.as_deref(), path_var.as_deref())
        .ok_or(DocError::Absent)?;
    let c = convert::convert(&anydoc, file, &cfg.limits, root)?;
    let id = tmp::random_hex(12);
    let source = frame::safe_name(file);
    let text = frame::wrap(
        &c,
        &frame::Meta {
            source: &source,
            format,
            id: &id,
        },
    );
    Ok((text, c, format))
}

/// `playbook doc status`: whether the converter is available.
pub fn status(home: &Path) -> i32 {
    let cfg = settings(home);
    let path_var = std::env::var_os("PATH");
    let exe = std::env::current_exe().ok();
    let Some(found) =
        convert::find_converter(&cfg.anydoc_path, exe.as_deref(), path_var.as_deref())
    else {
        println!(
            "{}{}",
            convert::STATUS_NOT_FOUND_PREFIX,
            convert::INSTALL_HINT
        );
        return 1;
    };
    let version = convert::version_of(&found.path, &std::env::temp_dir());
    match found.kind {
        convert::Kind::Bundled => {
            print_bundled(&found.path, &version, path_var.as_deref());
        }
        convert::Kind::External => {
            print_external(&found.path, &version, exe.as_deref());
        }
    }
    0
}

fn print_bundled(path: &Path, version: &str, path_var: Option<&std::ffi::OsStr>) {
    let own = convert::pbdoc_number(version);
    println!("{}{}", convert::STATUS_BUNDLED_PREFIX, path.display());
    println!("{}{}", convert::STATUS_VERSION_PREFIX, own);
    println!("{}", convert::STATUS_ANYDOC_LINE);
    if own != env!("CARGO_PKG_VERSION") {
        println!(
            "{}",
            convert::version_skew_note(own, env!("CARGO_PKG_VERSION"))
        );
    }
    if path_var.and_then(convert::find_on_path).is_some() {
        println!("{}", convert::STATUS_NPM_NOTE);
    }
}

fn print_external(path: &Path, version: &str, exe: Option<&Path>) {
    println!("{}{}", convert::STATUS_EXTERNAL_PREFIX, path.display());
    println!(
        "{}{}",
        convert::STATUS_EXTERNAL_VERSION_PREFIX,
        if version.is_empty() {
            "unknown"
        } else {
            version
        }
    );
    if !version.is_empty() && !version.starts_with(convert::TESTED_VERSION) {
        println!(
            "note: playbook was tested with {}.x, other versions may behave differently",
            convert::TESTED_VERSION
        );
    }
    if exe.and_then(convert::find_bundled).is_some() {
        println!("{}", convert::STATUS_OVERRIDE_NOTE);
    }
}
