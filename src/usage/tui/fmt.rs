// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Number and text formatting for the terminal view.

/// `$1,234.56`.
pub fn money(usd: f64) -> String {
    let cents = (usd.abs() * 100.0).round() as u64;
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if usd < 0.0 && cents > 0 { "-" } else { "" };
    format!("{sign}${grouped}.{:02}", cents % 100)
}

/// `950`, `1.2K`, `3.4M`, `5.6B`.
pub fn compact(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];
    for (size, suffix) in UNITS {
        if n >= size {
            return format!("{:.1}{suffix}", n as f64 / size as f64);
        }
    }
    n.to_string()
}

/// A number with at most three decimals and no trailing zeros: `2.5`, `0.05`.
fn trimmed(n: f64) -> String {
    let text = format!("{n:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// An axis label in dollars: `$0`, `$0.25`, `$12.5`, `$250`, `$2.5K`.
pub fn axis_money(usd: f64) -> String {
    format!("${}", axis_number(usd))
}

/// An axis label in tokens: `0`, `250`, `500K`, `2.5M`.
pub fn axis_tokens(n: f64) -> String {
    axis_number(n)
}

fn axis_number(n: f64) -> String {
    const UNITS: [(f64, &str); 3] = [(1e9, "B"), (1e6, "M"), (1e3, "K")];
    for (size, suffix) in UNITS {
        if n >= size * 0.9995 {
            return format!("{}{suffix}", trimmed(n / size));
        }
    }
    if n >= 100.0 {
        format!("{n:.0}")
    } else {
        trimmed(n)
    }
}

/// `Mon` to `Sun` for a weekday index with Monday as 0.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A horizontal bar `width` cells wide, filled to `fraction` (0 to 1).
pub fn bar(fraction: f64, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(width - filled)
    )
}

/// Cuts `text` to `width` characters, ending with `~` when it was cut.
pub fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    if width > 0 {
        out.push('~');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_groups_thousands_and_rounds_cents() {
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(1234.567), "$1,234.57");
        assert_eq!(money(1_000_000.0), "$1,000,000.00");
        assert_eq!(money(-0.001), "$0.00");
        assert_eq!(money(-5.5), "-$5.50");
    }

    #[test]
    fn compact_uses_one_decimal_units() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(1_500), "1.5K");
        assert_eq!(compact(3_400_000), "3.4M");
        assert_eq!(compact(5_600_000_000), "5.6B");
    }

    #[test]
    fn axis_labels_are_short_and_round() {
        for (n, money, tokens) in [
            (0.0, "$0", "0"),
            (0.05, "$0.05", "0.05"),
            (0.25, "$0.25", "0.25"),
            (1.25, "$1.25", "1.25"),
            (12.5, "$12.5", "12.5"),
            (250.0, "$250", "250"),
            (999.9, "$1K", "1K"),
            (2_500.0, "$2.5K", "2.5K"),
            (500_000.0, "$500K", "500K"),
            (2_500_000.0, "$2.5M", "2.5M"),
            (1_000_000_000.0, "$1B", "1B"),
        ] {
            assert_eq!(axis_money(n), money, "{n}");
            assert_eq!(axis_tokens(n), tokens, "{n}");
        }
    }

    #[test]
    fn bar_fills_in_proportion_and_clamps() {
        assert_eq!(bar(0.5, 4), "\u{2588}\u{2588}\u{2591}\u{2591}");
        assert_eq!(bar(2.0, 2), "\u{2588}\u{2588}");
        assert_eq!(bar(-1.0, 2), "\u{2591}\u{2591}");
    }

    #[test]
    fn fit_marks_a_cut() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("abcdefgh", 5), "abcd~");
        assert_eq!(fit("abc", 0), "");
    }
}
