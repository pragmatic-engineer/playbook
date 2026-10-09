// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Server-rendered inline SVG bar charts. Self-contained on purpose: no
//! external resources, no scripts, every label escaped, so the page needs no
//! charting library and a hostile repo or branch name cannot inject markup.

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 240.0;
const LEFT: f64 = 88.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 28.0;
const BOTTOM: f64 = 48.0;
const MAX_LABELLED_BARS: usize = 12;
const MAX_LABEL_CHARS: usize = 14;

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `claude-haiku-4-5-20251001` shows as `haiku-4-5` on an axis: the vendor
/// prefix and a trailing eight digit date add nothing there. The tooltip keeps
/// the full name.
fn display_label(label: &str) -> String {
    let name = label.strip_prefix("claude-").unwrap_or(label);
    match name.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => name.to_string(),
    }
}

fn short(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        return label.to_string();
    }
    let kept: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
    format!("{kept}…")
}

/// One bar per `(label, value)`, scaled to the largest value. Colours come
/// from CSS variables the page defines, so the chart follows light and dark.
pub fn bar_chart(title: &str, unit: &str, bars: &[(String, f64)]) -> String {
    let max = bars.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let plot_w = WIDTH - LEFT - RIGHT;
    let plot_h = HEIGHT - TOP - BOTTOM;
    let slot = if bars.is_empty() {
        plot_w
    } else {
        plot_w / bars.len() as f64
    };
    let bar_w = (slot * 0.7).max(1.0);

    let mut svg = format!(
        "<svg viewBox=\"0 0 {WIDTH} {HEIGHT}\" role=\"img\" aria-label=\"{}\" class=\"chart\">",
        escape(title)
    );
    svg.push_str(&format!(
        "<text x=\"{LEFT}\" y=\"16\" class=\"chart-title\">{} ({})</text>",
        escape(title),
        escape(unit)
    ));
    svg.push_str(&format!(
        "<line x1=\"{LEFT}\" y1=\"{y}\" x2=\"{x2}\" y2=\"{y}\" class=\"axis\"/>",
        y = TOP + plot_h,
        x2 = WIDTH - RIGHT
    ));
    svg.push_str(&format!(
        "<text x=\"{x}\" y=\"{y}\" class=\"tick\" text-anchor=\"end\">{max:.2}</text>",
        x = LEFT - 6.0,
        y = TOP + 4.0
    ));
    for (i, (label, value)) in bars.iter().enumerate() {
        let h = if max > 0.0 { plot_h * value / max } else { 0.0 };
        let x = LEFT + slot * i as f64 + (slot - bar_w) / 2.0;
        let y = TOP + plot_h - h;
        svg.push_str(&format!(
            "<rect class=\"bar\" x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bar_w:.1}\" height=\"{h:.1}\"><title>{}: {value:.4}</title></rect>",
            escape(label)
        ));
        if bars.len() <= MAX_LABELLED_BARS || i % (bars.len() / MAX_LABELLED_BARS + 1) == 0 {
            svg.push_str(&format!(
                "<text class=\"tick\" x=\"{cx:.1}\" y=\"{ty}\" text-anchor=\"end\" transform=\"rotate(-35 {cx:.1} {ty})\">{}</text>",
                escape(&short(&display_label(label))),
                cx = x + bar_w / 2.0,
                ty = TOP + plot_h + 14.0
            ));
        }
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Vec<(String, f64)> {
        vec![("a".into(), 1.0), ("b".into(), 2.0), ("c".into(), 0.0)]
    }

    #[test]
    fn draws_one_rect_per_bar_scaled_to_the_largest() {
        let svg = bar_chart("Cost", "USD", &bars());

        assert_eq!(svg.matches("<rect").count(), 3);
        // Plot height is 240 - 28 - 48 = 164: the largest bar fills it, the
        // half-value bar is half as tall, the zero bar has no height.
        assert!(svg.contains("height=\"164.0\""));
        assert!(svg.contains("height=\"82.0\""));
        assert!(svg.contains("height=\"0.0\""));
    }

    #[test]
    fn is_self_contained() {
        let svg = bar_chart("Cost", "USD", &bars());

        for forbidden in [
            "http://", "https://", "href", "src=", "<script", "url(", "xlink", " onload",
            " onerror",
        ] {
            assert!(!svg.contains(forbidden), "found {forbidden} in {svg}");
        }
    }

    #[test]
    fn escapes_hostile_labels() {
        let hostile = vec![("<img src=x onerror=alert(1)>".to_string(), 1.0)];

        let svg = bar_chart("t\"itle", "USD", &hostile);

        assert!(!svg.contains("<img"));
        assert!(svg.contains("&lt;img"));
        assert!(svg.contains("t&quot;itle"));
    }

    #[test]
    fn model_names_lose_the_vendor_prefix_and_trailing_date_on_the_axis() {
        for (given, shown) in [
            ("claude-haiku-4-5-20251001", "haiku-4-5"),
            ("claude-opus-5-5", "opus-5-5"),
            ("claude-sonnet-5", "sonnet-5"),
            ("<synthetic>", "<synthetic>"),
            ("2026-09-30", "2026-09-30"),
            ("claude-opus-4-8", "opus-4-8"),
        ] {
            assert_eq!(display_label(given), shown, "{given}");
        }
    }

    #[test]
    fn the_axis_shows_the_short_name_and_the_tooltip_the_full_one() {
        let model = vec![("claude-haiku-4-5-20251001".to_string(), 3.0)];

        let svg = bar_chart("Cost per model", "USD", &model);

        assert!(svg.contains(">haiku-4-5</text>"), "{svg}");
        assert!(svg.contains("<title>claude-haiku-4-5-20251001: 3.0000</title>"));
        // The text of every `<text ...>X</text>` element, nothing else.
        let texts: Vec<&str> = svg
            .split("</text>")
            .filter_map(|chunk| chunk.rsplit_once('>').map(|(_, text)| text))
            .collect();
        assert!(texts.contains(&"haiku-4-5"), "{texts:?}");
        assert!(
            texts.iter().all(|text| !text.contains("claude-")),
            "no axis text may show the long name: {texts:?}"
        );
    }

    #[test]
    fn the_title_reads_name_then_one_parenthesised_unit_note() {
        let svg = bar_chart("Cost per day", "USD, UTC", &bars());

        assert!(svg.contains(">Cost per day (USD, UTC)</text>"), "{svg}");
        assert!(!svg.contains(") ("), "no doubled parentheses: {svg}");
    }

    #[test]
    fn an_empty_or_all_zero_chart_does_not_divide_by_zero() {
        assert_eq!(bar_chart("Cost", "USD", &[]).matches("<rect").count(), 0);
        let zero = bar_chart("Cost", "USD", &[("a".into(), 0.0)]);
        assert!(zero.contains("height=\"0.0\"") && !zero.contains("NaN"));
    }
}
