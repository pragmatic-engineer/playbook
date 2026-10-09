// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whole-screen snapshots of the view at fixed sizes, from a fixed set of
//! events and a fixed clock. A snapshot is plain text in
//! `tests/fixtures/usage_tui/`. After an intended layout change, regenerate
//! them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and
//! review the diff.

use super::app::App;
use super::data::Data;
use super::theme::Theme;
use super::view::render;
use crate::usage::query::{live_view, Range};
use crate::usage::UsageEvent;
use ratatui::backend::TestBackend;
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
    check("full_100x30", &draw(&app(Theme::Dark), 100, 30));
}

#[test]
fn minimum_screen_80x24() {
    check("min_80x24", &draw(&app(Theme::Dark), 80, 24));
}

#[test]
fn wide_screen_160x50() {
    check("wide_160x50", &draw(&app(Theme::Dark), 160, 50));
}

#[test]
fn too_small_screen() {
    check("too_small_60x20", &draw(&app(Theme::Dark), 60, 20));
}

#[test]
fn before_the_first_read() {
    check("loading_100x30", &draw(&App::new(Range::All), 100, 30));
}

#[test]
fn editing_the_filter() {
    let mut a = app(Theme::Dark);
    a.editing = true;
    a.draft = "opus".into();
    check("editing_100x30", &draw(&a, 100, 30));
}

#[test]
fn the_theme_changes_color_not_text() {
    let dark = draw(&app(Theme::Dark), 100, 30);
    let light = draw(&app(Theme::Light), 100, 30);
    let mono = draw(&app(Theme::Mono), 100, 30);
    // Only the footer's theme name differs.
    assert_eq!(
        dark.replace("(dark)", "(x)"),
        light.replace("(light)", "(x)")
    );
    assert_eq!(dark.replace("(dark)", "(x)"), mono.replace("(mono)", "(x)"));
}
