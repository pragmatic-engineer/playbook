// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The Burn-up panel: cumulative spend over the billing period as filled
//! columns, a dotted projection to the period end, horizontal lines at 50,
//! 80 and 100 percent of the limit, and markers where those are crossed. Every
//! line has its own glyph, so the chart reads the same without color. Under
//! `MIN_CHART_WIDTH` columns, or without room for a few rows, the panel shows
//! the plain table instead.

use super::budget;
use super::data::Data;
use super::fmt::{axis_money, fit};
use super::theme::{Ink, Palette};
use crate::burnup::Burnup;
use crate::limits::{CrossState, Limit, Unit};
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
const HALF: f64 = 50.0;
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
    let first_day = b.zone.day_of(b.start);
    for (col, at) in times.iter().enumerate().filter(|(_, at)| **at <= b.now) {
        let day = (b.zone.day_of(*at) - first_day).max(0) as usize;
        if let Some(v) = b.actual.get(day) {
            for row in grid.iter_mut().skip(plot.row(*v)) {
                row[col] = Role::Actual;
            }
        }
    }
    if let Some(limit) = b.limit {
        for (pct, role) in [
            (HALF, Role::Half),
            (NEAR, Role::Near),
            (PERCENT, Role::Limit),
        ] {
            let row = plot.row(limit * pct / PERCENT);
            grid[row].fill(role);
        }
    }
    for (col, at) in times.iter().enumerate().filter(|(_, at)| **at > b.now) {
        if let Some(v) = b.projected_at(*at) {
            grid[plot.row(v)][col] = Role::Projection;
        }
    }
    if let (Some(limit), true) = (b.limit, !b.crossings.is_empty()) {
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

/// `MM-DD` of a second, in the chosen zone.
fn short(at: i64) -> String {
    budget::stamp(at)[..5].to_string()
}

/// The crossings as one line: `50% 01-11  80% 01-17  100% beyond`.
fn crossing_line(b: &Burnup) -> String {
    if b.crossings.is_empty() {
        return "no limit known: set usage.budget.monthlyUsd".to_string();
    }
    b.crossings
        .iter()
        .map(|c| match c.state {
            CrossState::Crossed { at } => format!("{}% {} {MARKER}", c.pct, short(at)),
            CrossState::Projected { at } => format!("{}% ~{} {MARKER}", c.pct, short(at)),
            CrossState::Beyond { .. } => format!("{}% beyond", c.pct),
            CrossState::Unknown => format!("{}% too early", c.pct),
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// The window percentages, when any window is known.
fn windows_line(limits: &[Limit]) -> Option<String> {
    let parts: Vec<String> = limits
        .iter()
        .filter(|l| l.unit == Unit::Percent)
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
    limits: &[Limit],
    pal: &Palette,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let windows = windows_line(limits);
    let legend_rows = 2 + usize::from(windows.is_some());
    let plot = Plot::new(b, width - GUTTER, rows - 1 - legend_rows);
    let grid = cells(b, &plot);
    let mut out = Vec::new();
    for (i, row) in grid.iter().enumerate() {
        let label = if i == 0 {
            axis_money(plot.top)
        } else if i == plot.height - 1 {
            axis_money(0.0)
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
    let (first, last) = (short(b.start), short(b.end - 1));
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
        fit(
            &format!("{ACTUAL} spent  {PROJECTION} projected  {LIMIT_LINE} limit  {BILLED} billed by the server"),
            width,
        ),
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
                    format!("spend this period, {}", crate::tz::label()),
                    style(pal, pal.fg),
                ),
                Span::styled("\u{251c}\u{2500}", edge),
            ])
            .right_aligned(),
        );
    let inner = block.inner(area);
    let (width, rows) = (usize::from(inner.width), usize::from(inner.height));
    let lines = match b {
        None => vec![Line::styled("no burn-up yet", style(pal, pal.inactive))],
        Some(b) if b.actual.is_empty() => {
            vec![Line::styled(
                "the period has not started",
                style(pal, pal.inactive),
            )]
        }
        Some(b)
            if inner.width >= MIN_CHART_WIDTH
                && rows >= MIN_PLOT_ROWS + 4 + usize::from(windows_line(limits).is_some()) =>
        {
            chart_lines(b, limits, pal, width, rows)
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
    render_burnup(
        frame,
        area,
        data.limits.burnup.as_ref(),
        &data.limits.limits,
        pal,
    );
}
