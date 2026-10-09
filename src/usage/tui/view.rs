// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Draws the app into a ratatui frame. Pure over `App`, so a `TestBackend`
//! renders it to a buffer for snapshot tests.

use super::app::App;
use super::data::Data;
use super::fmt::{bar, compact, fit, money};
use crate::usage::aggregate::{date_key, Group};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};
use ratatui::Frame;

/// Smallest screen the layout fits.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 12;

const ACCENT: Color = Color::Cyan;

fn total_tokens(g: &Group) -> u64 {
    g.input_tokens + g.output_tokens + g.cache_creation_tokens + g.cache_read_tokens
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small: need {MIN_WIDTH}x{MIN_HEIGHT}, have {}x{}",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(msg), area);
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);

    render_header(frame, header, app);
    match &app.data {
        Some(data) => {
            render_groups(frame, left, "Models", &data.models, data.totals.cost_usd);
            render_groups(frame, right, "Projects", &data.repos, data.totals.cost_usd);
        }
        None => {
            frame.render_widget(Paragraph::new("Reading usage..."), body);
        }
    }
    frame.render_widget(
        Paragraph::new(" r range   q quit").style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
}

fn range_label(data: &Data) -> String {
    match data.range.days(data.now) {
        Some((first, past)) => format!(
            "{} {} to {}",
            data.range.key(),
            date_key(first),
            date_key(past - 1)
        ),
        None => "all time".to_string(),
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(Span::styled(
        " playbook usage ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    let line = match (&app.data, &app.error) {
        (Some(d), error) => {
            let tokens = d.totals.input_tokens
                + d.totals.output_tokens
                + d.totals.cache_creation_tokens
                + d.totals.cache_read_tokens;
            let mut spans = vec![
                Span::styled(range_label(d), Style::default().fg(ACCENT)),
                Span::raw(format!(
                    "   {}   {} messages   {} tokens",
                    money(d.totals.cost_usd),
                    d.totals.messages,
                    compact(tokens)
                )),
            ];
            if let Some(e) = error {
                spans.push(Span::styled(
                    format!("   {e}"),
                    Style::default().fg(Color::Red),
                ));
            }
            Line::from(spans)
        }
        (None, Some(e)) => Line::styled(e.clone(), Style::default().fg(Color::Red)),
        (None, None) => Line::raw("Reading usage..."),
    };
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn render_groups(frame: &mut Frame, area: Rect, title: &str, groups: &[Group], total_cost: f64) {
    let mut sorted: Vec<&Group> = groups.iter().collect();
    sorted.sort_by(|a, b| {
        b.cost_usd
            .total_cmp(&a.cost_usd)
            .then_with(|| a.key.cmp(&b.key))
    });
    let name_width = usize::from(area.width.saturating_sub(2 + 8 + 9 + 11 + 9 + 4)).max(8);
    let rows: Vec<Row> = sorted
        .iter()
        .map(|g| {
            let share = if total_cost > 0.0 {
                g.cost_usd / total_cost
            } else {
                0.0
            };
            Row::new(vec![
                Cell::from(fit(&g.key, name_width)),
                Cell::from(g.messages.to_string()),
                Cell::from(compact(total_tokens(g))),
                Cell::from(money(g.cost_usd)),
                Cell::from(bar(share, 8)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(9),
    ];
    let header = Row::new(["", "msgs", "tokens", "cost", "share"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::bordered().title(format!(" {title} ")));
    frame.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::query::{LiveView, Range};
    use crate::usage::UsageEvent;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const NOW: i64 = 1_788_400_000;

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_with(events: &[UsageEvent]) -> App {
        let mut app = App::new(Range::All);
        app.data = Some(Data::build(
            events,
            LiveView::default(),
            NOW,
            Range::All,
            "",
        ));
        app
    }

    fn event(model: &str, repo: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("{model}{repo}"),
            timestamp: NOW - 60,
            session_id: "s".into(),
            model: model.into(),
            repo: repo.into(),
            input_tokens: 1_500,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn the_header_and_both_tables_show_the_totals_and_rows() {
        let app = app_with(&[event("opus", "alpha", 12.5), event("sonnet", "beta", 1.25)]);
        let text = screen(&app, 100, 14);
        assert!(text.contains("$13.75"), "{text}");
        assert!(text.contains("2 messages"), "{text}");
        assert!(text.contains("opus") && text.contains("alpha"), "{text}");
        assert!(
            text.contains("Models") && text.contains("Projects"),
            "{text}"
        );
    }

    #[test]
    fn a_screen_below_the_minimum_says_so_instead_of_drawing() {
        let app = app_with(&[]);
        let text = screen(&app, 40, 8);
        assert!(text.contains("Terminal too small"), "{text}");
    }

    #[test]
    fn before_the_first_read_the_view_says_it_is_reading() {
        let app = App::new(Range::All);
        assert!(screen(&app, 80, 14).contains("Reading usage"));
    }
}
