// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whole-screen snapshots of the view at fixed sizes, from a fixed set of
//! events and a fixed clock. A snapshot is plain text in
//! `tests/fixtures/usage_tui/`. After an intended layout change, regenerate
//! them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and
//! review the diff.

use super::app::App;
use super::data::Data;
use super::theme::{ColorDepth, Theme};
use super::view::render;
use crate::usage::query::{live_view, Range};
use crate::usage::UsageEvent;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;
use std::path::PathBuf;

const NOW: i64 = 1_788_400_000;
const DAY: i64 = 86_400;

fn event(age_secs: i64, session: &str, model: &str, repo: &str, cost: f64) -> UsageEvent {
    UsageEvent {
        event_id: format!("{age_secs}{session}{model}"),
        timestamp: NOW - age_secs,
        session_id: session.into(),
        model: model.into(),
        repo: repo.into(),
        branch: "main".into(),
        input_tokens: 3_000,
        output_tokens: 800,
        cache_read_tokens: 20_000,
        cost_usd: cost,
        ..UsageEvent::default()
    }
}

fn fixture() -> Vec<UsageEvent> {
    vec![
        event(20, "s-live", "claude-opus-5", "alpha", 0.75),
        event(300, "s-live", "claude-opus-5", "alpha", 0.5),
        event(2 * DAY, "s-old", "claude-sonnet-5", "beta", 1.25),
        event(9 * DAY, "s-old", "claude-opus-5", "beta", 6.0),
        event(20 * DAY, "s-older", "claude-haiku-5", "gamma", 0.1),
    ]
}

fn app(theme: Theme) -> App {
    let events = fixture();
    let live = live_view(&events, &events, &events, NOW);
    let mut app = App::new(Range::LastDays(30));
    app.theme = theme;
    app.data = Some(Data::build(&events, live, NOW, Range::LastDays(30), ""));
    app
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| render(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows: Vec<String> = (0..height)
        .map(|y| {
            let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            row.trim_end().to_string()
        })
        .collect();
    rows.join("\n") + "\n"
}

fn check(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/usage_tui")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing snapshot {}: {e}", path.display()));
    assert_eq!(
        actual, want,
        "snapshot {name} changed; rerun with UPDATE_SNAPSHOTS=1 if intended"
    );
}

#[test]
fn full_screen_100x30() {
    check("full_100x30", &draw(&app(Theme::Btop), 100, 30));
}

#[test]
fn minimum_screen_80x24() {
    check("min_80x24", &draw(&app(Theme::Btop), 80, 24));
}

#[test]
fn wide_screen_160x50() {
    check("wide_160x50", &draw(&app(Theme::Btop), 160, 50));
}

#[test]
fn too_small_screen() {
    check("too_small_60x20", &draw(&app(Theme::Btop), 60, 20));
}

#[test]
fn before_the_first_read() {
    check("loading_100x30", &draw(&App::new(Range::All), 100, 30));
}

#[test]
fn editing_the_filter() {
    let mut a = app(Theme::Btop);
    a.editing = true;
    a.draft = "opus".into();
    check("editing_100x30", &draw(&a, 100, 30));
}

#[test]
fn the_theme_changes_color_not_text() {
    let btop = draw(&app(Theme::Btop), 100, 30);
    let dark = draw(&app(Theme::Dark), 100, 30);
    let light = draw(&app(Theme::Light), 100, 30);
    let mono = draw(&app(Theme::Mono), 100, 30);
    // Only the footer's theme name differs.
    assert_eq!(btop.replace("(btop)", "(x)"), dark.replace("(dark)", "(x)"));
    assert_eq!(
        dark.replace("(dark)", "(x)"),
        light.replace("(light)", "(x)")
    );
    assert_eq!(dark.replace("(dark)", "(x)"), mono.replace("(mono)", "(x)"));
}

fn buffer(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The first cell on `row` whose symbol is `symbol`.
fn find(buf: &Buffer, symbol: &str, row: u16) -> (u16, u16) {
    let x = (0..buf.area.width)
        .find(|x| buf[(*x, row)].symbol() == symbol)
        .unwrap_or_else(|| panic!("no {symbol} on row {row}"));
    (x, row)
}

#[test]
fn btop_paints_a_black_ground_and_gray_text() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    // Every cell has the black ground.
    assert!(buf.content().iter().all(|c| c.bg == rgb(0, 0, 0)
        || c.bg == rgb(0x6a, 0x2f, 0x2f)
        || c.bg == rgb(0xb5, 0x40, 0x40)));
    // The header's totals use the main text color.
    let (x, y) = find(&buf, "$", 1);
    assert_eq!(buf[(x, y)].fg, rgb(0xcc, 0xcc, 0xcc));
}

#[test]
fn each_box_has_its_own_outline_color() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    let corner = |x: u16, y: u16| {
        assert_eq!(
            buf[(x, y)].symbol(),
            "\u{256d}",
            "rounded corner at {x},{y}"
        );
        buf[(x, y)].fg
    };
    assert_eq!(corner(0, 0), rgb(0x55, 0x6d, 0x59)); // header and spend: cpu
    assert_eq!(corner(0, 3), rgb(0x55, 0x6d, 0x59)); // spend
    assert_eq!(corner(50, 3), rgb(0x5c, 0x58, 0x8d)); // tokens: net
    assert_eq!(corner(0, 11), rgb(0x6c, 0x6c, 0x4b)); // models: mem
    assert_eq!(corner(50, 11), rgb(0x6c, 0x6c, 0x4b)); // projects: mem
    assert_eq!(corner(0, 19), rgb(0x80, 0x52, 0x52)); // sessions: proc
    assert_eq!(corner(50, 19), rgb(0x80, 0x52, 0x52)); // events: proc
}

#[test]
fn titles_carry_a_hint_in_the_accent_color() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    // Spend sits at row 3, the hint after the corner, a dash and the `┤`.
    let (x, y) = find(&buf, "\u{b9}", 3);
    assert_eq!(buf[(x, y)].fg, rgb(0xb5, 0x40, 0x40));
    assert_eq!(buf[(x - 1, y)].symbol(), "\u{2524}");
    assert_eq!(buf[(x + 1, y)].fg, rgb(0xee, 0xee, 0xee));
    // The focused panel (models, hint 3) wears a badge: title on accent.
    let (x, y) = find(&buf, "\u{b3}", 11);
    assert_eq!(buf[(x, y)].bg, rgb(0xb5, 0x40, 0x40));
}

#[test]
fn the_graph_runs_the_gradient_from_start_toward_end() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    // Collect the colors of braille cells inside the spend panel.
    let mut colors: Vec<Color> = Vec::new();
    for y in 4..11 {
        for x in 1..49 {
            let c = &buf[(x, y)];
            if c.symbol()
                .chars()
                .next()
                .is_some_and(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
            {
                colors.push(c.fg);
            }
        }
    }
    assert!(!colors.is_empty());
    // The peak day is the end color, the small days are nearer the start.
    assert!(colors.contains(&rgb(0xdc, 0x4c, 0x4c)), "{colors:?}");
    assert!(colors.iter().any(|c| *c != rgb(0xdc, 0x4c, 0x4c)));
}

#[test]
fn the_selected_row_of_the_focused_panel_is_highlighted() {
    let a = app(Theme::Btop);
    let buf = buffer(&a, 100, 30);
    // Models is focused: its first data row (row 13) has the selection ground.
    assert_eq!(buf[(2, 13)].bg, rgb(0x6a, 0x2f, 0x2f));
    assert_eq!(buf[(2, 13)].fg, rgb(0xee, 0xee, 0xee));
    assert_eq!(buf[(2, 14)].bg, rgb(0, 0, 0));
    // The projects panel is not focused, so it shows no selection.
    assert_eq!(buf[(52, 13)].bg, rgb(0, 0, 0));
}

#[test]
fn meters_use_the_gradient_and_the_dim_color_for_the_rest() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    // The haiku share is tiny: a dim dot at the end of its row (row 15).
    let (x, y) = find(&buf, "\u{b7}", 15);
    assert_eq!(buf[(x, y)].fg, rgb(0x40, 0x40, 0x40));
    // The opus share fills most of its row with colored squares.
    let squares: Vec<Color> = (0..50)
        .filter(|x| buf[(*x, 13)].symbol() == "\u{25a0}")
        .map(|x| buf[(x, 13)].fg)
        .collect();
    assert!(squares.len() >= 6, "{squares:?}");
    assert_ne!(squares[0], squares[squares.len() - 1]);
}

#[test]
fn a_256_color_terminal_gets_indexed_colors_only() {
    let mut a = app(Theme::Btop);
    a.depth = ColorDepth::Ansi256;
    let buf = buffer(&a, 100, 30);
    for c in buf.content() {
        for color in [c.fg, c.bg] {
            assert!(!matches!(color, Color::Rgb(..)), "{color:?}");
        }
    }
    assert!(buf
        .content()
        .iter()
        .any(|c| matches!(c.fg, Color::Indexed(_))));
}

#[test]
fn a_16_color_terminal_gets_named_colors_only() {
    let mut a = app(Theme::Btop);
    a.depth = ColorDepth::Ansi16;
    let buf = buffer(&a, 100, 30);
    for c in buf.content() {
        for color in [c.fg, c.bg] {
            assert!(
                !matches!(color, Color::Rgb(..) | Color::Indexed(_)),
                "{color:?}"
            );
        }
    }
}

#[test]
fn mono_draws_no_color_at_all_but_still_marks_the_selection() {
    let buf = buffer(&app(Theme::Mono), 100, 30);
    assert!(buf
        .content()
        .iter()
        .all(|c| c.fg == Color::Reset && c.bg == Color::Reset));
    assert!(buf[(2, 13)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED));
}

#[test]
fn the_dark_theme_keeps_the_terminal_ground() {
    let buf = buffer(&app(Theme::Dark), 100, 30);
    assert!(buf
        .content()
        .iter()
        .all(|c| matches!(c.bg, Color::Reset | Color::Blue | Color::Cyan)));
}
