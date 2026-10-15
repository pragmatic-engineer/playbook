// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The Projection panel: for each known limit, one plain line saying when it
//! will be reached at the current pace, or that it will not be reached in
//! this period. A spending budget shows two paces side by side, the last 24
//! hours and the average of the period, so a recent spike and a steady month
//! can be told apart. A usage window (5 hour, weekly) only has its average,
//! because Claude Code reports a percentage and not a history.

use super::data::Data;
use super::fmt::{fit, money};
use super::theme::Palette;
use crate::limits::{Limit, Unit, MIN_PROJECTION_SECONDS};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

const DAY: f64 = 86_400.0;

/// The trailing window the recent pace is measured over.
pub const RECENT_SECONDS: i64 = 24 * 3600;

/// What a pace means for a limit.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    /// The limit is already reached.
    Reached,
    /// Nothing was used in the span, so nothing can be projected.
    Idle,
    /// The limit is reached at this UTC second, before the period ends.
    At(i64),
    /// The period ends first.
    Safe,
}

fn outcome(remaining: f64, pace_per_day: f64, now: i64, end: i64) -> Outcome {
    if remaining <= 0.0 {
        return Outcome::Reached;
    }
    if !(pace_per_day.is_finite() && pace_per_day > 0.0) {
        return Outcome::Idle;
    }
    // Compare in seconds as a float first, so a tiny pace cannot overflow.
    let wait = (remaining / pace_per_day * DAY).ceil();
    if wait < (end - now) as f64 {
        Outcome::At(now + wait as i64)
    } else {
        Outcome::Safe
    }
}

/// The limit's name with a capital letter, as the line starts with it.
fn heading(l: &Limit) -> String {
    let n = super::budget::name(l);
    let mut chars = n.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// One clause: the pace, and what it leads to.
fn clause(label: &str, pace: Option<f64>, unit: Unit, o: Outcome) -> String {
    let rate = |n: f64| match unit {
        Unit::Dollars => format!("{}/day", money(n)),
        Unit::Percent => format!("{n:.1}%/day"),
    };
    if o == Outcome::Reached {
        return format!("{label}: limit reached");
    }
    let Some(pace) = pace else {
        return format!("{label}: no pace yet");
    };
    let pace_text = rate(pace);
    match o {
        Outcome::Reached | Outcome::Idle => format!("{label} {pace_text}: no hit"),
        Outcome::At(at) => format!("{label} {pace_text}: hit {}", super::budget::stamp(at)),
        Outcome::Safe => format!("{label} {pace_text}: won't hit before it resets"),
    }
}

/// The plain line for one limit. `recent_cost` is the spend of the last
/// 24 hours, all projects, used for dollar budgets that are older than a day.
pub fn line(l: &Limit, recent_cost: f64, now: i64) -> String {
    let used = match l.unit {
        Unit::Dollars => format!("{} of {}", money(l.used), money(l.limit)),
        Unit::Percent => format!("{:.0}% used", l.used),
    };
    let head = format!("{}, {used}.", heading(l));
    let Some(h) = l.headroom(now) else {
        return head;
    };
    let remaining = h.remaining;
    // Like the Budget panel, wait for an hour of the period before projecting.
    let avg = h
        .avg_per_day
        .filter(|_| h.elapsed_seconds >= MIN_PROJECTION_SECONDS);
    let avg_label = match l.unit {
        Unit::Dollars => "period average",
        Unit::Percent => "window average",
    };
    let avg_clause = clause(
        avg_label,
        avg,
        l.unit,
        outcome(remaining, avg.unwrap_or(0.0), now, l.end),
    );
    if l.unit == Unit::Percent {
        return format!("{head} {avg_clause}.");
    }
    // The recent pace covers the last 24 hours. A period younger than that
    // uses its own spend over its own age, so spend from before the period
    // began never counts. Under an hour there is too little to say.
    let age = now - l.start;
    let (spent, span) = if age < RECENT_SECONDS {
        (l.used, age)
    } else {
        (recent_cost, RECENT_SECONDS)
    };
    let recent = (span >= MIN_PROJECTION_SECONDS).then(|| spent / (span as f64 / DAY));
    let recent_clause = clause(
        "last 24h",
        recent,
        l.unit,
        outcome(remaining, recent.unwrap_or(0.0), now, l.end),
    );
    format!("{head} {recent_clause}. {avg_clause}.")
}

/// Every limit's line, worst first as the limits are sorted.
pub fn lines(limits: &[Limit], recent_cost: f64, now: i64) -> Vec<String> {
    limits.iter().map(|l| line(l, recent_cost, now)).collect()
}

pub const EMPTY: [&str; 2] = [
    "No limit is known, so there is nothing to project.",
    "Set one: playbook config set --global usage.budget.monthlyUsd <usd>",
];

pub fn render(frame: &mut Frame, area: Rect, data: &Data, pal: &Palette) {
    if area.height == 0 {
        return;
    }
    let edge = Style::default().fg(pal.color(pal.box_cpu));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(edge)
        .title(Line::from(vec![
            Span::styled("\u{2500}\u{2524}", edge),
            Span::styled("Projection", Style::default().fg(pal.color(pal.title))),
            Span::styled("\u{251c}", edge),
        ]))
        .title(
            Line::from(vec![
                Span::styled("\u{2524}", edge),
                Span::styled(
                    format!("at the current pace, {}", crate::tz::label()),
                    Style::default().fg(pal.color(pal.fg)),
                ),
                Span::styled("\u{251c}\u{2500}", edge),
            ])
            .right_aligned(),
        );
    let width = usize::from(area.width.saturating_sub(2));
    let v = &data.limits;
    let dim = Style::default().fg(pal.color(pal.inactive));
    let mut text: Vec<Line> = if v.limits.is_empty() {
        EMPTY
            .iter()
            .map(|t| Line::styled(fit(t, width), dim))
            .collect()
    } else {
        lines(&v.limits, v.recent_cost_24h, data.now)
            .into_iter()
            .map(|t| Line::styled(fit(&t, width), Style::default().fg(pal.color(pal.fg))))
            .collect()
    };
    if let Some(note) = v.windows_note {
        text.push(Line::styled(fit(note, width), dim));
    }
    frame.render_widget(Paragraph::new(text).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{LimitPeriod, Origin};

    const NOW: i64 = 1_788_400_000;
    const DAYS: i64 = 86_400;

    fn budget(used: f64, limit: f64, start: i64, end: i64) -> Limit {
        Limit {
            unit: Unit::Dollars,
            period: LimitPeriod::Monthly,
            origin: Origin::Manual,
            used,
            limit,
            start,
            end,
        }
    }

    #[test]
    fn a_fast_recent_pace_hits_the_limit_while_the_slow_average_stays_under() {
        crate::tz::set(crate::tz::Zone::Utc);
        let l = budget(300.0, 1000.0, NOW - 15 * DAYS, NOW + 15 * DAYS);

        let text = line(&l, 100.0, NOW);

        assert!(
            text.contains("last 24h $100.00/day: hit 09-10 01:46"),
            "{text}"
        );
        assert!(
            text.contains("period average $20.00/day: won't hit before it resets"),
            "{text}"
        );
    }

    #[test]
    fn a_limit_already_used_up_says_it_is_reached() {
        let l = budget(1000.0, 1000.0, NOW - 15 * DAYS, NOW + 15 * DAYS);

        let text = line(&l, 10.0, NOW);

        assert!(text.contains("last 24h: limit reached"), "{text}");
        assert!(text.contains("period average: limit reached"), "{text}");
    }

    #[test]
    fn no_spend_in_the_last_day_is_idle_and_not_a_hit() {
        let l = budget(100.0, 1000.0, NOW - 15 * DAYS, NOW + 15 * DAYS);

        let text = line(&l, 0.0, NOW);

        assert!(text.contains("last 24h $0.00/day: no hit"), "{text}");
    }

    #[test]
    fn a_period_younger_than_an_hour_has_no_recent_pace_yet() {
        let l = budget(5.0, 1000.0, NOW - 600, NOW + 30 * DAYS);

        let text = line(&l, 5.0, NOW);

        assert!(text.contains("last 24h: no pace yet"), "{text}");
    }

    #[test]
    fn a_period_younger_than_a_day_uses_its_own_spend_and_age_for_the_recent_pace() {
        let l = budget(6.0, 1000.0, NOW - 12 * 3600, NOW + 30 * DAYS);

        let text = line(&l, 12.0, NOW);

        assert!(text.contains("last 24h $12.00/day"), "{text}");
    }

    #[test]
    fn a_period_younger_than_an_hour_has_no_average_pace_yet() {
        let l = budget(2.0, 300.0, NOW - 600, NOW + 30 * DAYS);

        let text = line(&l, 2.0, NOW);

        assert!(text.contains("period average: no pace yet"), "{text}");
    }

    #[test]
    fn a_tiny_pace_does_not_overflow_the_hit_time() {
        let l = budget(1e-300, 1000.0, NOW - 15 * DAYS, NOW + 15 * DAYS);

        let text = line(&l, 0.0, NOW);

        assert!(text.contains("won't hit before it resets"), "{text}");
    }

    #[test]
    fn a_young_period_that_used_up_its_limit_says_so() {
        let l = budget(1000.0, 1000.0, NOW - 600, NOW + 30 * DAYS);

        let text = line(&l, 1000.0, NOW);

        assert!(text.contains("last 24h: limit reached"), "{text}");
    }

    #[test]
    fn a_usage_window_shows_only_its_average_in_percent() {
        crate::tz::set(crate::tz::Zone::Utc);
        let l = Limit {
            unit: Unit::Percent,
            period: LimitPeriod::FiveHour,
            origin: Origin::Measured,
            used: 50.0,
            limit: 100.0,
            start: NOW - 3 * 3600,
            end: NOW + 2 * 3600,
        };

        let text = line(&l, 0.0, NOW);

        assert!(text.starts_with("5 hour window, 50% used."), "{text}");
        assert!(text.contains("window average"), "{text}");
        assert!(!text.contains("last 24h"), "{text}");
    }

    #[test]
    fn no_limits_give_no_lines() {
        assert!(lines(&[], 0.0, NOW).is_empty());
    }
}
