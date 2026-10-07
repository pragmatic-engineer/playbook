// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Pure formatting helpers for the status line, ported from `statusline.sh`.
//! Colours stay as the script's literal `\033[...m` text and are turned into
//! real escapes once, by [`percent_b`], at the very end.

pub const GREEN: &str = "\\033[38;2;166;227;161m";
pub const RED: &str = "\\033[38;2;243;139;168m";
pub const YELLOW: &str = "\\033[38;2;249;226;175m";
pub const ORANGE: &str = "\\033[38;2;250;179;135m";
pub const WHITE: &str = "\\033[38;2;205;214;244m";
pub const TEAL: &str = "\\033[38;2;148;226;213m";
pub const MAUVE: &str = "\\033[38;2;203;166;247m";
pub const DIM: &str = "\\033[38;2;127;132;156m";
pub const RESET: &str = "\\033[0m";

/// Dim pipe between line-2 and line-3 segments.
pub fn sep() -> String {
    format!("{DIM} | {RESET}")
}

/// Bash `${v%%.*}` then arithmetic: the integer part, 0 when empty or not a
/// number.
pub fn int_part(v: &str) -> i64 {
    let head = v.split('.').next().unwrap_or("");
    head.trim().parse().unwrap_or(0)
}

/// A strict integer, as bash arithmetic over a plain number accepts it.
pub fn strict_int(v: &str) -> Option<i64> {
    v.trim().parse().ok()
}

/// What `printf '%.0f'` makes of `v`, 0 for a non-number.
pub fn round0(v: &str) -> String {
    format!("{:.0}", v.trim().parse::<f64>().unwrap_or(0.0))
}

pub fn rl_color(pct: &str) -> &'static str {
    match int_part(pct) {
        p if p > 80 => RED,
        p if p > 50 => YELLOW,
        _ => GREEN,
    }
}

pub fn ctx_color(pct: &str) -> &'static str {
    match int_part(pct) {
        p if p > 80 => RED,
        p if p > 65 => YELLOW,
        _ => GREEN,
    }
}

/// `width` cells, one per `100/width` percent, filled cells rounded to nearest.
pub fn ctx_bar(used: &str, width: i64) -> String {
    let filled = ((int_part(used) * width + 50) / 100).clamp(0, width.max(0));
    let empty = (width - filled).max(0);
    "█".repeat(filled as usize) + &"░".repeat(empty as usize)
}

pub fn fmt_age(s: i64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        let (h, m) = (s / 3600, (s % 3600) / 60);
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h{m}m")
        }
    }
}

pub fn fmt_ago(s: i64) -> String {
    if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86400)
    }
}

pub fn fmt_tokens(n: i64) -> String {
    if n >= 1000 {
        format!("{}k", (n + 500) / 1000)
    } else {
        n.to_string()
    }
}

/// Cache hit ratio as integer percent, `None` when there is no cache activity.
pub fn cache_hit_pct(write: i64, read: i64) -> Option<i64> {
    let total = write + read;
    (total != 0).then(|| (read * 100) / total)
}

pub fn cache_color(pct: i64) -> &'static str {
    if pct >= 80 {
        GREEN
    } else if pct >= 50 {
        YELLOW
    } else {
        RED
    }
}

/// Cost burn rate in $/min to 4 decimals, `None` when there is no wall time.
pub fn cost_per_min(cost: f64, wall_ms: i64) -> Option<String> {
    (wall_ms > 0).then(|| format!("{:.4}", (cost * 60000.0) / wall_ms as f64))
}

/// Gap to the autocompact trigger; `None` while usage is under 50%.
pub fn compact_gap(used: &str, trigger: i64) -> Option<i64> {
    let used = int_part(used);
    (used >= 50).then(|| (trigger - used).max(0))
}

/// Filesystem-safe slug for cache file names.
pub fn cache_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Visible width as the script measures it: real `ESC[..m` sequences are
/// stripped, then characters are counted. The script's own colour text is a
/// literal backslash form, so it is counted too, exactly as in the shell.
pub fn visible_len(s: &str) -> usize {
    let mut n = 0;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' && it.peek() == Some(&'[') {
            let mut look = it.clone();
            look.next();
            while matches!(look.peek(), Some(d) if d.is_ascii_digit() || *d == ';') {
                look.next();
            }
            if look.peek() == Some(&'m') {
                look.next();
                it = look;
                continue;
            }
        }
        n += 1;
    }
    n
}

/// Interprets backslash escapes the way bash's `printf '%b'` does.
pub fn percent_b(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        match c {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'e' | b'E' => out.push(0x1b),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'\\' => out.push(b'\\'),
            b'c' => return out,
            b'0'..=b'7' => {
                let max = if c == b'0' { 3 } else { 2 };
                let mut v = u32::from(c - b'0');
                let mut taken = 0;
                while taken < max && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                    v = v * 8 + u32::from(b[i] - b'0');
                    i += 1;
                    taken += 1;
                }
                out.push(v as u8);
            }
            b'x' => {
                let mut v = 0u32;
                let mut taken = 0;
                while taken < 2 && i < b.len() && b[i].is_ascii_hexdigit() {
                    v = v * 16 + (b[i] as char).to_digit(16).unwrap_or(0);
                    i += 1;
                    taken += 1;
                }
                if taken == 0 {
                    out.extend_from_slice(b"\\x");
                } else {
                    out.push(v as u8);
                }
            }
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctx_bar_fills_to_the_nearest_cell_and_clamps() {
        assert_eq!(ctx_bar("50", 10), "█████░░░░░");
        assert_eq!(ctx_bar("0", 10), "░░░░░░░░░░");
        assert_eq!(ctx_bar("100", 10), "██████████");
        assert_eq!(ctx_bar("250", 10), "██████████");
        assert_eq!(ctx_bar("", 10), "░░░░░░░░░░");
    }

    #[test]
    fn ages_follow_the_script_ladders() {
        assert_eq!(fmt_age(5), "5s");
        assert_eq!(fmt_age(90), "1m");
        assert_eq!(fmt_age(3660), "1h1m");
        assert_eq!(fmt_age(7200), "2h");
        assert_eq!(fmt_ago(0), "0m");
        assert_eq!(fmt_ago(3600), "1h");
        assert_eq!(fmt_ago(172800), "2d");
    }

    #[test]
    fn tokens_round_to_the_nearest_thousand() {
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1000), "1k");
        assert_eq!(fmt_tokens(127600), "128k");
        assert_eq!(fmt_tokens(127400), "127k");
    }

    #[test]
    fn cache_and_cost_helpers_match_the_script() {
        assert_eq!(cache_hit_pct(0, 0), None);
        assert_eq!(cache_hit_pct(50, 150), Some(75));
        assert_eq!(cache_color(80), GREEN);
        assert_eq!(cache_color(60), YELLOW);
        assert_eq!(cache_color(49), RED);
        assert_eq!(cost_per_min(0.0, 0), None);
        assert_eq!(cost_per_min(1.0, 60000).as_deref(), Some("1.0000"));
    }

    #[test]
    fn compact_gap_is_empty_under_fifty_and_clamped_at_zero() {
        assert_eq!(compact_gap("30", 90), None);
        assert_eq!(compact_gap("80", 90), Some(10));
        assert_eq!(compact_gap("95", 90), Some(0));
    }

    #[test]
    fn slug_replaces_every_unsafe_character() {
        assert_eq!(cache_slug("a/b c"), "a_b_c");
    }

    #[test]
    fn visible_len_strips_real_escapes_but_counts_literal_ones() {
        assert_eq!(visible_len("\x1b[1;32mabc\x1b[0m"), 3);
        assert_eq!(visible_len("\\033[0mab"), 9);
        assert_eq!(visible_len("█░"), 2);
    }

    #[test]
    fn percent_b_expands_escapes_like_bash() {
        assert_eq!(percent_b("\\033[0m"), b"\x1b[0m");
        assert_eq!(percent_b("\\033]8;;u\\033\\\\x"), b"\x1b]8;;u\x1b\\x");
        assert_eq!(percent_b("a\\nb\\q\\"), b"a\nb\\q\\");
        assert_eq!(percent_b("\\x41\\0101"), b"AA");
    }
}
