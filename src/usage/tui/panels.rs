// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The six panels: spend and token graphs, the model and project breakdowns,
//! the live sessions and the recent events. Each is a rounded box with its
//! key hint and title in the top border, like btop.

use super::app::App;
use super::data::Data;
use super::fmt::{compact, fit, money};
use super::graph;
use super::sort::{event_rows, group_rows, group_tokens, session_rows, Panel, Sort};
use super::theme::{Fill, Ink, Palette};
use super::view::{visible_rows, window_start};
use crate::usage::aggregate::{date_key, Group};
use crate::usage::query::{clock, total_tokens, ActiveSession, ACTIVE_SECONDS};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table};
use ratatui::Frame;

/// A session whose last message is newer than this is working right now.
const LIVE_SECONDS: i64 = 120;

/// Meter cell: filled, and the empty rest.
const METER: &str = "\u{25a0}";
const METER_EMPTY: &str = "\u{b7}";

fn fg(pal: &Palette, ink: Ink) -> Style {
    Style::default().fg(pal.color(ink))
}

/// The selected row: the theme's selection colors, or reverse video in mono.
fn selected_style(pal: &Palette) -> Style {
    if pal.mono {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .fg(pal.color(pal.sel_fg))
            .bg(pal.color(pal.sel_bg))
            .add_modifier(Modifier::BOLD)
    }
}

/// `┤¹Title├` the way btop writes a box title, with the panel's key hint in the
/// accent color. The focused panel's hint is a badge.
fn title_line(
    pal: &Palette,
    panel: Panel,
    text: &str,
    box_ink: Ink,
    focused: bool,
) -> Line<'static> {
    let edge = fg(pal, box_ink);
    let hint = if focused {
        let badge = Style::default().add_modifier(Modifier::BOLD);
        if pal.mono {
            badge.add_modifier(Modifier::REVERSED)
        } else {
            badge.fg(pal.color(pal.title)).bg(pal.color(pal.hi))
        }
    } else {
        fg(pal, pal.hi)
    };
    let name = fg(pal, pal.title).add_modifier(if focused {
        Modifier::BOLD
    } else {
        Modifier::empty()
    });
    Line::from(vec![
        Span::styled("\u{2500}\u{2524}", edge),
        Span::styled(panel.hint(), hint),
        Span::styled(text.to_string(), name),
        Span::styled("\u{251c}", edge),
    ])
}

/// A note in the border, `┤note├`, for the right of the top border or the
/// bottom border.
fn note_line(pal: &Palette, text: &str, box_ink: Ink) -> Line<'static> {
    let edge = fg(pal, box_ink);
    Line::from(vec![
        Span::styled("\u{2524}", edge),
        Span::styled(text.to_string(), fg(pal, pal.fg)),
        Span::styled("\u{251c}\u{2500}", edge),
    ])
}

fn panel_block(
    pal: &Palette,
    panel: Panel,
    title: &str,
    box_ink: Ink,
    focused: bool,
) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(pal, box_ink))
        .title(title_line(pal, panel, title, box_ink, focused))
}

fn num(text: impl Into<String>) -> Cell<'static> {
    Cell::from(Text::from(text.into()).alignment(Alignment::Right))
}

/// One table column. `min_inner` is the inner panel width below which the
/// column is left out, so a narrow terminal keeps the names readable. A label
/// starting with `>` is right aligned, like its numbers.
struct Col {
    head: &'static str,
    width: Constraint,
    min_inner: u16,
}

const fn col(head: &'static str, width: Constraint, min_inner: u16) -> Col {
    Col {
        head,
        width,
        min_inner,
    }
}

/// The header row: sorted column marked with an arrow, in the title color.
fn head(pal: &Palette, cols: &[Col], keep: &[bool], sort: Sort, sortable: bool) -> Row<'static> {
    let cells = cols
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(i, c)| {
            let (label, right) = match c.head.strip_prefix('>') {
                Some(l) => (l, true),
                None => (c.head, false),
            };
            let mark = match (sortable && sort.col == i, sort.desc) {
                (true, true) => "\u{25bc}",
                (true, false) => "\u{25b2}",
                _ => "",
            };
            let text = format!("{label}{mark}");
            if right {
                num(text)
            } else {
                Cell::from(text)
            }
        });
    Row::new(cells).style(fg(pal, pal.title).add_modifier(Modifier::BOLD))
}

/// Draws a table inside its panel box. `rows` gets the width in cells of every
/// column (0 for a column that is left out) so it can cut text to fit, and
/// returns one cell per column for every row. Only the rows that fit are drawn,
/// scrolled so the selected one is in view.
#[allow(clippy::too_many_arguments)]
fn table<'a>(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    pal: &Palette,
    panel: Panel,
    block: Block<'static>,
    cols: &[Col],
    rows: impl FnOnce(&[usize]) -> Vec<Vec<Cell<'a>>>,
) {
    let inner = area.width.saturating_sub(2);
    let keep: Vec<bool> = cols.iter().map(|c| inner >= c.min_inner).collect();
    let constraints: Vec<Constraint> = cols
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(c, _)| c.width)
        .collect();
    // The table spaces columns by one cell inside its border.
    let shown = Layout::horizontal(&constraints)
        .spacing(1)
        .split(Rect::new(0, 0, inner, 1));
    let mut widths = vec![0; cols.len()];
    let mut next = shown.iter();
    for (i, k) in keep.iter().enumerate() {
        if *k {
            widths[i] = next.next().map_or(0, |r| usize::from(r.width));
        }
    }
    let all = rows(&widths);
    let visible = visible_rows(area);
    let selected = app.selection(panel);
    let start = window_start(selected, visible);
    let focused = app.focus == panel;
    let rows: Vec<Row> = all
        .into_iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, cells)| {
            let row = Row::new(
                cells
                    .into_iter()
                    .zip(&keep)
                    .filter(|(_, k)| **k)
                    .map(|(c, _)| c),
            );
            if focused && i == selected {
                row.style(selected_style(pal))
            } else {
                row
            }
        })
        .collect();
    let header = head(pal, cols, &keep, app.sort(panel), true);
    frame.render_widget(
        Table::new(rows, constraints).header(header).block(block),
        area,
    );
}

fn empty(frame: &mut Frame, area: Rect, pal: &Palette, block: Block<'static>, message: &str) {
    frame.render_widget(
        Paragraph::new(message.to_string())
            .style(fg(pal, pal.inactive))
            .block(block),
        area,
    );
}

/// A meter like btop's: `■` cells colored along the gradient up to `fraction`,
/// the rest dim dots.
fn meter(pal: &Palette, fill: Fill, fraction: f64, width: usize) -> Cell<'static> {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let mut spans = Vec::with_capacity(width);
    for i in 0..width {
        spans.push(if i < filled {
            let pct = (i + 1) as f64 / width as f64 * 100.0;
            Span::styled(METER, Style::default().fg(pal.graded(fill, pct)))
        } else {
            Span::styled(METER_EMPTY, fg(pal, pal.meter_bg))
        });
    }
    Cell::from(Line::from(spans))
}

/// A cost cell tinted by how it compares with the largest one on screen.
fn cost_cell(pal: &Palette, cost: f64, peak: f64) -> Cell<'static> {
    let pct = if peak > 0.0 { cost / peak * 100.0 } else { 0.0 };
    num(money(cost)).style(Style::default().fg(pal.graded(pal.process, pct)))
}

/// The newest points that fit in `width` columns, two per cell.
fn tail(series: &[super::data::DayPoint], width: u16) -> &[super::data::DayPoint] {
    let keep = usize::from(width.saturating_sub(2)) * 2;
    &series[series.len().saturating_sub(keep)..]
}

fn graph_panel(
    frame: &mut Frame,
    area: Rect,
    pal: &Palette,
    focused: bool,
    panel: Panel,
    (title, peak, dates): (&str, String, Option<String>),
    (values, fill, box_ink): (Vec<f64>, Fill, Ink),
) {
    let mut block = panel_block(pal, panel, title, box_ink, focused)
        .title(note_line(pal, &format!("peak {peak}"), box_ink).right_aligned());
    if let Some(dates) = dates {
        // The dates go only where they fit between the corners.
        if dates.chars().count() + 4 <= usize::from(area.width.saturating_sub(2)) {
            block = block.title_bottom(note_line(pal, &dates, box_ink).left_aligned());
        }
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    graph::render(frame.buffer_mut(), inner, &values, fill, pal);
}

/// Cost per day.
pub fn spend(frame: &mut Frame, area: Rect, app: &App, data: &Data, pal: &Palette) {
    let focused = app.focus == Panel::Spend;
    let shown = tail(&data.series, area.width);
    if shown.is_empty() {
        let block = panel_block(pal, Panel::Spend, "Spend", pal.box_cpu, focused);
        return empty(frame, area, pal, block, "No usage in this range.");
    }
    let peak = shown.iter().map(|p| p.cost_usd).fold(0.0, f64::max);
    let dates = format!(
        "{} to {}",
        date_key(shown[0].day),
        date_key(shown[shown.len() - 1].day)
    );
    graph_panel(
        frame,
        area,
        pal,
        focused,
        Panel::Spend,
        ("Spend per day", money(peak), Some(dates)),
        (
            shown.iter().map(|p| p.cost_usd).collect(),
            pal.cpu,
            pal.box_cpu,
        ),
    );
}

/// Tokens per day.
pub fn tokens(frame: &mut Frame, area: Rect, app: &App, data: &Data, pal: &Palette) {
    let focused = app.focus == Panel::Tokens;
    let shown = tail(&data.series, area.width);
    if shown.is_empty() {
        let block = panel_block(pal, Panel::Tokens, "Tokens", pal.box_net, focused);
        return empty(frame, area, pal, block, "No usage in this range.");
    }
    let peak = shown.iter().map(|p| p.tokens).max().unwrap_or(0);
    graph_panel(
        frame,
        area,
        pal,
        focused,
        Panel::Tokens,
        ("Tokens per day", compact(peak), None),
        (
            shown.iter().map(|p| p.tokens as f64).collect(),
            pal.down,
            pal.box_net,
        ),
    );
}

/// Models or projects: messages, tokens, cost and share of the total cost.
#[allow(clippy::too_many_arguments)]
pub fn groups(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    pal: &Palette,
    panel: Panel,
    title: &str,
    groups: &[Group],
    total_cost: f64,
) {
    let focused = app.focus == panel;
    let (box_ink, fill) = match panel {
        Panel::Models => (pal.box_mem, pal.cached),
        _ => (pal.box_mem, pal.avail),
    };
    let block = panel_block(pal, panel, title, box_ink, focused);
    if groups.is_empty() {
        return empty(frame, area, pal, block, "No usage in this range.");
    }
    let sorted = group_rows(groups, app.sort(panel));
    let cols = [
        col("name", Constraint::Fill(1), 0),
        col(">msgs", Constraint::Length(5), 0),
        col(">tokens", Constraint::Length(7), 0),
        col(">cost", Constraint::Length(9), 0),
        col("share", Constraint::Length(8), 48),
    ];
    table(frame, area, app, pal, panel, block, &cols, |w| {
        sorted
            .iter()
            .map(|g| {
                let share = if total_cost > 0.0 {
                    g.cost_usd / total_cost
                } else {
                    0.0
                };
                vec![
                    Cell::from(fit(&g.key, w[0])),
                    num(g.messages.to_string()),
                    num(compact(group_tokens(g))),
                    num(money(g.cost_usd)),
                    meter(pal, fill, share, 8),
                ]
            })
            .collect()
    });
}

/// `45s`, `7m`, `2h`.
fn age(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

/// A model name without the vendor prefix, which every row shares.
fn short_model(model: &str) -> &str {
    model.strip_prefix("claude-").unwrap_or(model)
}

fn session_state(now: i64, s: &ActiveSession, pal: &Palette) -> (&'static str, Color) {
    if now - s.last < LIVE_SECONDS {
        ("live", pal.color(pal.live))
    } else {
        ("idle", pal.color(pal.warn))
    }
}

/// Sessions with a message in the last 15 minutes, with state and cost so far.
pub fn sessions(frame: &mut Frame, area: Rect, app: &App, data: &Data, pal: &Palette) {
    let live = &data.live;
    let focused = app.focus == Panel::Sessions;
    let block = panel_block(pal, Panel::Sessions, "Live sessions", pal.box_proc, focused).title(
        note_line(
            pal,
            &format!(
                "hour {}  today {}",
                money(live.hour.cost_usd),
                money(live.today.cost_usd)
            ),
            pal.box_proc,
        )
        .right_aligned(),
    );
    if live.active.is_empty() {
        let minutes = ACTIVE_SECONDS / 60;
        let text = format!("No session has sent a message in the last {minutes} minutes.");
        return empty(frame, area, pal, block, &text);
    }
    let cols = [
        col("state", Constraint::Length(5), 0),
        col("project", Constraint::Fill(1), 0),
        col("model", Constraint::Fill(1), 0),
        col(">msgs", Constraint::Length(5), 46),
        col(">cost", Constraint::Length(8), 0),
        col(">ago", Constraint::Length(4), 0),
    ];
    let rows = session_rows(&live.active, app.sort(Panel::Sessions));
    let peak = rows.iter().map(|s| s.cost_usd).fold(0.0, f64::max);
    table(frame, area, app, pal, Panel::Sessions, block, &cols, |w| {
        rows.iter()
            .map(|s| {
                let (state, color) = session_state(data.now, s, pal);
                vec![
                    Cell::from(state).style(Style::default().fg(color)),
                    Cell::from(fit(&s.repo, w[1])),
                    Cell::from(fit(short_model(&s.model), w[2])),
                    num(s.messages.to_string()),
                    cost_cell(pal, s.cost_usd, peak),
                    num(age(data.now - s.last)),
                ]
            })
            .collect()
    });
}

/// The newest messages across all sessions.
pub fn events(frame: &mut Frame, area: Rect, app: &App, data: &Data, pal: &Palette) {
    let focused = app.focus == Panel::Events;
    let block = panel_block(pal, Panel::Events, "Recent messages", pal.box_proc, focused)
        .title(note_line(pal, "UTC", pal.box_proc).right_aligned());
    if data.live.feed.is_empty() {
        return empty(frame, area, pal, block, "No messages recorded yet.");
    }
    let cols = [
        col("time", Constraint::Length(8), 0),
        col("project", Constraint::Fill(1), 0),
        col("model", Constraint::Fill(1), 0),
        col(">tokens", Constraint::Length(7), 46),
        col(">cost", Constraint::Length(8), 0),
    ];
    let rows = event_rows(&data.live.feed, app.sort(Panel::Events));
    let peak = rows.iter().map(|e| e.cost_usd).fold(0.0, f64::max);
    table(frame, area, app, pal, Panel::Events, block, &cols, |w| {
        rows.iter()
            .map(|e| {
                vec![
                    Cell::from(clock(e.timestamp)),
                    Cell::from(fit(&e.repo, w[1])),
                    Cell::from(fit(short_model(&e.model), w[2])),
                    num(compact(total_tokens(e))),
                    cost_cell(pal, e.cost_usd, peak),
                ]
            })
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_fills_in_proportion_and_clamps() {
        let pal = super::super::theme::Theme::Mono.palette(Default::default());
        let text = |f: f64, w: usize| {
            let cell = meter(&pal, pal.cpu, f, w);
            format!("{cell:?}")
        };
        assert_eq!(text(0.5, 4).matches(METER).count(), 2);
        assert_eq!(text(2.0, 2).matches(METER).count(), 2);
        assert_eq!(text(-1.0, 2).matches(METER_EMPTY).count(), 2);
    }

    #[test]
    fn short_model_drops_the_vendor_prefix_only() {
        assert_eq!(short_model("claude-opus-5"), "opus-5");
        assert_eq!(short_model("gpt-5-codex"), "gpt-5-codex");
    }

    #[test]
    fn age_picks_the_largest_unit() {
        assert_eq!(age(45), "45s");
        assert_eq!(age(7 * 60), "7m");
        assert_eq!(age(2 * 3600 + 5), "2h");
        assert_eq!(age(-3), "0s");
    }
}
