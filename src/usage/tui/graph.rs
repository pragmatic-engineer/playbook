// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A braille graph like btop's: two data points per cell column, four dot rows
//! per cell, each column colored by its value along the theme's gradient.

use super::theme::{Fill, Palette};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Braille dot bits for the rows of one cell, bottom first, for the left and
/// the right column.
const LEFT: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
const RIGHT: [u32; 4] = [0x80, 0x20, 0x10, 0x08];

/// Spreads `values` over `points` slots: a short series repeats each value so
/// it fills the width, a long one keeps its newest values.
pub fn resample(values: &[f64], points: usize) -> Vec<f64> {
    if values.is_empty() || points == 0 {
        return Vec::new();
    }
    let each = (points / values.len()).max(1);
    let mut out: Vec<f64> = values
        .iter()
        .flat_map(|v| std::iter::repeat_n(*v, each))
        .collect();
    out.drain(..out.len().saturating_sub(points));
    out
}

/// The braille character for one cell, given how many of its four dot rows
/// each column fills from the bottom.
fn cell(left: usize, right: usize) -> char {
    let mut bits = 0;
    for row in 0..4 {
        if left > row {
            bits |= LEFT[row];
        }
        if right > row {
            bits |= RIGHT[row];
        }
    }
    char::from_u32(0x2800 + bits).unwrap_or(' ')
}

/// Dot rows a value fills in a graph `rows` cells high. A value above zero
/// always shows at least one dot.
fn dots(value: f64, peak: f64, rows: usize) -> usize {
    if peak <= 0.0 || value <= 0.0 {
        return 0;
    }
    let total = rows * 4;
    (((value / peak) * total as f64).round() as usize).clamp(1, total)
}

/// Draws `values` (oldest first, drawn from the left) into `area`.
pub fn render(buf: &mut Buffer, area: Rect, values: &[f64], fill: Fill, pal: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = usize::from(area.height);
    let points = resample(values, usize::from(area.width) * 2);
    let peak = points.iter().copied().fold(0.0, f64::max);
    for cx in 0..usize::from(area.width) {
        let at = |i: usize| points.get(i).copied().unwrap_or(0.0);
        let (l, r) = (at(cx * 2), at(cx * 2 + 1));
        let pct = if peak > 0.0 {
            l.max(r) / peak * 100.0
        } else {
            0.0
        };
        let color = pal.graded(fill, pct);
        let (ld, rd) = (dots(l, peak, rows), dots(r, peak, rows));
        for cy in 0..rows {
            // Row 0 is the bottom cell.
            let below = cy * 4;
            let ch = cell(
                ld.saturating_sub(below).min(4),
                rd.saturating_sub(below).min(4),
            );
            if ch != '\u{2800}' {
                let x = area.x + cx as u16;
                let y = area.y + area.height - 1 - cy as u16;
                buf[(x, y)]
                    .set_char(ch)
                    .set_style(Style::default().fg(color));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::tui::theme::{ColorDepth, Theme};
    use ratatui::style::Color;

    #[test]
    fn resample_repeats_a_short_series_and_keeps_the_newest_of_a_long_one() {
        assert_eq!(resample(&[1.0, 2.0], 6), [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        assert_eq!(resample(&[1.0, 2.0, 3.0], 2), [2.0, 3.0]);
        assert!(resample(&[], 4).is_empty());
    }

    #[test]
    fn a_cell_lights_dots_from_the_bottom() {
        assert_eq!(cell(0, 0), '\u{2800}');
        assert_eq!(cell(1, 0), '\u{2840}');
        assert_eq!(cell(4, 4), '\u{28ff}');
        assert_eq!(cell(2, 1), '\u{28c4}');
    }

    #[test]
    fn a_nonzero_value_always_shows_a_dot() {
        assert_eq!(dots(0.001, 100.0, 2), 1);
        assert_eq!(dots(100.0, 100.0, 2), 8);
        assert_eq!(dots(0.0, 100.0, 2), 0);
        assert_eq!(dots(5.0, 0.0, 2), 0);
    }

    #[test]
    fn columns_take_their_color_from_the_gradient() {
        let pal = Theme::Btop.palette(ColorDepth::True);
        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        // Two cells: a low value then the peak.
        render(&mut buf, area, &[1.0, 1.0, 100.0, 100.0], pal.cpu, &pal);
        let low = buf[(0, 1)].fg;
        let high = buf[(1, 1)].fg;
        assert_ne!(low, high);
        assert_eq!(high, Color::Rgb(0xdc, 0x4c, 0x4c));
        // The peak column fills both rows, the low one only its bottom row.
        assert_eq!(buf[(1, 0)].symbol(), "\u{28ff}");
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
