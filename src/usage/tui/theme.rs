// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Colors for the terminal view. `t` cycles the themes. `Mono` uses no color
//! at all, for terminals and pipes that cannot show it.
//!
//! The `btop` theme is the default. Its values are btop's built-in default
//! theme (`Default_theme` in `src/btop_theme.cpp` of
//! https://github.com/aristocratos/btop, Apache-2.0, Copyright 2021 Aristocratos),
//! mapped to the roles this view has: a two digit hex in btop is a gray (`#cc`
//! is 204, 204, 204), and a graph or meter colors a value from a start, mid
//! and end color the way btop does. See NOTICE.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Btop,
    Dark,
    Light,
    Mono,
}

/// How many colors the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    #[default]
    True,
    Ansi256,
    Ansi16,
}

impl ColorDepth {
    /// From `COLORTERM` and `TERM`: true color when `COLORTERM` says so, 256
    /// colors when `TERM` names them, else the 16 basic ones.
    pub fn detect(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
        if matches!(colorterm, Some("truecolor" | "24bit")) {
            ColorDepth::True
        } else if term.is_some_and(|t| t.contains("256color")) {
            ColorDepth::Ansi256
        } else {
            ColorDepth::Ansi16
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// A color role value: the terminal's own color, a named 16 color, or an RGB
/// value that is reduced to the terminal's depth when drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Default,
    Named(Color),
    Rgb(Rgb),
}

/// What a graph or meter paints with: one color, or btop's three point
/// gradient (start at 0%, mid at 50%, end at 100%).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Flat(Ink),
    Gradient(Rgb, Rgb, Rgb),
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
}

impl Fill {
    /// The color at `pct` percent (clamped to 0 to 100).
    pub fn at(self, pct: f64) -> Ink {
        match self {
            Fill::Flat(ink) => ink,
            Fill::Gradient(start, mid, end) => {
                let pct = pct.clamp(0.0, 100.0);
                let (a, b, t) = if pct <= 50.0 {
                    (start, mid, pct / 50.0)
                } else {
                    (mid, end, (pct - 50.0) / 50.0)
                };
                Ink::Rgb(Rgb(lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t)))
            }
        }
    }
}

/// Every color the view uses, by role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub depth: ColorDepth,
    pub mono: bool,
    pub bg: Ink,
    pub fg: Ink,
    pub title: Ink,
    /// Panel number hints and the accent.
    pub hi: Ink,
    pub sel_bg: Ink,
    pub sel_fg: Ink,
    pub inactive: Ink,
    pub graph_text: Ink,
    pub meter_bg: Ink,
    pub div_line: Ink,
    pub box_cpu: Ink,
    pub box_mem: Ink,
    pub box_net: Ink,
    pub box_proc: Ink,
    pub cpu: Fill,
    pub down: Fill,
    pub cached: Fill,
    pub avail: Fill,
    pub process: Fill,
    pub live: Ink,
    pub warn: Ink,
    pub error: Ink,
}

const fn gray(v: u8) -> Rgb {
    Rgb(v, v, v)
}

const fn hex(rgb: u32) -> Rgb {
    Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const fn ink(rgb: Rgb) -> Ink {
    Ink::Rgb(rgb)
}

impl Theme {
    pub fn next(self) -> Theme {
        match self {
            Theme::Btop => Theme::Dark,
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Mono,
            Theme::Mono => Theme::Btop,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Btop => "btop",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Mono => "mono",
        }
    }

    pub fn parse(name: &str) -> Option<Theme> {
        [Theme::Btop, Theme::Dark, Theme::Light, Theme::Mono]
            .into_iter()
            .find(|t| t.name() == name)
    }

    pub fn palette(self, depth: ColorDepth) -> Palette {
        match self {
            Theme::Btop => Palette {
                depth,
                mono: false,
                bg: ink(gray(0x00)),
                fg: ink(gray(0xcc)),
                title: ink(gray(0xee)),
                hi: ink(hex(0xb54040)),
                sel_bg: ink(hex(0x6a2f2f)),
                sel_fg: ink(gray(0xee)),
                inactive: ink(gray(0x40)),
                graph_text: ink(gray(0x60)),
                meter_bg: ink(gray(0x40)),
                div_line: ink(gray(0x30)),
                box_cpu: ink(hex(0x556d59)),
                box_mem: ink(hex(0x6c6c4b)),
                box_net: ink(hex(0x5c588d)),
                box_proc: ink(hex(0x805252)),
                cpu: Fill::Gradient(hex(0x77ca9b), hex(0xcbc06c), hex(0xdc4c4c)),
                down: Fill::Gradient(hex(0x291f75), hex(0x4f43a3), hex(0xb0a9de)),
                cached: Fill::Gradient(hex(0x163350), hex(0x74e6fc), hex(0x26c5ff)),
                avail: Fill::Gradient(hex(0x4e3f0e), hex(0xffd77a), hex(0xffb814)),
                process: Fill::Gradient(hex(0x80d0a3), hex(0xdcd179), hex(0xd45454)),
                live: ink(hex(0x0de756)),
                warn: ink(hex(0xffb814)),
                error: ink(hex(0xff4769)),
            },
            Theme::Dark => Palette {
                depth,
                mono: false,
                bg: Ink::Default,
                fg: Ink::Default,
                title: Ink::Default,
                hi: Ink::Named(Color::Cyan),
                sel_bg: Ink::Named(Color::Blue),
                sel_fg: Ink::Named(Color::White),
                inactive: Ink::Named(Color::DarkGray),
                graph_text: Ink::Named(Color::DarkGray),
                meter_bg: Ink::Named(Color::DarkGray),
                div_line: Ink::Named(Color::DarkGray),
                box_cpu: Ink::Default,
                box_mem: Ink::Default,
                box_net: Ink::Default,
                box_proc: Ink::Default,
                cpu: Fill::Flat(Ink::Named(Color::Cyan)),
                down: Fill::Flat(Ink::Named(Color::Magenta)),
                cached: Fill::Flat(Ink::Named(Color::Cyan)),
                avail: Fill::Flat(Ink::Named(Color::Cyan)),
                process: Fill::Flat(Ink::Default),
                live: Ink::Named(Color::Green),
                warn: Ink::Named(Color::Yellow),
                error: Ink::Named(Color::Red),
            },
            // Bright colors vanish on a white ground, so use the dark ones.
            Theme::Light => Palette {
                hi: Ink::Named(Color::Blue),
                inactive: Ink::Named(Color::Gray),
                graph_text: Ink::Named(Color::Gray),
                meter_bg: Ink::Named(Color::Gray),
                div_line: Ink::Named(Color::Gray),
                cpu: Fill::Flat(Ink::Named(Color::Blue)),
                cached: Fill::Flat(Ink::Named(Color::Blue)),
                avail: Fill::Flat(Ink::Named(Color::Blue)),
                warn: ink(Rgb(160, 100, 0)),
                ..Theme::Dark.palette(depth)
            },
            Theme::Mono => Palette {
                mono: true,
                hi: Ink::Default,
                sel_bg: Ink::Default,
                sel_fg: Ink::Default,
                inactive: Ink::Default,
                graph_text: Ink::Default,
                meter_bg: Ink::Default,
                div_line: Ink::Default,
                cpu: Fill::Flat(Ink::Default),
                down: Fill::Flat(Ink::Default),
                cached: Fill::Flat(Ink::Default),
                avail: Fill::Flat(Ink::Default),
                live: Ink::Default,
                warn: Ink::Default,
                error: Ink::Default,
                ..Theme::Dark.palette(depth)
            },
        }
    }
}

/// The 16 basic colors with the values xterm shows, for the nearest match.
const BASIC: [(Color, Rgb); 16] = [
    (Color::Black, Rgb(0, 0, 0)),
    (Color::Red, Rgb(205, 0, 0)),
    (Color::Green, Rgb(0, 205, 0)),
    (Color::Yellow, Rgb(205, 205, 0)),
    (Color::Blue, Rgb(0, 0, 238)),
    (Color::Magenta, Rgb(205, 0, 205)),
    (Color::Cyan, Rgb(0, 205, 205)),
    (Color::Gray, Rgb(229, 229, 229)),
    (Color::DarkGray, Rgb(127, 127, 127)),
    (Color::LightRed, Rgb(255, 0, 0)),
    (Color::LightGreen, Rgb(0, 255, 0)),
    (Color::LightYellow, Rgb(255, 255, 0)),
    (Color::LightBlue, Rgb(92, 92, 255)),
    (Color::LightMagenta, Rgb(255, 0, 255)),
    (Color::LightCyan, Rgb(0, 255, 255)),
    (Color::White, Rgb(255, 255, 255)),
];

fn distance(a: Rgb, b: Rgb) -> i32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// The nearest of the 6 levels of the 256 color cube.
fn cube_level(v: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    (0..6u8)
        .min_by_key(|i| (i32::from(LEVELS[usize::from(*i)]) - i32::from(v)).abs())
        .unwrap_or(0)
}

impl Rgb {
    /// This color at the given depth. 256 colors use the cube or, for a gray,
    /// the 24 step gray ramp, whichever is nearer.
    pub fn at(self, depth: ColorDepth) -> Color {
        match depth {
            ColorDepth::True => Color::Rgb(self.0, self.1, self.2),
            ColorDepth::Ansi256 => {
                let levels = [0, 95, 135, 175, 215, 255];
                let (r, g, b) = (cube_level(self.0), cube_level(self.1), cube_level(self.2));
                let cube = Rgb(
                    levels[usize::from(r)],
                    levels[usize::from(g)],
                    levels[usize::from(b)],
                );
                let avg = ((u16::from(self.0) + u16::from(self.1) + u16::from(self.2)) / 3) as u8;
                let step = (i32::from(avg) - 8 + 5) / 10;
                let step = step.clamp(0, 23) as u8;
                let ramp_v = 8 + 10 * step;
                let ramp = Rgb(ramp_v, ramp_v, ramp_v);
                if distance(self, ramp) < distance(self, cube) {
                    Color::Indexed(232 + step)
                } else {
                    Color::Indexed(16 + 36 * r + 6 * g + b)
                }
            }
            ColorDepth::Ansi16 => BASIC
                .iter()
                .min_by_key(|(_, c)| distance(self, *c))
                .map_or(Color::Reset, |(color, _)| *color),
        }
    }
}

impl Palette {
    pub fn color(&self, ink: Ink) -> Color {
        match ink {
            Ink::Default => Color::Reset,
            Ink::Named(c) => c,
            Ink::Rgb(rgb) => rgb.at(self.depth),
        }
    }

    pub fn graded(&self, fill: Fill, pct: f64) -> Color {
        self.color(fill.at(pct))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_visits_every_theme_and_wraps() {
        let mut t = Theme::Btop;
        let mut seen = vec![t];
        for _ in 0..4 {
            t = t.next();
            seen.push(t);
        }
        assert_eq!(
            seen,
            [
                Theme::Btop,
                Theme::Dark,
                Theme::Light,
                Theme::Mono,
                Theme::Btop
            ]
        );
    }

    #[test]
    fn btop_is_the_default_and_names_round_trip() {
        assert_eq!(Theme::default(), Theme::Btop);
        for t in [Theme::Btop, Theme::Dark, Theme::Light, Theme::Mono] {
            assert_eq!(Theme::parse(t.name()), Some(t));
        }
        assert_eq!(Theme::parse("solarized"), None);
    }

    #[test]
    fn btop_uses_the_values_of_btops_default_theme() {
        let p = Theme::Btop.palette(ColorDepth::True);
        assert_eq!(p.color(p.bg), Color::Rgb(0, 0, 0));
        assert_eq!(p.color(p.fg), Color::Rgb(0xcc, 0xcc, 0xcc));
        assert_eq!(p.color(p.title), Color::Rgb(0xee, 0xee, 0xee));
        assert_eq!(p.color(p.hi), Color::Rgb(0xb5, 0x40, 0x40));
        assert_eq!(p.color(p.sel_bg), Color::Rgb(0x6a, 0x2f, 0x2f));
        assert_eq!(p.color(p.box_cpu), Color::Rgb(0x55, 0x6d, 0x59));
        assert_eq!(p.color(p.box_mem), Color::Rgb(0x6c, 0x6c, 0x4b));
        assert_eq!(p.color(p.box_net), Color::Rgb(0x5c, 0x58, 0x8d));
        assert_eq!(p.color(p.box_proc), Color::Rgb(0x80, 0x52, 0x52));
    }

    #[test]
    fn a_gradient_runs_start_to_mid_to_end() {
        let g = Fill::Gradient(Rgb(0, 0, 0), Rgb(100, 100, 100), Rgb(200, 0, 0));
        assert_eq!(g.at(0.0), Ink::Rgb(Rgb(0, 0, 0)));
        assert_eq!(g.at(25.0), Ink::Rgb(Rgb(50, 50, 50)));
        assert_eq!(g.at(50.0), Ink::Rgb(Rgb(100, 100, 100)));
        assert_eq!(g.at(100.0), Ink::Rgb(Rgb(200, 0, 0)));
        assert_eq!(g.at(250.0), g.at(100.0));
    }

    #[test]
    fn depth_comes_from_colorterm_then_term() {
        use ColorDepth::*;
        assert_eq!(ColorDepth::detect(Some("truecolor"), Some("xterm")), True);
        assert_eq!(ColorDepth::detect(Some("24bit"), None), True);
        assert_eq!(ColorDepth::detect(None, Some("xterm-256color")), Ansi256);
        assert_eq!(ColorDepth::detect(None, Some("linux")), Ansi16);
        assert_eq!(ColorDepth::detect(None, None), Ansi16);
    }

    #[test]
    fn colors_reduce_to_the_256_and_16_color_sets() {
        // A gray lands on the gray ramp, a hue on the cube, red on basic red.
        assert_eq!(gray(0xcc).at(ColorDepth::Ansi256), Color::Indexed(252));
        assert_eq!(Rgb(255, 0, 0).at(ColorDepth::Ansi256), Color::Indexed(196));
        assert_eq!(Rgb(0, 0, 0).at(ColorDepth::Ansi256), Color::Indexed(16));
        assert_eq!(Rgb(0xdc, 0x4c, 0x4c).at(ColorDepth::Ansi16), Color::Red);
        assert_eq!(gray(0x30).at(ColorDepth::Ansi16), Color::Black);
        assert_eq!(gray(0xee).at(ColorDepth::Ansi16), Color::Gray);
    }

    #[test]
    fn mono_sets_no_color() {
        let p = Theme::Mono.palette(ColorDepth::True);
        assert!(p.mono);
        for i in [p.fg, p.hi, p.box_cpu, p.live, p.warn, p.error, p.bg] {
            assert_eq!(p.color(i), Color::Reset);
        }
        assert_eq!(p.graded(p.cpu, 90.0), Color::Reset);
        assert_eq!(p.graded(p.process, 90.0), Color::Reset);
    }
}
