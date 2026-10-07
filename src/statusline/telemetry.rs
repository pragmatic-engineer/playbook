// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Per-render telemetry sample and the once-per-crossing capture latch. Every
//! write is best effort: a failure here never stops the status line printing.

use super::fmt::int_part;
use std::io::Write;
use std::path::Path;

fn or_zero(v: &str) -> &str {
    if v.is_empty() {
        "0"
    } else {
        v
    }
}

/// Appends one sample and updates the `capture-due` / `capture-fired` /
/// `capture-crossings` files under `<home>/.config/playbook/runtime/<sid>`.
pub fn record(home: &str, sid: &str, cost: &str, used: &str, capture_at: i64, now: i64) {
    let dir = Path::new(home).join(".config/playbook/runtime").join(sid);
    let _ = std::fs::create_dir_all(&dir);
    if !dir.is_dir() || !is_writable(&dir) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("telemetry.jsonl"))
    {
        let _ = writeln!(
            f,
            "{{\"ts\":{now},\"cost_usd\":{},\"used_pct\":{}}}",
            or_zero(cost),
            or_zero(used)
        );
    }
    let fired = dir.join("capture-fired");
    if int_part(used) >= capture_at {
        if !fired.is_file() {
            let _ = std::fs::write(dir.join("capture-due"), "");
            let _ = std::fs::write(&fired, "");
            let crossings = std::fs::read_to_string(dir.join("capture-crossings"))
                .ok()
                .map(|s| s.trim_end_matches('\n').to_string())
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let _ = std::fs::write(dir.join("capture-crossings"), (crossings + 1).to_string());
        }
    } else {
        let _ = std::fs::remove_file(&fired);
    }
}

/// `[[ -w dir ]]`: true when the effective user may write into `dir`.
fn is_writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}
