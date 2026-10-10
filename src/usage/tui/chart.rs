// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A bar chart with axes, for a spend or token series: eighth blocks for the
//! bars, a y axis with round values, an x axis with dates and weekdays, a
//! cursor that reads one bar out, and a line of totals.
//!
//! From the top, inside the box: a totals line, the cursor readout, the plot,
//! a row of weekday letters, a row of dates. Rows are dropped from the least
//! useful when the panel is short, and the y axis when it is narrow, so the
//! chart still draws at any size. `Plot` is the geometry and the data; drawing
//! and mouse hits both use it, so a click lands on the bar it points at.

use super::data::DayPoint;
use super::fmt::{axis_money, axis_tokens, compact, fit, money};
use super::scale::Axis;
use super::theme::{Fill, Palette};
use crate::usage::aggregate::date_key;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use std::collections::BTreeMap;

/// Vertical eighth blocks, one to eight eighths of a cell.
const EIGHTHS: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];

/// The shortest plot worth drawing, in rows.
const MIN_PLOT: u16 = 3;
/// A bar is never drawn wider than this, so a few days do not become slabs.
const MAX_CELL: usize = 8;
/// The narrowest inner width that keeps a y axis.
const MIN_AXIS_WIDTH: u16 = 16;

/// What a chart plots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cost,
    Tokens,
}

impl Metric {
    /// A value as it reads in text: `$1.25` or `23.8K`.
    pub fn show(self, v: f64) -> String {
        match self {
            Metric::Cost => money(v),
            Metric::Tokens => compact(v.round() as u64),
        }
    }

    fn axis_label(self, v: f64) -> String {
        match self {
            Metric::Cost => axis_money(v),
            Metric::Tokens => axis_tokens(v),
        }
    }
}

/// One bar: a day, or several days added up when the range is wider than the
/// plot.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub first: i64,
    pub last: i64,
    pub cost: f64,
    pub tokens: u64,
    pub messages: usize,
    /// Cost per model, by model name.
    pub models: Vec<(String, f64)>,
}

impl Bar {
    pub fn value(&self, metric: Metric) -> f64 {
        match metric {
            Metric::Cost => self.cost,
            Metric::Tokens => self.tokens as f64,
        }
    }

    pub fn days(&self) -> i64 {
        self.last - self.first + 1
    }

    pub fn contains(&self, day: i64) -> bool {
        (self.first..=self.last).contains(&day)
    }

    /// The model that cost the most, with its share of the bar's cost.
    pub fn top_model(&self) -> Option<(&str, f64)> {
        let (name, cost) = self
            .models
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
        (self.cost > 0.0).then(|| (name.as_str(), cost / self.cost))
    }
}

/// The days of `series` in groups of `per`, each added up into a bar.
pub fn bars(series: &[DayPoint], per: usize) -> Vec<Bar> {
    series
        .chunks(per.max(1))
        .map(|days| {
            let mut models: BTreeMap<&str, f64> = BTreeMap::new();
            for d in days {
                for (name, cost) in &d.models {
                    *models.entry(name.as_str()).or_default() += cost;
                }
            }
            Bar {
                first: days[0].day,
                last: days[days.len() - 1].day,
                cost: days.iter().map(|d| d.cost_usd).sum(),
                tokens: days.iter().map(|d| d.tokens).sum(),
                messages: days.iter().map(|d| d.messages).sum(),
                models: models
                    .into_iter()
                    .map(|(name, cost)| (name.to_string(), cost))
                    .collect(),
            }
        })
        .collect()
}

/// Which rows the chart shows, for an inner height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows {
    pub stats: bool,
    pub dates: bool,
    pub plot: u16,
}

impl Rows {
    /// Keeps as many rows as fit while the plot stays at least `MIN_PLOT`
    /// tall. The totals line goes first, then the dates.
    pub fn for_height(height: u16) -> Rows {
        let used = |s: bool, d: bool| u16::from(s) + u16::from(d);
        let mut stats = true;
        let mut dates = true;
        if height < used(stats, dates) + MIN_PLOT {
            stats = false;
        }
        if height < used(stats, dates) + MIN_PLOT {
            dates = false;
        }
        Rows {
            stats,
            dates,
            plot: height.saturating_sub(used(stats, dates)),
        }
    }
}

/// Where everything goes inside a chart panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    pub stats_y: Option<u16>,
    pub plot: Rect,
    pub dates_y: Option<u16>,
    /// Columns of the y axis: its labels and the axis line.
    pub gutter: u16,
    pub inner: Rect,
    /// Days added up into one bar.
    pub per: usize,
    /// For each bar, its first column in the plot and the columns it takes,
    /// with its gap.
    pub cells: Vec<(u16, u16)>,
}

impl Geometry {
    /// The layout for `days` days in a panel, with y labels `label_width`
    /// columns wide.
    pub fn new(panel: Rect, days: usize, label_width: u16) -> Geometry {
        let inner = Rect::new(
            panel.x + 1,
            panel.y + 1,
            panel.width.saturating_sub(2),
            panel.height.saturating_sub(2),
        );
        let wide = inner.width >= MIN_AXIS_WIDTH;
        let gutter = if wide { label_width + 1 } else { 0 };
        let plot_width = usize::from(inner.width.saturating_sub(gutter));
        let (per, bars) = if days == 0 || plot_width == 0 {
            (1, 0)
        } else if days > plot_width {
            let per = days.div_ceil(plot_width);
            (per, days.div_ceil(per))
        } else {
            (1, days)
        };
        // A few bars get a fixed width. Many share the plot, a column or two
        // each, so the chart always fills its width.
        let cells: Vec<(u16, u16)> = if bars == 0 {
            Vec::new()
        } else if plot_width / bars >= MAX_CELL {
            (0..bars)
                .map(|i| ((i * MAX_CELL) as u16, MAX_CELL as u16))
                .collect()
        } else {
            (0..bars)
                .map(|i| {
                    let (from, to) = (i * plot_width / bars, (i + 1) * plot_width / bars);
                    (from as u16, (to - from) as u16)
                })
                .collect()
        };
        let rows = Rows::for_height(inner.height);
        let mut y = inner.y;
        let mut take = |rows: u16| {
            y += rows;
            y - rows
        };
        let stats_y = rows.stats.then(|| take(1));
        let plot_y = take(rows.plot);
        let dates_y = rows.dates.then(|| take(1));
        Geometry {
            stats_y,
            plot: Rect::new(inner.x + gutter, plot_y, plot_width as u16, rows.plot),
            dates_y,
            gutter,
            inner,
            per,
            cells,
        }
    }

    pub fn bars(&self) -> usize {
        self.cells.len()
    }

    /// The first column of bar `i` on screen.
    pub fn x_of(&self, i: usize) -> u16 {
        self.plot.x + self.cells[i].0
    }

    /// The columns the bar itself fills: its cell less a gap of one when the
    /// cell is wider than a column.
    pub fn bar_width(&self, i: usize) -> u16 {
        match self.cells[i].1 {
            0 | 1 => self.cells[i].1,
            w => w - 1,
        }
    }

    /// The columns from the plot start to the end of the last bar.
    pub fn span(&self) -> usize {
        self.cells
            .len()
            .checked_sub(1)
            .map_or(0, |i| usize::from(self.cells[i].0 + self.bar_width(i)))
    }
}

/// A series laid out in a panel: geometry, bars and the scale.
#[derive(Debug, Clone)]
pub struct Plot {
    pub geo: Geometry,
    pub bars: Vec<Bar>,
    pub axis: Axis,
    pub metric: Metric,
}

impl Plot {
    pub fn new(panel: Rect, series: &[DayPoint], metric: Metric) -> Plot {
        // The label width depends on the scale, which depends on how many
        // days a bar holds, which depends on the label width. It settles in a
        // pass or two; a third pass is the last.
        let mut label_width = 2;
        let mut plot = Plot::sized(panel, series, metric, label_width);
        for _ in 0..2 {
            let width = plot.label_width();
            if width == label_width {
                break;
            }
            label_width = width;
            plot = Plot::sized(panel, series, metric, label_width);
        }
        plot
    }

    fn sized(panel: Rect, series: &[DayPoint], metric: Metric, label_width: u16) -> Plot {
        let geo = Geometry::new(panel, series.len(), label_width);
        let bars = bars(series, geo.per);
        let peak = bars.iter().map(|b| b.value(metric)).fold(0.0, f64::max);
        Plot {
            geo,
            bars,
            axis: Axis::of(peak),
            metric,
        }
    }

    /// The columns the widest y label needs.
    fn label_width(&self) -> u16 {
        self.axis
            .ticks()
            .iter()
            .map(|t| self.metric.axis_label(*t).chars().count() as u16)
            .max()
            .unwrap_or(2)
    }
}

/// What the chart shows besides the bars.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub fill: Fill,
    /// The day number of today, to mark its bar.
    pub today: i64,
}

fn dim(pal: &Palette) -> Style {
    Style::default().fg(pal.color(pal.graph_text))
}

fn text(pal: &Palette) -> Style {
    Style::default().fg(pal.color(pal.fg))
}

/// `09-03`, the date without its year.
fn short_date(day: i64) -> String {
    date_key(day)[5..].to_string()
}

/// The totals line, the longest form that fits.
pub fn stats(plot: &Plot, series: &[DayPoint], today: i64, width: usize) -> String {
    let m = plot.metric;
    let value = |d: &DayPoint| match m {
        Metric::Cost => d.cost_usd,
        Metric::Tokens => d.tokens as f64,
    };
    let total: f64 = series.iter().map(value).sum();
    let peak = series.iter().map(value).fold(0.0, f64::max);
    let peak_day = series.iter().find(|d| value(d) == peak && peak > 0.0);
    // The average is per day that has happened, so a week still in progress
    // is not pulled down by the days to come.
    let elapsed = series.iter().filter(|d| d.day <= today).count();
    let avg = if elapsed == 0 {
        0.0
    } else {
        total / elapsed as f64
    };
    let on = peak_day.map_or(String::new(), |d| format!(" on {}", short_date(d.day)));
    let forms = [
        format!(
            "total {}  peak {}{on}  avg {}/day",
            m.show(total),
            m.show(peak),
            m.show(avg)
        ),
        format!(
            "total {}  peak {}  avg {}/day",
            m.show(total),
            m.show(peak),
            m.show(avg)
        ),
        format!(
            "total {}  peak {}  avg {}",
            m.show(total),
            m.show(peak),
            m.show(avg)
        ),
        format!("total {}  peak {}", m.show(total), m.show(peak)),
        format!("total {}", m.show(total)),
    ];
    for form in &forms {
        if form.chars().count() <= width {
            return form.clone();
        }
    }
    fit(&forms[4], width)
}

/// The three date labels under the plot: first, middle and last, as many as
/// fit without touching. Years are shown when there is room for them.
fn date_labels(plot: &Plot, width: usize) -> Vec<(usize, String)> {
    let bars = &plot.bars;
    let (Some(first), Some(last)) = (bars.first(), bars.last()) else {
        return Vec::new();
    };
    let span = plot.geo.span();
    let long = |d: i64| date_key(d);
    let short = |d: i64| short_date(d);
    let pick = |fmt: &dyn Fn(i64) -> String| {
        let mut out = vec![(0, fmt(first.first))];
        if bars.len() > 1 {
            out.push((
                span.saturating_sub(fmt(last.last).chars().count()),
                fmt(last.last),
            ));
        }
        if bars.len() > 2 {
            let mid = &bars[bars.len() / 2];
            let label = fmt(mid.first);
            let at = usize::from(plot.geo.cells[bars.len() / 2].0);
            let at = at.min(span.saturating_sub(label.chars().count()));
            out.push((at, label));
        }
        out.sort_by_key(|(at, _)| *at);
        out
    };
    let fits = |labels: &[(usize, String)]| {
        labels
            .windows(2)
            .all(|w| w[0].0 + w[0].1.chars().count() < w[1].0)
            && labels
                .last()
                .is_some_and(|(at, l)| at + l.chars().count() <= width)
    };
    let drop_middle = |mut labels: Vec<(usize, String)>| {
        if labels.len() == 3 {
            labels.remove(1);
        }
        labels
    };
    for labels in [pick(&long), pick(&short), drop_middle(pick(&short))] {
        if fits(&labels) {
            return labels;
        }
    }
    vec![(0, short(first.first))]
}

/// Draws the chart inside `plot.geo.inner`. The caller draws the box.
pub fn render(buf: &mut Buffer, plot: &Plot, series: &[DayPoint], view: &View, pal: &Palette) {
    let geo = &plot.geo;
    if geo.inner.width == 0 || geo.inner.height == 0 {
        return;
    }
    let width = usize::from(geo.inner.width);
    if let Some(y) = geo.stats_y {
        let line = stats(plot, series, view.today, width);
        buf.set_stringn(geo.inner.x, y, line, width, text(pal));
    }
    draw_axis(buf, plot, pal);
    draw_bars(buf, plot, view, pal);
    if let Some(y) = geo.dates_y {
        for (at, label) in date_labels(plot, usize::from(geo.plot.width)) {
            buf.set_stringn(
                geo.plot.x + at as u16,
                y,
                &label,
                usize::from(geo.plot.width).saturating_sub(at),
                dim(pal),
            );
        }
    }
}

/// The y axis: labels at the bottom, middle and top, an axis line, and faint
/// guide lines across the plot at the top and the middle.
fn draw_axis(buf: &mut Buffer, plot: &Plot, pal: &Palette) {
    let geo = &plot.geo;
    let p = geo.plot;
    if p.height == 0 || p.width == 0 {
        return;
    }
    let guide = Style::default().fg(pal.color(pal.div_line));
    let mid = p.height / 2;
    let rows: Vec<(u16, f64)> = match p.height {
        1 => vec![(0, plot.axis.max)],
        2 => vec![(0, plot.axis.max), (1, 0.0)],
        h => vec![(0, plot.axis.max), (mid, plot.axis.max / 2.0), (h - 1, 0.0)],
    };
    if p.height >= MIN_PLOT {
        for row in [0, mid] {
            for x in p.x..p.x + p.width {
                buf[(x, p.y + row)].set_char('\u{2504}').set_style(guide);
            }
        }
    }
    if geo.gutter == 0 {
        return;
    }
    let axis_x = p.x - 1;
    for row in 0..p.height {
        buf[(axis_x, p.y + row)]
            .set_char('\u{2502}')
            .set_style(dim(pal));
    }
    for (row, value) in rows {
        buf[(axis_x, p.y + row)]
            .set_char('\u{2524}')
            .set_style(dim(pal));
        let label = fit(&plot.metric.axis_label(value), usize::from(geo.gutter - 1));
        let width = label.chars().count() as u16;
        buf.set_string(axis_x - width, p.y + row, label, dim(pal));
    }
}

/// The bars, colored along the fill by their height on the axis.
fn draw_bars(buf: &mut Buffer, plot: &Plot, view: &View, pal: &Palette) {
    let geo = &plot.geo;
    let p = geo.plot;
    for (i, bar) in plot.bars.iter().enumerate() {
        let value = bar.value(plot.metric);
        let levels = plot.axis.levels(value, p.height);
        let pct = value / plot.axis.max * 100.0;
        let color = pal.graded(view.fill, pct);
        let x0 = geo.x_of(i);
        for col in 0..geo.bar_width(i) {
            let x = x0 + col;
            if x >= p.x + p.width {
                break;
            }
            for r in 0..p.height {
                let eighths = levels.saturating_sub(usize::from(r) * 8).min(8);
                if eighths == 0 {
                    break;
                }
                let y = p.y + p.height - 1 - r;
                buf[(x, y)]
                    .set_char(EIGHTHS[eighths - 1])
                    .set_style(Style::default().fg(color));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::tui::theme::{ColorDepth, Theme};

    fn pal() -> Palette {
        Theme::Btop.palette(ColorDepth::True)
    }

    /// 2026-08-05, a Wednesday.
    const FIRST: i64 = 20_670;

    fn series(costs: &[f64]) -> Vec<DayPoint> {
        costs
            .iter()
            .enumerate()
            .map(|(i, c)| DayPoint {
                day: FIRST + i as i64,
                cost_usd: *c,
                tokens: (*c * 1000.0) as u64,
                messages: usize::from(*c > 0.0),
                models: if *c > 0.0 {
                    vec![("claude-opus-5".to_string(), *c)]
                } else {
                    Vec::new()
                },
            })
            .collect()
    }

    fn draw(panel: Rect, series: &[DayPoint]) -> (Plot, Buffer) {
        let plot = Plot::new(panel, series, Metric::Cost);
        let mut buf = Buffer::empty(panel);
        let pal = pal();
        let view = View {
            fill: pal.cpu,
            today: series.last().map_or(0, |d| d.day),
        };
        render(&mut buf, &plot, series, &view, &pal);
        (plot, buf)
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (buf.area.x..buf.area.x + buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect()
    }

    fn screen(buf: &Buffer) -> String {
        (buf.area.y..buf.area.y + buf.area.height)
            .map(|y| row(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rows_keep_the_plot_at_least_three_tall_and_drop_the_totals_then_the_dates() {
        let r = Rows::for_height;
        assert_eq!(
            r(8),
            Rows {
                stats: true,
                dates: true,
                plot: 6
            }
        );
        assert_eq!(
            r(5),
            Rows {
                stats: true,
                dates: true,
                plot: 3
            }
        );
        assert_eq!(
            r(4),
            Rows {
                stats: false,
                dates: true,
                plot: 3
            }
        );
        assert_eq!(
            r(3),
            Rows {
                stats: false,
                dates: false,
                plot: 3
            }
        );
        assert_eq!(r(1).plot, 1);
        assert_eq!(r(0).plot, 0);
    }

    #[test]
    fn bars_share_the_plot_so_the_chart_fills_its_width() {
        // 38 inner columns less a 5 column axis leaves 33.
        let g = |days| Geometry::new(Rect::new(0, 0, 40, 10), days, 4);
        let widths = |g: &Geometry| g.cells.iter().map(|c| c.1).collect::<Vec<_>>();
        let thirty = g(30);
        assert_eq!((thirty.per, thirty.bars()), (1, 30));
        assert_eq!(widths(&thirty).iter().sum::<u16>(), 33);
        assert!(widths(&thirty).iter().all(|w| (1..=2).contains(w)));
        let seven = g(7);
        assert_eq!(widths(&seven).iter().sum::<u16>(), 33);
        assert!(widths(&seven).iter().all(|w| (4..=5).contains(w)));
        assert_eq!(seven.bar_width(0), seven.cells[0].1 - 1, "a gap of one");
        assert_eq!(thirty.bar_width(0), 1, "no gap at one column");
        // A few days get a fixed width and do not stretch into slabs.
        let one = g(1);
        assert_eq!(one.cells, vec![(0, MAX_CELL as u16)]);
        assert_eq!(one.bar_width(0), MAX_CELL as u16 - 1);
        // Bars never overlap and never leave the plot.
        for days in 1..200 {
            let g = g(days);
            let mut end = 0;
            for (from, w) in &g.cells {
                assert!(*from >= end && from + w <= 33, "{days}");
                end = from + w;
            }
        }
        // More days than columns are added up, so nothing is cut off.
        let ninety = g(90);
        assert_eq!((ninety.per, ninety.bars()), (3, 30));
        let year = g(400);
        assert_eq!((year.per, year.bars()), (13, 31));
        assert_eq!(g(0).bars(), 0);
    }

    #[test]
    fn buckets_add_up_days_cost_tokens_messages_and_models() {
        let mut s = series(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        s[1].models.push(("claude-haiku-5".to_string(), 0.5));
        let b = bars(&s, 2);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].first, b[0].last, b[0].cost), (FIRST, FIRST + 1, 3.0));
        assert_eq!(b[0].messages, 2);
        assert_eq!(b[0].tokens, 3_000);
        assert_eq!(b[0].models.len(), 2);
        assert_eq!(
            (b[2].first, b[2].last, b[2].days()),
            (FIRST + 4, FIRST + 4, 1)
        );
        assert_eq!(b[0].top_model().map(|(n, _)| n), Some("claude-opus-5"));
        let total: f64 = b.iter().map(|x| x.cost).sum();
        assert!((total - 15.0).abs() < 1e-9);
    }

    #[test]
    fn the_y_axis_labels_the_bottom_middle_and_top_with_round_values() {
        let s = series(&[1.0, 4.1, 0.5]);
        let (plot, buf) = draw(Rect::new(0, 0, 40, 12), &s);
        let p = plot.geo.plot;
        assert_eq!(plot.axis.max, 5.0);
        let label = |y: u16| {
            row(&buf, y)
                .chars()
                .skip(1)
                .take(usize::from(p.x - 2))
                .collect::<String>()
        };
        assert_eq!(label(p.y).trim(), "$5");
        assert_eq!(label(p.y + p.height / 2).trim(), "$2.5");
        assert_eq!(label(p.y + p.height - 1).trim(), "$0");
        // The axis line is a tick at each label and a bar between them.
        let axis_x = p.x - 1;
        assert_eq!(buf[(axis_x, p.y)].symbol(), "\u{2524}");
        assert_eq!(buf[(axis_x, p.y + 1)].symbol(), "\u{2502}");
    }

    #[test]
    fn the_peak_bar_reaches_the_scale_proportionally() {
        // The top of the axis is $5, so $2.5 fills half of a four row plot.
        let s = series(&[2.5, 5.0]);
        let (plot, buf) = draw(Rect::new(0, 0, 40, 10), &s);
        let p = plot.geo.plot;
        let h = p.height;
        let col = |i: usize| plot.geo.x_of(i);
        let filled = |x: u16| {
            (0..h)
                .filter(|r| buf[(x, p.y + h - 1 - r)].symbol() == "\u{2588}")
                .count()
        };
        assert_eq!(filled(col(1)), usize::from(h));
        assert_eq!(filled(col(0)), usize::from(h) / 2);
    }

    #[test]
    fn a_small_day_still_shows_a_sliver() {
        let s = series(&[0.001, 100.0]);
        let (plot, buf) = draw(Rect::new(0, 0, 40, 10), &s);
        let p = plot.geo.plot;
        assert_eq!(buf[(p.x, p.y + p.height - 1)].symbol(), "\u{2581}");
    }

    #[test]
    fn the_x_axis_shows_the_first_middle_and_last_date() {
        let s = series(&vec![1.0; 30]);
        let (plot, buf) = draw(Rect::new(0, 0, 60, 12), &s);
        let dates = row(&buf, plot.geo.dates_y.unwrap());
        assert!(dates.contains("2026-08-05"), "{dates}");
        assert!(dates.contains("2026-08-20"), "{dates}");
        assert!(dates.contains("2026-09-03"), "{dates}");
        // On a narrow panel the dates lose their year, then the middle one.
        let (plot, buf) = draw(Rect::new(0, 0, 30, 12), &s);
        let dates = row(&buf, plot.geo.dates_y.unwrap());
        assert!(
            dates.contains("08-05") && dates.contains("09-03"),
            "{dates}"
        );
        assert!(!dates.contains("2026"), "{dates}");
    }

    #[test]
    fn the_totals_line_has_total_peak_and_average_and_shrinks_to_fit() {
        let s = series(&[1.0, 6.0, 2.0, 3.0]);
        let plot = Plot::new(Rect::new(0, 0, 40, 12), &s, Metric::Cost);
        assert_eq!(
            stats(&plot, &s, FIRST + 3, 80),
            "total $12.00  peak $6.00 on 08-06  avg $3.00/day"
        );
        assert_eq!(
            stats(&plot, &s, FIRST + 3, 39),
            "total $12.00  peak $6.00  avg $3.00/day"
        );
        assert_eq!(
            stats(&plot, &s, FIRST + 3, 35),
            "total $12.00  peak $6.00  avg $3.00"
        );
        assert_eq!(stats(&plot, &s, FIRST + 3, 24), "total $12.00  peak $6.00");
        assert_eq!(stats(&plot, &s, FIRST + 3, 12), "total $12.00");
        assert_eq!(stats(&plot, &s, FIRST + 3, 6), "total~");
    }

    #[test]
    fn the_average_counts_only_the_days_that_have_happened() {
        let s = series(&[2.0, 4.0, 0.0, 0.0]);
        let plot = Plot::new(Rect::new(0, 0, 40, 12), &s, Metric::Cost);
        // Today is the second day, so the average is $6 over two days.
        let line = stats(&plot, &s, FIRST + 1, 80);
        assert!(line.contains("avg $3.00/day"), "{line}");
    }

    #[test]
    fn the_chart_draws_at_every_size_without_panicking() {
        let s = series(&vec![1.0; 45]);
        for w in 0..70 {
            for h in 0..22 {
                let panel = Rect::new(0, 0, w, h);
                let (plot, buf) = draw(panel, &s);
                if w >= 3 && h >= 3 {
                    assert!(plot.geo.plot.height + 2 <= h, "{w}x{h}");
                }
                assert_eq!(buf.area, panel);
                let _ = Plot::new(panel, &[], Metric::Tokens);
                let _ = draw(panel, &[]);
            }
        }
    }

    #[test]
    fn a_narrow_panel_drops_the_y_axis_but_keeps_the_bars() {
        let s = series(&[1.0, 2.0, 3.0]);
        let (plot, buf) = draw(Rect::new(0, 0, 14, 10), &s);
        assert_eq!(plot.geo.gutter, 0);
        assert!(screen(&buf).contains('\u{2588}'));
        assert!(!screen(&buf).contains('\u{2524}'));
    }

    #[test]
    fn a_range_wider_than_the_plot_is_added_up_and_the_axis_follows() {
        let s = series(&vec![1.0; 90]);
        let plot = Plot::new(Rect::new(0, 0, 40, 12), &s, Metric::Cost);
        assert_eq!(plot.geo.per, 3);
        assert_eq!(plot.bars.len(), 30);
        // Each bar holds three $1 days, so the scale tops out at $3.
        assert_eq!(plot.axis.max, 3.0);
    }
}
