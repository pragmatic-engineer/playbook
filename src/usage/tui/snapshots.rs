// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whole-screen snapshots of the view at fixed sizes, from a fixed set of
//! events and a fixed clock. A snapshot is plain text in
//! `tests/fixtures/usage_tui/`. After an intended layout change, regenerate
//! them with `UPDATE_SNAPSHOTS=1 cargo test --lib usage::tui::snapshots` and
//! review the diff.

use super::app::App;
use super::data::Data;
use super::sort::Panel;
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
    let mut events = vec![
        event(20, "s-live", "claude-opus-5", "alpha", 0.75),
        event(300, "s-live", "claude-opus-5", "alpha", 0.5),
        event(2 * DAY, "s-old", "claude-sonnet-5", "beta", 1.25),
        event(9 * DAY, "s-old", "claude-opus-5", "beta", 6.0),
        event(20 * DAY, "s-older", "claude-haiku-5", "gamma", 0.1),
    ];
    // A few more days of synthetic spend so the charts have a shape. Every
    // fifth day is idle. Haiku stays the smallest model by a wide margin.
    for d in 1..30_i64 {
        if d % 5 == 0 || [2, 9, 20].contains(&d) {
            continue;
        }
        let base = [0.4, 1.1, 2.6, 0.8, 1.9, 0.3, 1.4][(d % 7) as usize];
        let (model, repo, cost) = match d % 3 {
            0 => ("claude-opus-5", "alpha", base),
            1 => ("claude-sonnet-5", "beta", base / 2.0),
            _ => ("claude-haiku-5", "gamma", base / 100.0),
        };
        events.push(event(
            d * DAY + 3 * 3_600,
            &format!("s-{d}"),
            model,
            repo,
            cost,
        ));
    }
    events
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
fn the_week_range_has_wide_bars_and_weekday_names() {
    let mut a = app(Theme::Btop);
    let events = fixture();
    let live = live_view(&events, &events, &events, NOW);
    let range = Range::ThisWeek(crate::usage::query::WeekStart::Monday);
    a.range = range;
    a.data = Some(Data::build(&events, live, NOW, range, ""));
    check("week_160x50", &draw(&a, 160, 50));
}

#[test]
fn today_compares_with_yesterday_and_charts_the_last_seven_days() {
    let mut a = app(Theme::Btop);
    let events = fixture();
    let live = live_view(&events, &events, &events, NOW);
    a.range = Range::Today;
    a.data = Some(Data::build(&events, live, NOW, Range::Today, ""));
    check("today_100x30", &draw(&a, 100, 30));
}

#[test]
fn a_moved_cursor_reads_out_its_day() {
    let mut a = app(Theme::Btop);
    a.cursor = Some(NOW.div_euclid(DAY) - 9);
    let text = draw(&a, 100, 30);
    assert!(text.contains("Tue 2026-08-25"), "{text}");
}

fn with_spend_view(theme: Theme, view: crate::usage::tui::app::SpendView) -> App {
    let mut a = app(theme);
    a.spend_view = view;
    a
}

#[test]
fn spend_stacked_by_model_100x30() {
    use crate::usage::tui::app::SpendView;
    check(
        "stacked_100x30",
        &draw(&with_spend_view(Theme::Btop, SpendView::ByModel), 100, 30),
    );
}

#[test]
fn spend_stacked_by_model_without_color_uses_patterns() {
    use crate::usage::tui::app::SpendView;
    let text = draw(&with_spend_view(Theme::Mono, SpendView::ByModel), 100, 30);
    let shown = crate::usage::tui::chart::PATTERNS
        .iter()
        .filter(|c| text.contains(**c))
        .count();
    assert!(shown >= 3, "three models, three patterns, saw {shown}");
}

#[test]
fn the_daily_table_at_100x30() {
    use crate::usage::tui::app::SpendView;
    check(
        "daily_100x30",
        &draw(&with_spend_view(Theme::Btop, SpendView::Daily), 100, 30),
    );
}

#[test]
fn a_model_wears_the_same_color_in_the_table_and_the_stacked_bars() {
    use crate::usage::tui::app::SpendView;
    let a = with_spend_view(Theme::Btop, SpendView::ByModel);
    let buf = buffer(&a, 100, 30);
    let p = panels();
    let data = a.data.as_ref().unwrap();
    let pal = Theme::Btop.palette(ColorDepth::True);
    // The Models panel is sorted by cost: opus, sonnet, haiku.
    let rows: Vec<Color> = (0..3)
        .map(|i| buf[(p.models.x + 1, p.models.y + 2 + i)].fg)
        .collect();
    for (i, model) in ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            buf[(p.models.x + 1, p.models.y + 2 + i as u16)].symbol(),
            "\u{25a0}"
        );
        assert_eq!(
            rows[i],
            pal.color(pal.series[data.slots.slot(model)]),
            "{model}"
        );
    }
    let bars: std::collections::BTreeSet<String> = (p.spend.y + 1..p.spend.y + p.spend.height - 1)
        .flat_map(|y| (p.spend.x + 1..p.spend.x + p.spend.width - 1).map(move |x| (x, y)))
        .filter(|(x, y)| {
            ('\u{2581}'..='\u{2588}')
                .contains(&buf[(*x, *y)].symbol().chars().next().unwrap_or(' '))
        })
        .flat_map(|(x, y)| {
            [
                format!("{:?}", buf[(x, y)].fg),
                format!("{:?}", buf[(x, y)].bg),
            ]
        })
        .collect();
    // The two big models show up in the bars in the colors of their rows.
    for color in &rows[..2] {
        assert!(
            bars.contains(&format!("{color:?}")),
            "{color:?} not in {bars:?}"
        );
    }
    // The legend in the bottom border names them with the same swatches.
    let legend_row = p.spend.y + p.spend.height - 1;
    let legend: String = (p.spend.x..p.spend.x + p.spend.width)
        .map(|x| buf[(x, legend_row)].symbol())
        .collect();
    assert!(
        legend.contains("opus-5") && legend.contains("sonnet-5"),
        "{legend}"
    );
}

#[test]
fn the_daily_table_marks_today_and_the_peak_day() {
    use crate::usage::tui::app::SpendView;
    let mut a = with_spend_view(Theme::Btop, SpendView::Daily);
    let buf = buffer(&a, 100, 30);
    let p = panels();
    // Newest first, so today is the first row.
    let today = &buf[(p.spend.x + 2, p.spend.y + 2)];
    let hi = rgb(0xb5, 0x40, 0x40);
    assert_eq!(today.fg, hi);
    assert!(today
        .modifier
        .contains(ratatui::style::Modifier::UNDERLINED));
    // By cost, the peak day ($6.01) is the first row.
    a.sorts[Panel::Spend.index()] = crate::usage::tui::sort::Sort::new(3, true);
    let buf = buffer(&a, 100, 30);
    let peak_row = (p.spend.y + 2..p.spend.y + p.spend.height - 1)
        .find(|y| {
            let row: String = (p.spend.x..p.spend.x + p.spend.width)
                .map(|x| buf[(x, *y)].symbol())
                .collect();
            row.contains("$6.01")
        })
        .expect("the peak day is in the table");
    assert_eq!(buf[(p.spend.x + 2, peak_row)].fg, rgb(0xff, 0xb8, 0x14));
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
fn the_theme_changes_color_not_text() {
    let btop = draw(&app(Theme::Btop), 100, 30);
    let dark = draw(&app(Theme::Dark), 100, 30);
    let light = draw(&app(Theme::Light), 100, 30);
    let mono = draw(&app(Theme::Mono), 100, 30);
    // Only the footer differs: it names the theme.
    // Only the footer, which names the theme, and the legend, which the mono
    // theme leaves out, differ.
    let legend_row = usize::from(panels().spend.bottom() - 1);
    let body = |s: &str| {
        s.lines()
            .enumerate()
            .take(29)
            .filter(|(i, _)| *i != legend_row)
            // The models wear patterns, not colors, in mono.
            .map(|(_, l)| l.replace(crate::usage::tui::chart::PATTERNS, "\u{25a0}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(body(&btop), body(&dark));
    assert_eq!(body(&dark), body(&light));
    assert_eq!(body(&dark), body(&mono));
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
    let (x, y) = find(&buf, "$", 2);
    assert_eq!(buf[(x, y)].fg, rgb(0xcc, 0xcc, 0xcc));
}

fn panels() -> crate::usage::tui::view::Panels {
    crate::usage::tui::view::layout(ratatui::layout::Rect::new(0, 0, 100, 30))
}

#[test]
fn each_box_has_its_own_outline_color() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    let p = panels();
    let corner = |r: ratatui::layout::Rect| {
        assert_eq!(
            buf[(r.x, r.y)].symbol(),
            "\u{256d}",
            "rounded corner at {r:?}"
        );
        buf[(r.x, r.y)].fg
    };
    assert_eq!(corner(p.header), rgb(0x55, 0x6d, 0x59)); // header: cpu
    assert_eq!(corner(p.spend), rgb(0x55, 0x6d, 0x59)); // spend: cpu
    assert_eq!(corner(p.tokens), rgb(0x5c, 0x58, 0x8d)); // tokens: net
    assert_eq!(corner(p.models), rgb(0x6c, 0x6c, 0x4b)); // models: mem
    assert_eq!(corner(p.projects), rgb(0x6c, 0x6c, 0x4b)); // projects: mem
    assert_eq!(corner(p.live), rgb(0x80, 0x52, 0x52)); // live: proc
    assert_eq!(corner(p.sessions), rgb(0x80, 0x52, 0x52)); // sessions: proc
}

#[test]
fn titles_carry_a_hint_in_the_accent_color() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    let p = panels();
    // The hint follows the corner, a dash and the `┤`.
    let (x, y) = find(&buf, "\u{b9}", p.spend.y);
    assert_eq!(buf[(x, y)].fg, rgb(0xb5, 0x40, 0x40));
    assert_eq!(buf[(x - 1, y)].symbol(), "\u{2524}");
    assert_eq!(buf[(x + 1, y)].fg, rgb(0xee, 0xee, 0xee));
    // The focused panel (models, hint 3) wears a badge: title on accent.
    let (x, y) = find(&buf, "\u{b3}", p.models.y);
    assert_eq!(buf[(x, y)].bg, rgb(0xb5, 0x40, 0x40));
}

#[test]
fn the_bars_run_the_gradient_with_their_height_on_the_scale() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    let p = panels();
    // Collect the colors of bar cells inside the spend panel.
    let mut colors: Vec<Color> = Vec::new();
    for y in p.spend.y + 1..p.spend.y + p.spend.height - 1 {
        for x in p.spend.x + 1..p.spend.x + p.spend.width - 1 {
            let c = &buf[(x, y)];
            if c.symbol()
                .chars()
                .next()
                .is_some_and(|ch| ('\u{2581}'..='\u{2588}').contains(&ch))
            {
                colors.push(c.fg);
            }
        }
    }
    assert!(!colors.is_empty());
    // Taller days sit further along the gradient than small ones.
    let mut distinct = colors.clone();
    distinct.sort_by_key(|c| format!("{c:?}"));
    distinct.dedup();
    assert!(distinct.len() >= 3, "{distinct:?}");
}

#[test]
fn the_selected_row_of_the_focused_panel_is_highlighted() {
    let a = app(Theme::Btop);
    let buf = buffer(&a, 100, 30);
    let p = panels();
    let first = p.models.y + 2;
    // Models is focused: its first data row has the selection ground.
    assert_eq!(buf[(2, first)].bg, rgb(0x6a, 0x2f, 0x2f));
    assert_eq!(buf[(2, first)].fg, rgb(0xee, 0xee, 0xee));
    assert_eq!(buf[(2, first + 1)].bg, rgb(0, 0, 0));
    // The projects panel is not focused, so it shows no selection.
    assert_eq!(buf[(p.projects.x + 2, first)].bg, rgb(0, 0, 0));
}

#[test]
fn meters_use_the_gradient_and_the_dim_color_for_the_rest() {
    let buf = buffer(&app(Theme::Btop), 100, 30);
    let p = panels();
    let first = p.models.y + 2;
    // The haiku share is tiny: a dim dot at the end of its row.
    let (x, y) = find(&buf, "\u{b7}", first + 2);
    assert_eq!(buf[(x, y)].fg, rgb(0x40, 0x40, 0x40));
    // The opus share fills most of its row with colored squares.
    let squares: Vec<Color> = (0..50)
        .filter(|x| buf[(*x, first)].symbol() == "\u{25a0}")
        .map(|x| buf[(x, first)].fg)
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
    let first = panels().models.y + 2;
    assert!(buf[(2, first)]
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

fn agents_app(with_telemetry: bool) -> App {
    use crate::usage::aggregate::Group;
    use crate::usage::tui::app::ProjectsView;
    let mut a = app(Theme::Btop);
    a.projects_view = ProjectsView::Agents;
    if with_telemetry {
        let group = |key: &str, messages: u64, cost: f64| Group {
            key: key.into(),
            messages,
            input_tokens: 1_000 * messages,
            output_tokens: 500 * messages,
            cost_usd: cost,
            ..Group::default()
        };
        let data = a.data.as_mut().unwrap();
        data.agents.rows = vec![
            group("main", 40, 9.0),
            group("general-purpose", 12, 2.5),
            group("reviewer", 5, 0.5),
        ];
        data.agents.cost_usd = 12.0;
        data.telemetry_rows = true;
        data.skills.rows = vec![group("review", 9, 3.0), group("plan", 4, 1.0)];
        data.skills.cost_usd = 4.0;
    }
    a
}

#[test]
fn the_agents_view_lists_cost_per_agent_and_the_telemetry_share() {
    let text = draw(&agents_app(true), 100, 30);
    assert!(
        text.contains("Agents") && text.contains("reviewer"),
        "{text}"
    );
    assert!(text.contains("source telemetry"), "{text}");
    check("agents_100x30", &text);
}

#[test]
fn the_agents_view_says_telemetry_is_off_when_there_are_no_rows() {
    let text = draw(&agents_app(false), 100, 30);
    assert!(text.contains("Telemetry off"), "{text}");
    check("agents_off_100x30", &text);
}

#[test]
fn the_skills_view_lists_cost_per_skill_and_says_when_none_ran() {
    use crate::usage::tui::app::ProjectsView;
    let mut a = agents_app(true);
    a.projects_view = ProjectsView::Skills;
    let text = draw(&a, 100, 30);
    assert!(text.contains("Skills") && text.contains("review"), "{text}");
    check("skills_100x30", &text);

    a.data.as_mut().unwrap().skills = Default::default();
    let text = draw(&a, 100, 30);
    assert!(text.contains("No skill activity"), "{text}");
    assert!(!text.contains("Telemetry off"), "{text}");
}

#[test]
fn the_skills_view_says_telemetry_is_off_when_there_are_no_rows() {
    use crate::usage::tui::app::ProjectsView;
    let mut a = agents_app(false);
    a.projects_view = ProjectsView::Skills;
    assert!(draw(&a, 100, 30).contains("Telemetry off"));
}

#[test]
fn v_cycles_projects_agents_skills_and_back() {
    use crate::usage::tui::app::ProjectsView::*;
    assert_eq!(Projects.next().next().next().next().next(), Projects);
    assert_eq!(Agents.next(), Skills);
    assert_eq!(Skills.next(), Tools);
    assert_eq!(Tools.next(), Health);
}

fn health_app(with_telemetry: bool, view: crate::usage::tui::app::ProjectsView) -> App {
    use crate::usage::telemetry_db::{Health, ToolStat};
    let mut a = app(Theme::Btop);
    let events = fixture();
    let live = live_view(&events, &events, &events, NOW);
    a.range = Range::Today;
    let mut data = Data::build(&events, live, NOW, Range::Today, "");
    if with_telemetry {
        let t = |name: &str, calls: u64, failures: u64, total_ms: u64, max_ms: u64| ToolStat {
            name: name.into(),
            calls,
            failures,
            total_ms,
            max_ms,
        };
        data.telemetry_rows = true;
        data.health.tools = vec![
            t("Bash", 40, 4, 40 * 900, 12_000),
            t("Read", 120, 0, 120 * 8, 90),
            t("Edit", 30, 3, 30 * 15, 80),
        ];
        data.health.tools_before = vec![
            t("Bash", 30, 1, 30 * 600, 9_000),
            t("Read", 90, 0, 90 * 10, 70),
        ];
        data.health.now = Health {
            requests: 200,
            api_ms: 900_000,
            errors: 6,
            retried: 2,
            tool_calls: 190,
            tool_failures: 7,
            tool_ms: 37_000,
        };
        data.health.before = Some(Health {
            requests: 150,
            api_ms: 600_000,
            errors: 3,
            retried: 1,
            tool_calls: 120,
            tool_failures: 1,
            tool_ms: 20_000,
        });
    }
    a.data = Some(data);
    a.projects_view = view;
    a
}

#[test]
fn the_tools_view_shows_latency_and_error_rate_against_the_previous_period() {
    use crate::usage::tui::app::ProjectsView;
    let text = draw(&health_app(true, ProjectsView::Tools), 100, 30);
    assert!(text.contains("Tools") && text.contains("Bash"), "{text}");
    assert!(
        text.contains("vs yesterday") && text.contains("source telemetry"),
        "{text}"
    );
    check("tools_100x30", &text);
}

#[test]
fn the_health_view_shows_errors_retries_busy_time_and_cache_hit_against_the_previous_period() {
    use crate::usage::tui::app::ProjectsView;
    let text = draw(&health_app(true, ProjectsView::Health), 100, 30);
    assert!(
        text.contains("Health") && text.contains("Retries"),
        "{text}"
    );
    // A tall terminal shows every figure, the cache hit rate from transcripts too.
    let tall = draw(&health_app(true, ProjectsView::Health), 160, 50);
    assert!(
        tall.contains("Cache hit") && tall.contains("Busy time"),
        "{tall}"
    );
    check("health_100x30", &text);
}

#[test]
fn the_tools_and_health_views_say_telemetry_is_off_when_there_are_no_rows() {
    use crate::usage::tui::app::ProjectsView;
    for view in [ProjectsView::Tools, ProjectsView::Health] {
        let text = draw(&health_app(false, view), 100, 30);
        assert!(text.contains("Telemetry off"), "{view:?}\n{text}");
    }
}

#[test]
fn the_tools_view_is_readable_at_the_minimum_size_and_without_a_comparison() {
    use crate::usage::tui::app::ProjectsView;
    let a = health_app(true, ProjectsView::Tools);
    assert!(draw(&a, 80, 24).contains("Bash"));
    let mut b = app(Theme::Btop);
    b.projects_view = ProjectsView::Health;
    let data = b.data.as_mut().unwrap();
    data.telemetry_rows = true;
    data.health.now.requests = 5;
    let text = draw(&b, 100, 30);
    assert!(
        text.contains("API errors") && !text.contains("before"),
        "{text}"
    );
}
