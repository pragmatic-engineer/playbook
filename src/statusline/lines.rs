// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Lines 2 and 3 of the status line: model and context on one, session
//! economics on the other. Each segment is `None` when it has nothing to say.

use super::fmt::*;
use crate::json::statusline::value_text;

/// The session fields the segments read, as the script's shell variables.
#[derive(Debug, Default, Clone)]
pub struct Session {
    pub cwd: String,
    pub session_id: String,
    pub model: String,
    pub used: String,
    pub ctx_total_tokens: String,
    pub ctx_window_size: String,
    pub cache_create: String,
    pub cache_read: String,
    pub rl_5h: String,
    pub rl_5h_reset: String,
    pub rl_7d: String,
    pub effort: String,
    pub thinking: String,
    pub cost_usd: String,
    pub wall_ms: String,
}

impl Session {
    pub fn parse(input: &str, pwd: &str) -> Self {
        let mut s = Session::default();
        for (key, value) in crate::json::statusline::session_values(input, pwd) {
            let text = value_text(&value);
            match key {
                "cwd" => s.cwd = text,
                "session_id" => s.session_id = text,
                "model" => s.model = text,
                "used" => s.used = text,
                "ctx_total_tokens" => s.ctx_total_tokens = text,
                "ctx_window_size" => s.ctx_window_size = text,
                "cache_create" => s.cache_create = text,
                "cache_read" => s.cache_read = text,
                "rl_5h" => s.rl_5h = text,
                "rl_5h_reset" => s.rl_5h_reset = text,
                "rl_7d" => s.rl_7d = text,
                "json_effort" => s.effort = text,
                "json_thinking" => s.thinking = text,
                "cost_usd" => s.cost_usd = text,
                "wall_ms" => s.wall_ms = text,
                _ => {}
            }
        }
        s
    }
}

/// Everything the segments need beyond the session itself.
pub struct Opts {
    pub show_model: bool,
    pub show_context: bool,
    pub show_session_age: bool,
    pub show_cache_ratio: bool,
    pub show_rate_limits: bool,
    pub bar_width: i64,
    pub compact_trigger: i64,
    pub home: String,
    pub now: i64,
}

fn render_model(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_model || s.model.is_empty() {
        return None;
    }
    let mut extras = s.effort.clone();
    if s.thinking == "true" {
        if !extras.is_empty() {
            extras.push_str(", ");
        }
        extras.push_str("thinking");
    }
    let mut label = format!("{ORANGE}{}{RESET}", s.model);
    if !extras.is_empty() {
        label.push_str(&format!(" {DIM}({extras}){RESET}"));
    }
    Some(label)
}

fn render_context(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_context || s.used.is_empty() {
        return None;
    }
    let pct = round0(&s.used);
    let color = ctx_color(&s.used);
    let bar = ctx_bar(&s.used, o.bar_width);
    let mut out = format!("{DIM}Ctx{RESET} {color}[{bar}] {pct}%{RESET}");
    if !s.ctx_total_tokens.is_empty() {
        let tok = fmt_tokens(int_part(&s.ctx_total_tokens));
        if s.ctx_window_size.is_empty() {
            out.push_str(&format!(" {DIM}({tok}){RESET}"));
        } else {
            let win = fmt_tokens(int_part(&s.ctx_window_size));
            out.push_str(&format!(" {DIM}({tok}/{win}){RESET}"));
        }
    }
    let gap =
        compact_gap(&s.used, o.compact_trigger).filter(|_| pct.parse::<i64>().unwrap_or(0) > 65);
    match gap {
        Some(0) => out.push_str(&format!(" {DIM}→{RESET} {RED}COMPACTING NEXT TURN{RESET}")),
        Some(g) => out.push_str(&format!(
            " {DIM}→{RESET} {color}{g}%{RESET} {DIM}to compact{RESET}"
        )),
        None => {}
    }
    if !s.ctx_total_tokens.is_empty() && int_part(&s.ctx_total_tokens) >= CONTEXT_ROT_THRESHOLD {
        out.push_str(&format!(" {DIM}·{RESET} {RED}⚠ context rot risk{RESET}"));
    }
    Some(out)
}

/// Raw token count past which model quality is known to degrade.
const CONTEXT_ROT_THRESHOLD: i64 = 200_000;

fn render_session_age(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_session_age || s.session_id.is_empty() {
        return None;
    }
    let file = std::path::Path::new(&o.home)
        .join(".config/playbook/runtime")
        .join(&s.session_id)
        .join("start-ts");
    let raw = std::fs::read_to_string(file).ok()?;
    let start = raw.trim_end_matches('\n');
    let start = int_part(start);
    if start <= 0 {
        return None;
    }
    let age = o.now - start;
    (age >= 600).then(|| format!("{DIM}Up{RESET} {GREEN}{}{RESET}", fmt_age(age)))
}

fn render_cache_ratio(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_cache_ratio || (s.cache_create.is_empty() && s.cache_read.is_empty()) {
        return None;
    }
    let pct = cache_hit_pct(int_part(&s.cache_create), int_part(&s.cache_read))?;
    Some(format!(
        "{DIM}Cache{RESET} {}{pct}%{RESET}",
        cache_color(pct)
    ))
}

fn render_cost(s: &Session) -> Option<String> {
    if s.cost_usd.is_empty() {
        return None;
    }
    let cost: f64 = s.cost_usd.trim().parse().unwrap_or(0.0);
    if cost <= 0.0 || cost.is_nan() {
        return None;
    }
    let mut out = format!("{ORANGE}${cost:.4}{RESET}");
    if !s.wall_ms.is_empty() {
        if let Some(rate) = cost_per_min(cost, int_part(&s.wall_ms)) {
            out.push_str(&format!(" {DIM}(${rate}/min){RESET}"));
        }
    }
    Some(out)
}

fn render_rate_5h(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_rate_limits || s.rl_5h.is_empty() {
        return None;
    }
    let mut out = format!(
        "{DIM}5h{RESET} {}{}%{RESET}",
        rl_color(&s.rl_5h),
        round0(&s.rl_5h)
    );
    if !s.rl_5h_reset.is_empty() {
        // A non-integer reset is an arithmetic error in the script, which
        // drops the whole segment rather than only the countdown.
        let reset = strict_int(&s.rl_5h_reset)?;
        let left = reset - o.now;
        if left > 0 {
            out.push_str(&format!(" {DIM}({} left){RESET}", fmt_age(left)));
        }
    }
    Some(out)
}

fn render_rate_7d(s: &Session, o: &Opts) -> Option<String> {
    if !o.show_rate_limits || s.rl_7d.is_empty() {
        return None;
    }
    let v: f64 = s.rl_7d.trim().parse().unwrap_or(0.0);
    (v > 50.0).then(|| {
        format!(
            "{DIM}Rate 7d:{RESET} {}{}%{RESET}",
            rl_color(&s.rl_7d),
            round0(&s.rl_7d)
        )
    })
}

fn join(segments: Vec<Option<String>>) -> Option<String> {
    let parts: Vec<String> = segments.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(&sep()))
}

/// Line 2: model and context.
pub fn line_2(s: &Session, o: &Opts) -> Option<String> {
    join(vec![render_model(s, o), render_context(s, o)])
}

/// Line 3: cost, cache, 5h quota, session age, 7d rate.
pub fn line_3(s: &Session, o: &Opts) -> Option<String> {
    join(vec![
        render_cost(s),
        render_cache_ratio(s, o),
        render_rate_5h(s, o),
        render_session_age(s, o),
        render_rate_7d(s, o),
    ])
}
