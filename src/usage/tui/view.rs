// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Draws the app into a ratatui frame. Pure over `App`, so a `TestBackend`
//! renders it to a buffer for snapshot tests. This file is the layout, the
//! header and the footer; `panels` draws each panel.

use super::app::App;
use super::data::Data;
use super::fmt::{compact, money};
use super::panels;
use super::sort::Panel;
use super::theme::Palette;
use crate::usage::aggregate::date_key;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

/// Smallest screen the layout fits.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// Where each panel goes.
#[derive(Debug, PartialEq, Eq)]
pub struct Panels {
    pub header: Rect,
    pub spend: Rect,
    pub tokens: Rect,
    pub models: Rect,
    pub projects: Rect,
    pub sessions: Rect,
    pub events: Rect,
    pub footer: Rect,
}

/// Header and footer are fixed. The rest is three rows: graphs, breakdowns,
/// live, each split in two columns. Spare height goes to the live row.
pub fn layout(area: Rect) -> Panels {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [graphs, tables, live] = Layout::vertical([
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(40),
    ])
    .areas(body);
    let halves = |row: Rect| {
        let [a, b] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(row);
        (a, b)
    };
    let (spend, tokens) = halves(graphs);
    let (models, projects) = halves(tables);
    let (sessions, events) = halves(live);
    Panels {
        header,
        spend,
        tokens,
        models,
        projects,
        sessions,
        events,
        footer,
    }
}

/// Table rows that fit in a panel: its height less two borders and the header.
pub fn visible_rows(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(3))
}

/// The first row drawn so the selected one is the last one in view at most.
pub fn window_start(selected: usize, visible: usize) -> usize {
    (selected + 1).saturating_sub(visible)
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let pal = app.theme.palette(app.depth);
    // The ground first: widgets below only set what they color.
    let base = Style::default().fg(pal.color(pal.fg)).bg(pal.color(pal.bg));
    frame.buffer_mut().set_style(area, base);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small: need {MIN_WIDTH}x{MIN_HEIGHT}, have {}x{}",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(msg), area);
        return;
    }
    let p = layout(area);
    render_header(frame, p.header, app, &pal);
    match &app.data {
        Some(data) => {
            panels::spend(frame, p.spend, app, data, &pal);
            panels::tokens(frame, p.tokens, app, data, &pal);
            let total = data.totals.cost_usd;
            panels::groups(
                frame,
                p.models,
                app,
                &pal,
                Panel::Models,
                "Models",
                &data.models,
                total,
            );
            panels::groups(
                frame,
                p.projects,
                app,
                &pal,
                Panel::Projects,
                "Projects",
                &data.repos,
                total,
            );
            panels::sessions(frame, p.sessions, app, data, &pal);
            panels::events(frame, p.events, app, data, &pal);
        }
        None => {
            let body = Rect::new(p.spend.x, p.spend.y, area.width, 1);
            frame.render_widget(Paragraph::new("Reading usage..."), body);
        }
    }
    frame.render_widget(Paragraph::new(footer(app, &pal)), p.footer);
}

/// The key hints: each key in the accent color, its meaning in the text color.
fn footer(app: &App, pal: &Palette) -> Line<'static> {
    let dim = Style::default().fg(pal.color(pal.graph_text));
    if app.editing {
        return Line::styled(
            format!(" filter: {}_   enter apply   esc cancel", app.draft),
            dim,
        );
    }
    let keys = Style::default()
        .fg(pal.color(pal.hi))
        .add_modifier(Modifier::BOLD);
    let theme = format!("theme ({})", app.theme.name());
    let hints = [
        ("1-6", "panel"),
        ("r", "range"),
        ("/", "filter"),
        ("s/S", "sort"),
        ("enter", "pick"),
        ("t", theme.as_str()),
        ("m", "mouse"),
        ("q", "quit"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
        spans.push(Span::styled(key.to_string(), keys));
        spans.push(Span::styled(format!(" {label} "), dim));
    }
    Line::from(spans)
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

fn render_header(frame: &mut Frame, area: Rect, app: &App, pal: &Palette) {
    let edge = Style::default().fg(pal.color(pal.box_cpu));
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(edge)
        .title(Line::from(vec![
            Span::styled("\u{2500}\u{2524}", edge),
            Span::styled(
                "playbook usage",
                Style::default()
                    .fg(pal.color(pal.title))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("\u{251c}", edge),
        ]));
    if !app.filter.is_empty() {
        block = block.title(
            Line::from(vec![
                Span::styled("\u{2524}", edge),
                Span::styled(
                    format!("filter {}", app.filter),
                    Style::default().fg(pal.color(pal.hi)),
                ),
                Span::styled("\u{251c}\u{2500}", edge),
            ])
            .right_aligned(),
        );
    }
    let accent = Style::default().fg(pal.color(pal.hi));
    let error = Style::default().fg(pal.color(pal.error));
    let line = match (&app.data, &app.error) {
        (Some(d), err) => {
            let tokens = d.totals.input_tokens
                + d.totals.output_tokens
                + d.totals.cache_creation_tokens
                + d.totals.cache_read_tokens;
            let mut spans = vec![
                Span::styled(range_label(d), accent),
                Span::raw(format!(
                    "   {}   {} messages   {} tokens",
                    money(d.totals.cost_usd),
                    d.totals.messages,
                    compact(tokens)
                )),
            ];
            if d.unpriced > 0 {
                spans.push(Span::styled(
                    format!("   {} unpriced", d.unpriced),
                    Style::default().fg(pal.color(pal.warn)),
                ));
            }
            if let Some(e) = err {
                spans.push(Span::styled(format!("   {e}"), error));
            }
            Line::from(spans)
        }
        (None, Some(e)) => Line::styled(e.clone(), error),
        (None, None) => Line::raw("Reading usage..."),
    };
    frame.render_widget(Paragraph::new(line).block(block), area);
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
        let text = screen(&app, 100, 30);
        assert!(text.contains("$13.75"), "{text}");
        assert!(text.contains("2 messages"), "{text}");
        assert!(text.contains("opus") && text.contains("alpha"), "{text}");
        assert!(
            text.contains("Models") && text.contains("Projects"),
            "{text}"
        );
    }

    #[test]
    fn every_panel_draws_with_live_data() {
        let now_event = UsageEvent {
            timestamp: NOW - 20,
            ..event("claude-opus-5", "alpha", 0.5)
        };
        let live = crate::usage::query::live_view(
            std::slice::from_ref(&now_event),
            std::slice::from_ref(&now_event),
            std::slice::from_ref(&now_event),
            NOW,
        );
        let mut app = App::new(Range::LastDays(30));
        app.data = Some(Data::build(
            std::slice::from_ref(&now_event),
            live,
            NOW,
            Range::LastDays(30),
            "",
        ));
        let text = screen(&app, 100, 30);
        for want in [
            "Spend per day",
            "Tokens per day",
            "Models",
            "Projects",
            "Live sessions",
            "Recent messages",
            "live",
            "opus-5",
        ] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
    }

    #[test]
    fn a_narrow_screen_drops_the_share_bar_and_keeps_the_names() {
        let app = app_with(&[event("claude-opus-5", "alpha", 12.5)]);
        let wide = screen(&app, 120, 30);
        let narrow = screen(&app, 80, 24);
        assert!(
            wide.contains("share") && wide.contains('\u{25a0}'),
            "{wide}"
        );
        assert!(!narrow.contains("share"), "{narrow}");
        assert!(
            narrow.contains("claude-opus-5") && narrow.contains("alpha"),
            "{narrow}"
        );
    }

    #[test]
    fn the_layout_tiles_the_screen_without_overlap() {
        let area = Rect::new(0, 0, 100, 30);
        let p = layout(area);
        let rects = [
            p.header, p.spend, p.tokens, p.models, p.projects, p.sessions, p.events, p.footer,
        ];
        let cells: u32 = rects
            .iter()
            .map(|r| u32::from(r.width) * u32::from(r.height))
            .sum();
        assert_eq!(cells, 100 * 30);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
        }
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
        assert!(screen(&app, 80, 24).contains("Reading usage"));
    }

    #[test]
    fn the_footer_lists_the_keys_and_the_active_theme() {
        let app = App::new(Range::All);
        let text = screen(&app, 100, 30);
        assert!(text.contains("/ filter"));
        assert!(text.contains("s/S sort"));
        assert!(text.contains("t theme (btop)"));
    }

    #[test]
    fn the_footer_shows_the_draft_while_editing_and_the_filter_once_set() {
        let mut app = App::new(Range::All);
        app.editing = true;
        app.draft = "opus".into();
        assert!(screen(&app, 100, 30).contains("filter: opus_"));
        app.editing = false;
        app.filter = "opus".into();
        assert!(screen(&app, 100, 30).contains("filter opus"));
    }

    #[test]
    fn the_mono_theme_draws_no_foreground_color() {
        let mut app = app_with(&[event("claude-opus-4-1", "a/b", 1.0)]);
        app.theme = super::super::theme::Theme::Mono;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| render(f, &app)).unwrap();
        let colored = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .filter(|c| c.fg != ratatui::style::Color::Reset)
            .count();
        assert_eq!(colored, 0);
    }

    #[test]
    fn a_resize_redraws_to_the_new_size() {
        let app = App::new(Range::All);
        assert!(screen(&app, 60, 20).contains("Terminal too small"));
        assert!(!screen(&app, 100, 30).contains("Terminal too small"));
    }
}
