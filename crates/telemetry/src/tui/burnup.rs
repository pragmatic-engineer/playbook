// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The Burn-up panel: how much of one limit is used over its period as filled
//! columns, a dotted projection to the period end, horizontal lines at 50, 80
//! and 100 percent of the limit, and markers where those are crossed. A dollar
//! limit follows the local cost per day. A percent window follows the
//! readings, or a straight line when there are none. Every line has its own
//! glyph, so the chart reads the same without color. Under `MIN_CHART_WIDTH`
//! columns, or without room for a few rows, the panel shows the plain table
//! instead.

use super::budget;
use super::data::Data;
use super::fmt::{axis_money, fit};
use super::theme::{Ink, Palette};
use crate::burnup::Basis;
use crate::burnup::Burnup;
use crate::limits::{CrossState, Limit, LimitPeriod, Unit};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

/// Panel columns under which the table replaces the chart.
pub const MIN_CHART_WIDTH: u16 = 40;
/// The fewest plot rows worth drawing.
const MIN_PLOT_ROWS: usize = 4;
/// Columns left of the plot for the dollar labels.
const GUTTER: usize = 8;
/// The plot reaches this far above the highest value.
const HEADROOM: f64 = 1.05;
/// The thresholds drawn, in percent of the limit.
const NEAR: f64 = 80.0;
const PERCENT: f64 = 100.0;

const ACTUAL: char = '\u{2588}';
const PROJECTION: char = '\u{00b7}';
const HALF_LINE: char = '\u{2504}';
const NEAR_LINE: char = '\u{2508}';
const LIMIT_LINE: char = '\u{2501}';
const BILLED: char = '\u{25c6}';
const MARKER: char = '\u{25b2}';

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Empty,
    Actual,
    Projection,
    Half,
    Near,
    Limit,
    Billed,
    Marker,
}

impl Role {
    fn glyph(self) -> char {
        match self {
            Role::Empty => ' ',
            Role::Actual => ACTUAL,
            Role::Projection => PROJECTION,
            Role::Half => HALF_LINE,
            Role::Near => NEAR_LINE,
            Role::Limit => LIMIT_LINE,
            Role::Billed => BILLED,
            Role::Marker => MARKER,
        }
    }

    fn ink(self, pal: &Palette) -> Ink {
        match self {
            Role::Empty | Role::Half | Role::Near => pal.inactive,
            Role::Actual => pal.live,
            Role::Projection => pal.warn,
            Role::Limit => pal.error,
            Role::Billed => pal.hi,
            Role::Marker => pal.title,
        }
    }
}

/// An axis label in the unit of the chart.
fn axis(b: &Burnup, value: f64) -> String {
    match b.unit {
        Unit::Dollars => axis_money(value),
        Unit::Percent => format!("{value:.0}%"),
    }
}

/// What the chart is of, for the title.
fn subject(b: &Burnup, zone_label: &str) -> String {
    match b.unit {
        Unit::Dollars => format!("spend this period, {zone_label}"),
        Unit::Percent => format!(
            "{} used, {zone_label}",
            match b.period {
                LimitPeriod::FiveHour => "5 hour window",
                LimitPeriod::SevenDay => "weekly window",
                _ => "window",
            }
        ),
    }
}

/// Table lines that are not days: the header and the three crossings.
const TABLE_CHROME: usize = 4;

/// The scale and size of one plot.
struct Plot {
    width: usize,
    height: usize,
    top: f64,
}

impl Plot {
    fn new(b: &Burnup, width: usize, height: usize) -> Plot {
        let top = [
            b.limit,
            b.projected_end,
            Some(b.total),
            b.billed.map(|(_, usd)| usd),
        ]
        .into_iter()
        .flatten()
        .fold(0.0, f64::max)
            * HEADROOM;
        Plot {
            width,
            height,
            top: if top > 0.0 { top } else { 1.0 },
        }
    }

    /// The row of `usd`, the top row being 0.
    fn row(&self, usd: f64) -> usize {
        let from_bottom = (usd / self.top * (self.height - 1) as f64).round().max(0.0) as usize;
        (self.height - 1).saturating_sub(from_bottom.min(self.height - 1))
    }

    fn col(&self, b: &Burnup, at: i64) -> usize {
        let share = (at - b.start) as f64 / (b.end - b.start) as f64;
        ((share * (self.width - 1) as f64).round().max(0.0) as usize).min(self.width - 1)
    }

    fn time(&self, b: &Burnup, col: usize) -> i64 {
        b.start + ((b.end - b.start) as f64 * col as f64 / (self.width - 1) as f64) as i64
    }
}

/// The plot as rows of roles, top row first.
fn cells(b: &Burnup, plot: &Plot) -> Vec<Vec<Role>> {
    let mut grid = vec![vec![Role::Empty; plot.width]; plot.height];
    let times: Vec<i64> = (0..plot.width).map(|col| plot.time(b, col)).collect();
    for (col, at) in times.iter().enumerate().filter(|(_, at)| **at <= b.now) {
        if let Some(v) = value_at(b, *at) {
            for row in grid.iter_mut().skip(plot.row(v)) {
                row[col] = Role::Actual;
            }
        }
    }
    for (pct, value) in b.cutoffs() {
        let role = if pct >= PERCENT {
            Role::Limit
        } else if pct >= NEAR {
            Role::Near
        } else {
            Role::Half
        };
        grid[plot.row(value)].fill(role);
    }
    for (col, at) in times.iter().enumerate().filter(|(_, at)| **at > b.now) {
        if let Some(v) = b.projected_at(*at) {
            grid[plot.row(v)][col] = Role::Projection;
        }
    }
    if let Some(limit) = b.limit {
        for c in &b.crossings {
            if let CrossState::Crossed { at } | CrossState::Projected { at } = c.state {
                grid[plot.row(limit * c.pct / PERCENT)][plot.col(b, at)] = Role::Marker;
            }
        }
    }
    if let Some((at, usd)) = b.billed {
        grid[plot.row(usd)][plot.col(b, at)] = Role::Billed;
    }
    grid
}

/// What is used at `at`: the day's total for a dollar limit, the line
/// between the points for a percent window. `None` before the first point.
fn value_at(b: &Burnup, at: i64) -> Option<f64> {
    if b.basis == Basis::Daily {
        let day = (b.zone.day_of(at) - b.zone.day_of(b.start)).max(0) as usize;
        return b.actual.get(day).copied();
    }
    let next = b.points.iter().position(|(t, _)| *t > at);
    match next {
        None => b.points.last().map(|(_, v)| *v),
        Some(0) => None,
        Some(i) => {
            let ((t0, v0), (t1, v1)) = (b.points[i - 1], b.points[i]);
            let share = (at - t0) as f64 / (t1 - t0) as f64;
            Some(v0 + (v1 - v0) * share)
        }
    }
}

/// `MM-DD` of a second for a period of days or a window longer than a day,
/// `HH:MM` for a window of a day or less, in the zone of the chart.
fn short(b: &Burnup, at: i64) -> String {
    let iso = b.zone.iso(at);
    match b.unit {
        Unit::Dollars => iso[5..10].to_string(),
        Unit::Percent if b.end - b.start > 86_400 => iso[5..10].to_string(),
        Unit::Percent => iso[11..16].to_string(),
    }
}

/// The crossings as one line: `50% 01-11  80% 01-17  100% beyond`.
fn crossing_line(b: &Burnup) -> String {
    if b.crossings.is_empty() {
        return "no limit known".to_string();
    }
    b.crossings
        .iter()
        .map(|c| match c.state {
            CrossState::Crossed { at } => format!("{}% {} {MARKER}", c.pct, short(b, at)),
            CrossState::Projected { at } => format!("{}% ~{} {MARKER}", c.pct, short(b, at)),
            CrossState::Beyond { .. } => format!("{}% beyond", c.pct),
            CrossState::Unknown => format!("{}% too early", c.pct),
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// What each glyph draws.
fn legend(b: &Burnup) -> String {
    let used = match b.unit {
        Unit::Dollars => "spent",
        Unit::Percent => "used",
    };
    let mut parts = vec![
        format!("{ACTUAL} {used}"),
        format!("{PROJECTION} projected"),
    ];
    if b.limit.is_some() {
        parts.push(format!(
            "{LIMIT_LINE} limit  {NEAR_LINE} 80%  {HALF_LINE} 50%"
        ));
        parts.push(format!("{MARKER} crossing"));
    }
    if b.billed.is_some() {
        parts.push(format!("{BILLED} reported by Claude"));
    }
    parts.join("  ")
}

/// The window percentages, when any window is known.
fn windows_line(limits: &[Limit]) -> Option<String> {
    let parts: Vec<String> = limits
        .iter()
        .filter(|l| {
            l.unit == Unit::Percent
                && matches!(l.period, LimitPeriod::FiveHour | LimitPeriod::SevenDay)
        })
        .map(|l| format!("{} {:.0}%", budget::name(l), l.used))
        .collect();
    (!parts.is_empty()).then(|| parts.join("  "))
}

fn style(pal: &Palette, ink: Ink) -> Style {
    Style::default().fg(pal.color(ink))
}

/// The chart lines for a plot area `width` columns wide and `rows` tall,
/// including the date axis and the legend.
fn chart_lines(
    b: &Burnup,
    pal: &Palette,
    width: usize,
    rows: usize,
    windows: Option<String>,
) -> Vec<Line<'static>> {
    let legend_rows = 2 + usize::from(windows.is_some());
    let plot = Plot::new(b, width - GUTTER, rows - 1 - legend_rows);
    let grid = cells(b, &plot);
    let mut out = Vec::new();
    for (i, row) in grid.iter().enumerate() {
        let label = if i == 0 {
            axis(b, plot.top)
        } else if i == plot.height - 1 {
            axis(b, 0.0)
        } else {
            String::new()
        };
        let mut spans = vec![Span::styled(
            format!("{label:>w$} ", w = GUTTER - 1),
            style(pal, pal.inactive),
        )];
        spans.extend(
            row.iter()
                .map(|r| Span::styled(r.glyph().to_string(), style(pal, r.ink(pal)))),
        );
        out.push(Line::from(spans));
    }
    let (first, last) = (short(b, b.start), short(b, b.end - 1));
    let gap = plot.width.saturating_sub(first.len() + last.len());
    out.push(Line::styled(
        format!("{:>w$}{first}{}{last}", "", " ".repeat(gap), w = GUTTER),
        style(pal, pal.inactive),
    ));
    out.push(Line::styled(
        fit(&crossing_line(b), width),
        style(pal, pal.fg),
    ));
    out.push(Line::styled(
        fit(&legend(b), width),
        style(pal, pal.inactive),
    ));
    if let Some(w) = windows {
        out.push(Line::styled(fit(&w, width), style(pal, pal.inactive)));
    }
    out
}

/// Draws the panel for one burn-up and the limits shown beside it.
pub(super) fn render_burnup(
    frame: &mut Frame,
    area: Rect,
    b: Option<&Burnup>,
    limits: &[Limit],
    pal: &Palette,
) {
    if area.height == 0 {
        return;
    }
    let zone_label = b.map_or_else(crate::tz::label, |b| b.zone.label());
    let edge = style(pal, pal.box_cpu);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(edge)
        .title(Line::from(vec![
            Span::styled("\u{2500}\u{2524}", edge),
            Span::styled("Burn-up", style(pal, pal.title)),
            Span::styled("\u{251c}", edge),
        ]))
        .title(
            Line::from(vec![
                Span::styled("\u{2524}", edge),
                Span::styled(
                    b.map_or_else(
                        || format!("spend this period, {zone_label}"),
                        |b| subject(b, zone_label),
                    ),
                    style(pal, pal.fg),
                ),
                Span::styled("\u{251c}\u{2500}", edge),
            ])
            .right_aligned(),
        );
    let inner = block.inner(area);
    let (width, rows) = (usize::from(inner.width), usize::from(inner.height));
    let windows = windows_line(limits);
    let lines = match b {
        None => vec![Line::styled("no burn-up yet", style(pal, pal.inactive))],
        Some(b) if b.points.is_empty() => {
            vec![Line::styled(
                "the period has not started",
                style(pal, pal.inactive),
            )]
        }
        Some(b)
            if inner.width >= MIN_CHART_WIDTH
                && rows >= MIN_PLOT_ROWS + 4 + usize::from(windows.is_some()) =>
        {
            chart_lines(b, pal, width, rows, windows)
        }
        Some(b) => b
            .table(rows.saturating_sub(TABLE_CHROME).max(1))
            .into_iter()
            .chain(b.crossing_lines())
            .map(|t| Line::styled(fit(&t, width), style(pal, pal.fg)))
            .collect(),
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub fn render(frame: &mut Frame, area: Rect, data: &Data, pal: &Palette) {
    render_burnup(frame, area, data.limits.charted(), &data.limits.limits, pal);
}
