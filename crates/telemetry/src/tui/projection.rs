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
use crate::aggregate::date_key;
use crate::limits::{Limit, LimitPeriod, Unit};
use crate::query::{clock, day_of};
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
    let at = now + (remaining / pace_per_day * DAY).ceil() as i64;
    if at < end {
        Outcome::At(at)
    } else {
        Outcome::Safe
    }
}

/// `MM-DD HH:MM` in the chosen zone. The panel title names the zone.
fn when(at: i64) -> String {
    let date = date_key(day_of(at));
    format!("{} {}", &date[5..], &clock(at)[..5])
}

fn name(l: &Limit) -> &'static str {
    match (l.period, l.unit) {
        (LimitPeriod::FiveHour, _) => "5 hour window",
        (LimitPeriod::SevenDay, _) => "Weekly window",
        (LimitPeriod::Daily, _) => "Daily budget",
        (LimitPeriod::Weekly, Unit::Percent) => "Weekly spend limit",
        (LimitPeriod::Monthly, Unit::Percent) => "Monthly spend limit",
        (LimitPeriod::Weekly, _) => "Weekly budget",
        (LimitPeriod::Monthly, _) => "Monthly budget",
    }
}

/// One clause: the pace, and what it leads to.
fn clause(label: &str, pace: Option<f64>, unit: Unit, o: Outcome, end: i64) -> String {
    let rate = |n: f64| match unit {
        Unit::Dollars => format!("{}/day", money(n)),
        Unit::Percent => format!("{n:.1}%/day"),
    };
    let pace_text = pace.map_or_else(|| "no pace yet".to_string(), rate);
    match o {
        Outcome::Reached => format!("{label}: limit reached"),
        Outcome::Idle => format!("{label} {pace_text}: nothing used, no hit"),
        Outcome::At(at) => format!("{label} {pace_text}: hit {}", when(at)),
        Outcome::Safe => format!(
            "{label} {pace_text}: no hit before the reset at {}",
            when(end)
        ),
    }
}

/// The plain line for one limit. `recent_cost` is the spend of the last
/// 24 hours, all projects, used for dollar budgets.
pub fn line(l: &Limit, recent_cost: f64, now: i64) -> String {
    let used = match l.unit {
        Unit::Dollars => format!("{} of {}", money(l.used), money(l.limit)),
        Unit::Percent => format!("{:.0}% used", l.used),
    };
    let head = format!("{}, {used}.", name(l));
    let Some(h) = l.headroom(now) else {
        return head;
    };
    let remaining = h.remaining;
    let avg = h.avg_per_day;
    let avg_label = match l.unit {
        Unit::Dollars => "period average",
        Unit::Percent => "window average",
    };
    let avg_clause = clause(
        avg_label,
        avg,
        l.unit,
        avg.map_or(Outcome::Idle, |p| outcome(remaining, p, now, l.end)),
        l.end,
    );
    if l.unit == Unit::Percent {
        return format!("{head} {avg_clause}.");
    }
    // The recent pace covers the last 24 hours, or less when the period is
    // younger than that. Under an hour there is too little to say.
    let span = RECENT_SECONDS.min(now - l.start);
    let recent = (span >= 3600).then(|| recent_cost / (span as f64 / DAY));
    let recent_clause = clause(
        "last 24h",
        recent,
        l.unit,
        recent.map_or(Outcome::Idle, |p| outcome(remaining, p, now, l.end)),
        l.end,
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
    let text: Vec<Line> = if v.limits.is_empty() {
        EMPTY
            .iter()
            .map(|t| Line::styled(fit(t, width), Style::default().fg(pal.color(pal.inactive))))
            .collect()
    } else {
        lines(&v.limits, v.recent_cost_24h, data.now)
            .into_iter()
            .map(|t| Line::styled(fit(&t, width), Style::default().fg(pal.color(pal.fg))))
            .collect()
    };
    frame.render_widget(Paragraph::new(text).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Origin;

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

        assert!(text.contains("last 24h $100.00/day: hit "), "{text}");
        assert!(
            text.contains("period average $20.00/day: no hit before the reset at"),
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

        assert!(text.contains("last 24h $0.00/day: nothing used"), "{text}");
    }

    #[test]
    fn a_period_younger_than_an_hour_has_no_recent_pace_yet() {
        let l = budget(5.0, 1000.0, NOW - 600, NOW + 30 * DAYS);

        let text = line(&l, 5.0, NOW);

        assert!(text.contains("last 24h no pace yet"), "{text}");
    }

    #[test]
    fn a_period_younger_than_a_day_measures_the_recent_pace_over_its_own_age() {
        let l = budget(12.0, 1000.0, NOW - 12 * 3600, NOW + 30 * DAYS);

        let text = line(&l, 12.0, NOW);

        assert!(text.contains("last 24h $24.00/day"), "{text}");
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
