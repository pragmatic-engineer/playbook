// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Draws the app into a ratatui frame. Pure over `App`, so a `TestBackend`
//! renders it to a buffer for snapshot tests. This file is the layout, the
//! header and the footer; `panels` draws each panel.

use super::app::App;
use super::data::Data;
use super::fmt::{compact, fit, money, signed_compact, signed_count, signed_money, signed_percent};
use super::panels;
use super::theme::Palette;
use crate::usage::aggregate::date_key;
use crate::usage::period::{elapsed_text, Comparison, Span as TimeSpan};
use crate::usage::query::{clock, day_of, Range};
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
    pub live: Rect,
    pub sessions: Rect,
    pub footer: Rect,
}

/// The header holds the comparison: a context line and one row each for cost,
/// tokens and messages, between two borders.
const HEADER_HEIGHT: u16 = 6;

/// Header and footer are fixed. The rest is three rows: graphs, breakdowns,
/// live and sessions, each split in two columns.
pub fn layout(area: Rect) -> Panels {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [graphs, tables, live] = Layout::vertical([
        Constraint::Fill(40),
        Constraint::Fill(26),
        Constraint::Fill(34),
    ])
    .areas(body);
    let halves = |row: Rect| {
        let [a, b] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(row);
        (a, b)
    };
    let (spend, tokens) = halves(graphs);
    let (models, projects) = halves(tables);
    let (live, sessions) = halves(live);
    Panels {
        header,
        spend,
        tokens,
        models,
        projects,
        live,
        sessions,
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
            panels::models(frame, p.models, app, data, &pal);
            panels::projects(frame, p.projects, app, data, &pal);
            panels::live(frame, p.live, app, data, &pal);
            panels::sessions(frame, p.sessions, app, data, &pal);
        }
        None => {
            let body = Rect::new(p.spend.x, p.spend.y, area.width, 1);
            frame.render_widget(Paragraph::new("Reading usage..."), body);
        }
    }
    frame.render_widget(Paragraph::new(footer(app, &pal, area.width)), p.footer);
}

/// The key hints: each key in the accent color, its meaning in the text color,
/// and at the right when the view was last refreshed.
fn footer(app: &App, pal: &Palette, area_width: u16) -> Line<'static> {
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
    let (status, status_style) = refresh_status(app, pal, area_width);
    // The status keeps its place at the right, so the hints get what is left.
    let room = usize::from(area_width).saturating_sub(status.chars().count());
    let theme = format!("theme ({})", app.theme.name());
    // Listed in the order shown. When the line is too wide, hints drop in
    // `DROP` order, so the ones that matter most stay.
    let mut hints = vec![
        ("1-6", "panel".to_string()),
        ("r", "range".to_string()),
        ("d/w/W/m/M/a", "jump".to_string()),
        ("\u{2190}/\u{2192}", "day".to_string()),
        ("v", "view".to_string()),
        ("/", "filter".to_string()),
        ("s/S", "sort".to_string()),
        ("enter", "pick".to_string()),
        ("t", theme),
        ("x", "mouse".to_string()),
        ("q", "quit".to_string()),
    ];
    let width = |hints: &[(&str, String)]| {
        1 + hints
            .iter()
            .map(|(k, l)| k.chars().count() + l.chars().count() + 2)
            .sum::<usize>()
    };
    for drop in FOOTER_DROP {
        if width(&hints) <= room {
            break;
        }
        hints.retain(|(k, _)| *k != drop);
    }
    let used = width(&hints);
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
        spans.push(Span::styled(key.to_string(), keys));
        spans.push(Span::styled(format!(" {label} "), dim));
    }
    let gap = usize::from(area_width).saturating_sub(used + status.chars().count());
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(status, status_style));
    Line::from(spans)
}

/// The order footer hints are dropped in on a narrow screen.
const FOOTER_DROP: [&str; 5] = ["x", "t", "enter", "s/S", "1-6"];

/// What the right end of the footer says: when the data was last read, or why
/// the last read failed. The time is the snapshot's own clock, so it moves only
/// when a refresh really finished.
fn refresh_status(app: &App, pal: &Palette, area_width: u16) -> (String, Style) {
    if let Some(e) = &app.error {
        let room = usize::from(area_width / 2);
        let text = fit(&format!("refresh failed: {e} "), room);
        return (text, Style::default().fg(pal.color(pal.error)));
    }
    let dim = Style::default().fg(pal.color(pal.graph_text));
    match &app.data {
        // The interval goes first when the line is short.
        Some(d) if area_width >= 100 => (
            format!(
                "updated {} UTC, every {}s ",
                clock(d.now),
                app.refresh_every.as_secs()
            ),
            dim,
        ),
        Some(d) => (format!("updated {} UTC ", clock(d.now)), dim),
        None => (String::new(), dim),
    }
}

fn range_label(data: &Data) -> String {
    match data.range.days(data.now) {
        Some((first, past)) if past - first == 1 => {
            format!("{} {}", data.range.label(), date_key(first))
        }
        Some((first, past)) => format!(
            "{} {} to {}",
            data.range.label(),
            date_key(first),
            date_key(past - 1)
        ),
        None => "all time".to_string(),
    }
}

/// `08-31`, the UTC date without its year.
fn month_day(timestamp: i64) -> String {
    date_key(day_of(timestamp))[5..].to_string()
}

/// `08-31 to 09-03`, or just the date when the span is one day.
fn date_span(span: TimeSpan) -> String {
    let (first, last) = (month_day(span.start), month_day(span.end - 1));
    if first == last {
        first
    } else {
        format!("{first} to {last}")
    }
}

/// `12:00`, the UTC time of day the span reaches.
fn reaches(span: TimeSpan) -> String {
    clock(span.end - 1)[..5].to_string()
}

/// The context line: the period so far and the one it is compared with, and
/// the elapsed span they share. Returns the current part and the rest, in the
/// longest form that fits `room`.
fn context(c: &Comparison, room: usize) -> (String, String) {
    let (w, p) = (c.windows, c.period);
    let span = elapsed_text(w.current.seconds());
    let forms = [
        (
            format!(
                "{} {} until {}",
                p.label(),
                date_span(w.current),
                reaches(w.current)
            ),
            format!(
                "  vs  {} {} until {}  (same span {span})",
                p.previous_label(),
                date_span(w.previous),
                reaches(w.previous)
            ),
        ),
        (
            format!("{} {}", p.label(), date_span(w.current)),
            format!(
                "  vs  {} {}  (same span {span})",
                p.previous_label(),
                date_span(w.previous)
            ),
        ),
        (
            p.label().to_string(),
            format!("  vs  {}  (same {span})", p.previous_label()),
        ),
    ];
    let fits = |(a, b): &(String, String)| a.chars().count() + b.chars().count() <= room;
    let last = forms[2].clone();
    forms.into_iter().find(fits).unwrap_or(last)
}

/// The context line of the header: the comparison when the range is a period,
/// else the range and its dates alone.
fn context_line(d: &Data, room: usize) -> (String, String) {
    match &d.comparison {
        Some(c) => context(c, room),
        None => (
            range_label(d),
            "  (no previous period to compare)".to_string(),
        ),
    }
}

/// One metric of the header: its name, its value, and when the range is a
/// period the value before and the change.
struct Metric {
    name: &'static str,
    now: String,
    before: Option<String>,
    amount: String,
    percent: String,
    /// The change itself, whose sign picks the color.
    change: f64,
}

impl Metric {
    fn alone(name: &'static str, now: String) -> Metric {
        Metric {
            name,
            now,
            before: None,
            amount: String::new(),
            percent: String::new(),
            change: 0.0,
        }
    }
}

fn metrics(d: &Data) -> [Metric; 3] {
    let Some(c) = &d.comparison else {
        return [
            Metric::alone("cost", money(d.totals.cost_usd)),
            Metric::alone("tokens", compact(d.totals.tokens())),
            Metric::alone("messages", d.totals.messages.to_string()),
        ];
    };
    let (cost, tokens, messages) = (c.cost(), c.tokens(), c.messages());
    [
        Metric {
            name: "cost",
            now: money(c.current.cost_usd),
            before: Some(money(c.previous.cost_usd)),
            amount: signed_money(cost.amount),
            percent: signed_percent(cost.percent, c.current.cost_usd > 0.0),
            change: cost.amount,
        },
        Metric {
            name: "tokens",
            now: compact(c.current.tokens()),
            before: Some(compact(c.previous.tokens())),
            amount: signed_compact(tokens.amount),
            percent: signed_percent(tokens.percent, c.current.tokens() > 0),
            change: tokens.amount,
        },
        Metric {
            name: "messages",
            now: c.current.messages.to_string(),
            before: Some(c.previous.messages.to_string()),
            amount: signed_count(messages.amount),
            percent: signed_percent(messages.percent, c.current.messages > 0),
            change: messages.amount,
        },
    ]
}

/// The header body: the context line, then cost, tokens and messages, with the
/// previous period's value and the change for a period.
fn header_lines(d: &Data, pal: &Palette, room: usize) -> Vec<Line<'static>> {
    let accent = Style::default()
        .fg(pal.color(pal.hi))
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(pal.color(pal.graph_text));
    let (now, rest) = context_line(d, room);
    let mut lines = vec![Line::from(vec![
        Span::styled(now, accent),
        Span::styled(rest, dim),
    ])];
    for m in metrics(d) {
        let tint = if m.change > 0.0 {
            pal.warn
        } else if m.change < 0.0 {
            pal.live
        } else {
            pal.graph_text
        };
        let mut spans = vec![
            Span::styled(format!("{:<9}", m.name), dim),
            Span::styled(
                format!("{:>11}", m.now),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(before) = &m.before {
            spans.push(Span::styled(format!("   vs{before:>11}   "), dim));
            spans.push(Span::styled(
                format!("{:>9} {:>8}", m.amount, m.percent),
                Style::default().fg(pal.color(tint)),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// The time zone note in the header border. Weeks say which day they start on.
fn zone_note(app: &App) -> String {
    match app.range {
        Range::ThisWeek(w) | Range::LastWeek(w) => format!("UTC, weeks start {}", w.short()),
        _ => "UTC".to_string(),
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, pal: &Palette) {
    let edge = Style::default().fg(pal.color(pal.box_cpu));
    let note = |text: String, ink: Style| {
        Line::from(vec![
            Span::styled("\u{2524}", edge),
            Span::styled(text, ink),
            Span::styled("\u{251c}\u{2500}", edge),
        ])
        .right_aligned()
    };
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
        ]))
        .title(Line::from(vec![
            Span::styled("\u{2500}\u{2524}", edge),
            Span::styled(
                zone_note(app),
                Style::default().fg(pal.color(pal.graph_text)),
            ),
            Span::styled("\u{251c}", edge),
        ]));
    if !app.filter.is_empty() {
        let ink = Style::default().fg(pal.color(pal.hi));
        block = block.title(note(format!("filter {}", app.filter), ink));
    }
    if let Some(d) = app.data.as_ref().filter(|d| d.unpriced > 0) {
        let ink = Style::default().fg(pal.color(pal.warn));
        block = block.title(note(format!("{} unpriced", d.unpriced), ink));
    }
    let room = usize::from(area.width.saturating_sub(2));
    let lines = match (&app.data, &app.error) {
        (Some(d), _) => header_lines(d, pal, room),
        (None, Some(e)) => vec![Line::styled(
            e.clone(),
            Style::default().fg(pal.color(pal.error)),
        )],
        (None, None) => vec![Line::raw("Reading usage...")],
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
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

    fn yesterday(model: &str, repo: &str, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: format!("y{model}{repo}{cost}"),
            timestamp: NOW - 86_400 - 60,
            ..event(model, repo, cost)
        }
    }

    fn app_for(range: Range, events: &[UsageEvent]) -> App {
        let mut app = App::new(range);
        app.data = Some(Data::build(events, LiveView::default(), NOW, range, ""));
        app
    }

    #[test]
    fn the_header_and_both_tables_show_the_totals_and_rows() {
        let app = app_with(&[event("opus", "alpha", 12.5), event("sonnet", "beta", 1.25)]);
        let text = screen(&app, 100, 30);
        assert!(text.contains("$13.75"), "{text}");
        assert!(text.contains("messages") && text.contains("1.5K"), "{text}");
        assert!(text.contains("opus") && text.contains("alpha"), "{text}");
        assert!(
            text.contains("Models") && text.contains("Projects"),
            "{text}"
        );
    }

    #[test]
    fn a_range_that_is_not_a_period_has_no_comparison() {
        let text = screen(&app_with(&[event("opus", "a", 1.0)]), 100, 30);
        assert!(text.contains("(no previous period to compare)"), "{text}");
        assert!(!text.contains(" vs "), "{text}");
    }

    #[test]
    fn the_header_shows_both_periods_and_the_change_in_cost_tokens_and_messages() {
        let app = app_for(
            Range::Today,
            &[
                event("opus", "alpha", 12.5),
                event("sonnet", "beta", 1.25),
                yesterday("opus", "alpha", 10.0),
            ],
        );
        let text = screen(&app, 100, 30);
        for want in [
            "today",
            "yesterday",
            "same span",
            "$13.75",
            "$10.00",
            "+$3.75",
            "+37.5%",
            "+1",
            "UTC",
        ] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
        // Messages: 2 now against 1 before, tokens 3.0K against 1.5K.
        assert!(text.contains("+100%") && text.contains("+1.5K"), "{text}");
    }

    #[test]
    fn a_period_with_no_previous_spend_says_new_not_a_percent() {
        let text = screen(
            &app_for(Range::Today, &[event("opus", "alpha", 2.0)]),
            100,
            30,
        );
        assert!(text.contains("new"), "{text}");
        assert!(!text.contains("inf") && !text.contains("NaN"), "{text}");
    }

    #[test]
    fn a_fall_in_spend_reads_with_a_minus_sign() {
        let app = app_for(
            Range::Today,
            &[event("opus", "a", 1.0), yesterday("opus", "a", 4.0)],
        );
        let text = screen(&app, 100, 30);
        assert!(text.contains("-$3.00") && text.contains("-75.0%"), "{text}");
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
            "Sessions per day",
            "live",
            "opus-5",
        ] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
    }

    #[test]
    fn the_models_table_compares_each_model_with_the_previous_period() {
        let app = app_for(
            Range::Today,
            &[
                event("claude-opus-5", "alpha", 12.5),
                yesterday("claude-opus-5", "alpha", 10.0),
                yesterday("claude-haiku-5", "alpha", 0.5),
            ],
        );
        let text = screen(&app, 100, 30);
        let rows: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split('\u{2502}').nth(1))
            .filter(|r| r.contains("opus-5") || r.contains("haiku-5"))
            .collect();
        assert_eq!(rows.len(), 2, "{text}");
        // Opus: $12.50 now, $10.00 before, +$2.50 and +25.0%.
        let opus = rows.iter().find(|r| r.contains("opus-5")).unwrap();
        for want in ["$12.50", "$10.00", "+$2.50", "+25.0%"] {
            assert!(opus.contains(want), "{want}: {opus}");
        }
        // Haiku was used only yesterday: nothing now, down 100%.
        let haiku = rows.iter().find(|r| r.contains("haiku-5")).unwrap();
        for want in ["$0.00", "$0.50", "-$0.50", "-100%"] {
            assert!(haiku.contains(want), "{want}: {haiku}");
        }
        assert!(text.contains("vs yesterday"), "{text}");
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
            narrow.contains("opus-5") && narrow.contains("alpha"),
            "{narrow}"
        );
    }

    #[test]
    fn the_layout_tiles_the_screen_without_overlap() {
        let area = Rect::new(0, 0, 100, 30);
        let p = layout(area);
        let rects = [
            p.header, p.spend, p.tokens, p.models, p.projects, p.live, p.sessions, p.footer,
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
        let text = screen(&app, 120, 30);
        for hint in [
            "/ filter",
            "s/S sort",
            "v view",
            "\u{2190}/\u{2192} day",
            "t theme (btop)",
        ] {
            assert!(text.contains(hint), "{hint}");
        }
    }

    #[test]
    fn the_footer_shows_when_the_data_was_read_and_how_often() {
        let app = app_with(&[]);
        assert!(screen(&app, 150, 30).contains("updated 01:46:40 UTC, every 5s"));
        // A short line keeps the time and drops the interval.
        let short = screen(&app, 80, 24);
        assert!(short.contains("updated 01:46:40 UTC"), "{short}");
        assert!(!short.contains("every"), "{short}");
        // A configured interval is shown.
        let mut app = app;
        app.refresh_every = std::time::Duration::from_secs(30);
        assert!(screen(&app, 150, 30).contains("every 30s"));
    }

    #[test]
    fn a_failed_refresh_is_named_in_the_footer_and_the_data_stays() {
        let mut app = app_with(&[event("opus", "alpha", 2.0)]);
        app.error = Some("database is busy".into());
        let text = screen(&app, 120, 30);
        assert!(text.contains("refresh failed: database is busy"), "{text}");
        assert!(text.contains("$2.00"), "the last good data stays:\n{text}");
        assert!(!text.contains("updated"), "{text}");
    }

    #[test]
    fn the_sessions_table_names_each_day_once_and_lists_its_sessions() {
        let session = |id: &str, ts: i64, cost: f64| UsageEvent {
            event_id: format!("{id}{ts}"),
            timestamp: ts,
            session_id: id.into(),
            model: "claude-opus-5".into(),
            repo: "alpha".into(),
            cost_usd: cost,
            ..UsageEvent::default()
        };
        // Today is Thursday 09-03, one day back is Wednesday 09-02.
        let events = [
            session("aaaaaaaa-1", NOW - 60, 1.0),
            session("bbbbbbbb-2", NOW - 50, 2.0),
            session("cccccccc-3", NOW - 86_400, 3.0),
        ];
        let live = LiveView::default();
        let mut app = App::new(Range::All);
        app.data = Some(Data::build(&events, live, NOW, Range::All, ""));
        let text = screen(&app, 160, 50);
        let ids = ["aaaaaaaa", "bbbbbbbb", "cccccccc"];
        let rows: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split('\u{2502}').nth(3))
            .filter(|r| ids.iter().any(|id| r.contains(id)))
            .collect();
        assert_eq!(rows.len(), 3, "{text}");
        assert!(
            rows[0].contains("Thu 09-03") && rows[0].contains("bbbbbbbb"),
            "{rows:?}"
        );
        assert!(
            !rows[1].contains("09-03") && rows[1].contains("aaaaaaaa"),
            "{rows:?}"
        );
        assert!(
            rows[2].contains("Wed 09-02") && rows[2].contains("cccccccc"),
            "{rows:?}"
        );
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
    fn the_header_names_each_period_its_previous_one_and_their_exact_utc_dates() {
        use crate::usage::query::WeekStart;
        // Thursday 2026-09-03 01:46:40 UTC, so 1h 46m into the day.
        for (range, want) in [
            (
                Range::Today,
                "today 09-03 until 01:46  vs  yesterday 09-02 until 01:46  (same span 1h 46m)",
            ),
            (
                Range::ThisWeek(WeekStart::Monday),
                "this week 08-31 to 09-03 until 01:46  vs  last week 08-24 to 08-27 until 01:46  (same span 3d 1h)",
            ),
            (
                Range::CurrentMonth,
                "this month 09-01 to 09-03 until 01:46  vs  last month 08-01 to 08-03 until 01:46  (same span 2d 1h)",
            ),
            // The other ranges have no previous period.
            (Range::LastWeek(WeekStart::Monday), "last week 2026-08-24 to 2026-08-30"),
            (Range::LastMonth, "last month 2026-08-01 to 2026-08-31"),
            (Range::LastDays(30), "last 30 days 2026-08-05 to 2026-09-03"),
            (Range::All, "all time"),
        ] {
            let text = screen(&app_for(range, &[]), 120, 30);
            assert!(text.contains(want), "{want}:\n{text}");
            assert!(text.contains("UTC"), "{text}");
        }
        // A week says which day the weeks start on, and moves with it.
        let sunday = Range::ThisWeek(WeekStart::Sunday);
        let text = screen(&app_for(sunday, &[]), 120, 30);
        assert!(text.contains("UTC, weeks start Sun"), "{text}");
        assert!(
            text.contains("this week 08-30 to 09-03") && text.contains("last week 08-23 to 08-27"),
            "{text}"
        );
    }

    #[test]
    fn the_context_line_gives_up_detail_before_the_same_span_note() {
        let c = Data::build(&[], LiveView::default(), NOW, Range::Today, "")
            .comparison
            .expect("today compares");
        let mut lengths = Vec::new();
        for room in [110, 80, 50, 40] {
            let (a, b) = context(&c, room);
            let len = a.chars().count() + b.chars().count();
            assert!(len <= room || room < 45, "{room}: {a}{b}");
            assert!(b.contains("same"), "{room}: {b}");
            lengths.push(len);
        }
        assert!(lengths.windows(2).all(|w| w[0] >= w[1]), "{lengths:?}");
    }

    #[test]
    fn the_footer_drops_the_least_important_hints_first_on_a_narrow_screen() {
        let app = App::new(Range::All);
        let wide = screen(&app, 120, 30);
        let narrow = screen(&app, 80, 24);
        assert!(
            wide.contains("x mouse") && wide.contains("t theme"),
            "{wide}"
        );
        assert!(
            !narrow.contains("x mouse") && !narrow.contains("t theme"),
            "{narrow}"
        );
        for kept in ["r range", "d/w/W/m/M/a jump", "/ filter", "q quit"] {
            assert!(narrow.contains(kept), "{kept}:\n{narrow}");
        }
    }

    #[test]
    fn the_unpriced_count_shows_in_the_header_border() {
        let mut e = event("mystery", "alpha", 0.0);
        e.unpriced = true;
        let app = app_with(&[e]);
        assert!(screen(&app, 80, 24).contains("1 unpriced"));
        assert!(screen(&app, 120, 30).contains("1 unpriced"));
    }

    #[test]
    fn a_resize_redraws_to_the_new_size() {
        let app = App::new(Range::All);
        assert!(screen(&app, 60, 20).contains("Terminal too small"));
        assert!(!screen(&app, 100, 30).contains("Terminal too small"));
    }
}
