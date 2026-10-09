// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The six panels: spend and token graphs, the model and project breakdowns,
//! the live sessions and the recent events.

use super::data::{Data, DayPoint};
use super::fmt::{bar, compact, fit, money};
use super::theme::Palette;
use crate::usage::aggregate::{date_key, Group};
use crate::usage::query::{clock, total_tokens, ActiveSession, ACTIVE_SECONDS};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Cell, Paragraph, Row, Sparkline, Table};
use ratatui::Frame;

/// A session whose last message is newer than this is working right now.
const LIVE_SECONDS: i64 = 120;

fn block(title: String) -> Block<'static> {
    Block::bordered().title(format!(" {title} "))
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn num(text: impl Into<String>) -> Cell<'static> {
    Cell::from(Text::from(text.into()).alignment(Alignment::Right))
}

/// A header row. A label starting with `>` is right aligned, like its numbers.
fn head(labels: &[&'static str]) -> Row<'static> {
    Row::new(labels.iter().map(|l| match l.strip_prefix('>') {
        Some(right) => num(right),
        None => Cell::from(*l),
    }))
    .style(bold())
}

/// A model name without the vendor prefix, which every row shares.
fn short_model(model: &str) -> &str {
    model.strip_prefix("claude-").unwrap_or(model)
}

/// One table column. `min_inner` is the inner panel width below which the
/// column is left out, so a narrow terminal keeps the names readable.
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

/// Draws a table. `rows` gets the width in cells of every column (0 for a
/// column that is left out) so it can cut text to fit, and returns one cell per
/// column for every row.
fn table<'a>(
    frame: &mut Frame,
    area: Rect,
    title: String,
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
    let rows: Vec<Row> = rows(&widths)
        .into_iter()
        .map(|cells| {
            Row::new(
                cells
                    .into_iter()
                    .zip(&keep)
                    .filter(|(_, k)| **k)
                    .map(|(c, _)| c),
            )
        })
        .collect();
    let heads: Vec<&'static str> = cols
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(c, _)| c.head)
        .collect();
    frame.render_widget(
        Table::new(rows, constraints)
            .header(head(&heads))
            .block(block(title)),
        area,
    );
}

fn empty(frame: &mut Frame, area: Rect, title: String, message: &str) {
    frame.render_widget(
        Paragraph::new(message.to_string()).block(block(title)),
        area,
    );
}

/// The newest points that fit in `width` columns.
fn tail(series: &[DayPoint], width: u16) -> &[DayPoint] {
    let keep = usize::from(width.saturating_sub(2));
    &series[series.len().saturating_sub(keep)..]
}

/// Repeats each value so a short range fills the panel: 7 days in 56 columns
/// draw 8 columns per day.
fn stretch(values: Vec<u64>, width: u16) -> Vec<u64> {
    let inner = usize::from(width.saturating_sub(2));
    let each = (inner / values.len().max(1)).max(1);
    values
        .into_iter()
        .flat_map(|v| std::iter::repeat_n(v, each))
        .collect()
}

fn graph(frame: &mut Frame, area: Rect, title: String, values: Vec<u64>, color: Color) {
    let values = stretch(values, area.width);
    let spark = Sparkline::default()
        .data(values)
        .style(Style::default().fg(color))
        .block(block(title));
    frame.render_widget(spark, area);
}

/// Cost per day.
pub fn spend(frame: &mut Frame, area: Rect, data: &Data, pal: &Palette) {
    let shown = tail(&data.series, area.width);
    if shown.is_empty() {
        return empty(frame, area, "Spend".to_string(), "No usage in this range.");
    }
    let peak = shown.iter().map(|p| p.cost_usd).fold(0.0, f64::max);
    let title = format!(
        "Spend per day  peak {}  {} to {}",
        money(peak),
        date_key(shown[0].day),
        date_key(shown[shown.len() - 1].day)
    );
    let values = shown
        .iter()
        .map(|p| (p.cost_usd * 100.0).round() as u64)
        .collect();
    graph(frame, area, title, values, pal.accent);
}

/// Tokens per day.
pub fn tokens(frame: &mut Frame, area: Rect, data: &Data, pal: &Palette) {
    let shown = tail(&data.series, area.width);
    if shown.is_empty() {
        return empty(frame, area, "Tokens".to_string(), "No usage in this range.");
    }
    let peak = shown.iter().map(|p| p.tokens).max().unwrap_or(0);
    let title = format!("Tokens per day  peak {}", compact(peak));
    graph(
        frame,
        area,
        title,
        shown.iter().map(|p| p.tokens).collect(),
        pal.tokens,
    );
}

fn group_tokens(g: &Group) -> u64 {
    g.input_tokens + g.output_tokens + g.cache_creation_tokens + g.cache_read_tokens
}

/// Models or projects: messages, tokens, cost and share of the total cost,
/// most expensive first.
pub fn groups(frame: &mut Frame, area: Rect, title: &str, groups: &[Group], total_cost: f64) {
    if groups.is_empty() {
        return empty(frame, area, title.to_string(), "No usage in this range.");
    }
    let mut sorted: Vec<&Group> = groups.iter().collect();
    sorted.sort_by(|a, b| {
        b.cost_usd
            .total_cmp(&a.cost_usd)
            .then_with(|| a.key.cmp(&b.key))
    });
    let cols = [
        col("", Constraint::Fill(1), 0),
        col(">msgs", Constraint::Length(5), 0),
        col(">tokens", Constraint::Length(7), 0),
        col(">cost", Constraint::Length(9), 0),
        col("share", Constraint::Length(8), 48),
    ];
    table(frame, area, title.to_string(), &cols, |w| {
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
                    Cell::from(bar(share, 8)),
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

fn session_state(now: i64, s: &ActiveSession, pal: &Palette) -> (&'static str, Color) {
    if now - s.last < LIVE_SECONDS {
        ("live", pal.live)
    } else {
        ("idle", pal.warn)
    }
}

/// Sessions with a message in the last 15 minutes, with state and cost so far.
pub fn sessions(frame: &mut Frame, area: Rect, data: &Data, pal: &Palette) {
    let live = &data.live;
    let title = format!(
        "Live sessions  hour {}  today {}",
        money(live.hour.cost_usd),
        money(live.today.cost_usd)
    );
    if live.active.is_empty() {
        let minutes = ACTIVE_SECONDS / 60;
        let text = format!("No session has sent a message in the last {minutes} minutes.");
        return empty(frame, area, title, &text);
    }
    let cols = [
        col("state", Constraint::Length(5), 0),
        col("project", Constraint::Fill(1), 0),
        col("model", Constraint::Fill(1), 0),
        col(">msgs", Constraint::Length(4), 46),
        col(">cost", Constraint::Length(8), 0),
        col(">ago", Constraint::Length(4), 0),
    ];
    table(frame, area, title, &cols, |w| {
        live.active
            .iter()
            .map(|s| {
                let (state, color) = session_state(data.now, s, pal);
                vec![
                    Cell::from(state).style(Style::default().fg(color)),
                    Cell::from(fit(&s.repo, w[1])),
                    Cell::from(fit(short_model(&s.model), w[2])),
                    num(s.messages.to_string()),
                    num(money(s.cost_usd)),
                    num(age(data.now - s.last)),
                ]
            })
            .collect()
    });
}

/// The newest messages across all sessions.
pub fn events(frame: &mut Frame, area: Rect, data: &Data) {
    let title = "Recent messages (UTC)".to_string();
    if data.live.feed.is_empty() {
        return empty(frame, area, title, "No messages recorded yet.");
    }
    let cols = [
        col("time", Constraint::Length(8), 0),
        col("project", Constraint::Fill(1), 0),
        col("model", Constraint::Fill(1), 0),
        col(">tokens", Constraint::Length(7), 46),
        col(">cost", Constraint::Length(8), 0),
    ];
    table(frame, area, title, &cols, |w| {
        data.live
            .feed
            .iter()
            .map(|e| {
                vec![
                    Cell::from(clock(e.timestamp)),
                    Cell::from(fit(&e.repo, w[1])),
                    Cell::from(fit(short_model(&e.model), w[2])),
                    num(compact(total_tokens(e))),
                    num(money(e.cost_usd)),
                ]
            })
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_fills_the_panel_with_whole_columns_per_value() {
        assert_eq!(stretch(vec![1, 2], 8), vec![1, 1, 1, 2, 2, 2]);
        assert_eq!(stretch(vec![1, 2, 3], 4), vec![1, 2, 3]);
        assert!(stretch(vec![], 10).is_empty());
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
