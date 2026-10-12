// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pure text renderers. Each takes `Caps` and returns a `String`.

use super::Caps;
use std::fmt::Write as _;

/// Where a wizard screen sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub index: usize,
    pub total: usize,
}

/// One decision with its two outcomes spelled out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Stable name, also used by flags.
    pub id: String,
    /// Screen title.
    pub title: String,
    /// What this part does, in plain words.
    pub explain: String,
    /// Lines shown under "If you accept".
    pub accept: Vec<String>,
    /// Lines shown under "If you decline".
    pub decline: Vec<String>,
    /// Whether yes is the recommended answer.
    pub recommended: bool,
    /// The question asked at the bottom.
    pub question: String,
}

/// How one action landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Done,
    Unchanged,
    Skipped,
    Failed,
}

/// One row of the final summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub name: String,
    pub on: bool,
    pub detail: String,
}

const RESET: &str = "\x1b[0m";

#[derive(Clone, Copy)]
enum Tone {
    Bold,
    Dim,
    Green,
    Yellow,
    Red,
    Cyan,
}

fn paint(caps: &Caps, tone: Tone, text: &str) -> String {
    if !caps.color {
        return text.to_string();
    }
    let code = match tone {
        Tone::Bold => "1",
        Tone::Dim => "2",
        Tone::Green => "32",
        Tone::Yellow => "33",
        Tone::Red => "31",
        Tone::Cyan => "36",
    };
    format!("\x1b[{code}m{text}{RESET}")
}

/// The symbol for a status, falling back to ASCII words.
pub fn symbol(caps: &Caps, status: Status) -> &'static str {
    match (caps.unicode, status) {
        (true, Status::Done) => "\u{2714}",
        (true, Status::Unchanged) => "=",
        (true, Status::Skipped) => "\u{25CB}",
        (true, Status::Failed) => "\u{2718}",
        (false, Status::Done) => "[ok]",
        (false, Status::Unchanged) => "[=]",
        (false, Status::Skipped) => "[skip]",
        (false, Status::Failed) => "[fail]",
    }
}

fn rule(caps: &Caps) -> String {
    let ch = if caps.unicode { "\u{2500}" } else { "-" };
    ch.repeat(caps.width.min(60))
}

/// Wrap `text` to `width` columns, indenting every line by `indent` spaces.
pub fn wrap(text: &str, width: usize, indent: usize) -> String {
    let room = width.saturating_sub(indent).max(10);
    let pad = " ".repeat(indent);
    let mut lines: Vec<String> = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let need = if line.is_empty() { 0 } else { 1 } + word.chars().count();
            if !line.is_empty() && line.chars().count() + need > room {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
        .into_iter()
        .map(|l| if l.is_empty() { l } else { format!("{pad}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A bold heading with a rule under it.
pub fn heading(caps: &Caps, text: &str) -> String {
    format!("{}\n{}\n", paint(caps, Tone::Bold, text), rule(caps))
}

fn bullet_lines(caps: &Caps, lines: &[String], sign: &str, tone: Tone) -> String {
    let mut out = String::new();
    for l in lines {
        let wrapped = wrap(l, caps.width, 4);
        let mut it = wrapped.lines();
        if let Some(first) = it.next() {
            let _ = writeln!(out, "  {} {}", paint(caps, tone, sign), first.trim_start());
        }
        for rest in it {
            let _ = writeln!(out, "{rest}");
        }
    }
    out
}

/// One decision screen: counter, title, explanation, both outcomes.
pub fn step_screen(caps: &Caps, step: Step, choice: &Choice) -> String {
    let mut out = String::new();
    let counter = format!("Step {} of {}", step.index, step.total);
    let rec = if choice.recommended {
        "recommended: yes"
    } else {
        "recommended: no"
    };
    let _ = writeln!(
        out,
        "\n{}  {}  {}",
        paint(caps, Tone::Cyan, &counter),
        paint(caps, Tone::Bold, &choice.title),
        paint(caps, Tone::Dim, &format!("({rec})"))
    );
    let _ = writeln!(out, "{}", rule(caps));
    let _ = writeln!(out, "{}\n", wrap(&choice.explain, caps.width, 0));
    let _ = writeln!(out, "{}", paint(caps, Tone::Green, "If you accept:"));
    out.push_str(&bullet_lines(caps, &choice.accept, "+", Tone::Green));
    let _ = writeln!(out, "\n{}", paint(caps, Tone::Yellow, "If you decline:"));
    out.push_str(&bullet_lines(caps, &choice.decline, "-", Tone::Yellow));
    out.push('\n');
    out
}

/// The prompt line under a screen. `back` shows the back key from step 2 on.
pub fn prompt_line(caps: &Caps, question: &str, default_yes: bool, back: bool) -> String {
    let yn = if default_yes { "Y/n" } else { "y/N" };
    let keys = if back {
        format!("{yn}, b back, s skip, q quit")
    } else {
        format!("{yn}, s skip, q quit")
    };
    format!(
        "{} {} ",
        paint(caps, Tone::Bold, question),
        paint(caps, Tone::Dim, &format!("[{keys}]"))
    )
}

/// The choices before anything changes.
pub fn summary(caps: &Caps, rows: &[SummaryRow]) -> String {
    let mut out = heading(caps, "Summary of your choices");
    let w = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0);
    for r in rows {
        let (word, tone) = if r.on {
            ("on ", Tone::Green)
        } else {
            ("off", Tone::Yellow)
        };
        let _ = writeln!(
            out,
            "  {:<w$}  {}  {}",
            r.name,
            paint(caps, tone, word),
            r.detail,
            w = w
        );
    }
    out.push('\n');
    out
}

/// One progress line: symbol, action, detail.
pub fn progress_line(caps: &Caps, status: Status, name: &str, detail: &str) -> String {
    let tone = match status {
        Status::Done => Tone::Green,
        Status::Unchanged => Tone::Dim,
        Status::Skipped => Tone::Yellow,
        Status::Failed => Tone::Red,
    };
    format!(
        "{} {}  {}",
        paint(caps, tone, symbol(caps, status)),
        name,
        detail
    )
}

/// Counts and the suggested next commands.
pub fn next_steps(caps: &Caps, counts: [usize; 4], steps: &[&str]) -> String {
    let [done, unchanged, skipped, failed] = counts;
    let mut out =
        format!("\n{done} done, {unchanged} unchanged, {skipped} skipped, {failed} failed\n");
    if !steps.is_empty() {
        out.push('\n');
        out.push_str(&heading(caps, "Next steps"));
        for s in steps {
            let _ = writeln!(out, "  {}", wrap(s, caps.width, 2).trim_start());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice() -> Choice {
        Choice {
            id: "path".into(),
            title: "Shell PATH".into(),
            explain: "Puts the playbook binary on PATH so hooks can find it.".into(),
            accept: vec![
                "Adds to ~/.zshenv: # playbook binary".into(),
                "Adds to ~/.zshenv: export PATH=\"$HOME/.local/bin:$PATH\"".into(),
            ],
            decline: vec!["Nothing is written. Hooks may not find the binary.".into()],
            recommended: true,
            question: "Add it?".into(),
        }
    }

    #[test]
    fn step_screen_plain_snapshot() {
        let got = step_screen(&Caps::plain(80), Step { index: 3, total: 7 }, &choice());
        let want = "\nStep 3 of 7  Shell PATH  (recommended: yes)\n\
------------------------------------------------------------\n\
Puts the playbook binary on PATH so hooks can find it.\n\n\
If you accept:\n  \
+ Adds to ~/.zshenv: # playbook binary\n  \
+ Adds to ~/.zshenv: export PATH=\"$HOME/.local/bin:$PATH\"\n\n\
If you decline:\n  \
- Nothing is written. Hooks may not find the binary.\n\n";
        assert_eq!(got, want);
    }

    #[test]
    fn color_wraps_text_in_escape_codes_and_plain_does_not() {
        let mut c = Caps::plain(80);
        assert!(!heading(&c, "Hi").contains('\x1b'));
        c.color = true;
        assert!(heading(&c, "Hi").starts_with("\x1b[1mHi\x1b[0m"));
    }

    #[test]
    fn narrow_width_wraps_long_lines() {
        let mut ch = choice();
        ch.explain = "word ".repeat(30);
        let out = step_screen(&Caps::plain(40), Step { index: 1, total: 2 }, &ch);
        assert!(out.lines().all(|l| l.chars().count() <= 60), "{out}");
    }

    #[test]
    fn progress_lines_use_ascii_or_unicode_symbols() {
        let a = Caps::plain(80);
        assert_eq!(
            progress_line(&a, Status::Done, "hooks", "29 entries added"),
            "[ok] hooks  29 entries added"
        );
        assert_eq!(
            progress_line(&a, Status::Skipped, "aliases", "you chose off"),
            "[skip] aliases  you chose off"
        );
        assert_eq!(
            progress_line(&a, Status::Failed, "path", "cannot write"),
            "[fail] path  cannot write"
        );
        let mut u = a;
        u.unicode = true;
        assert!(progress_line(&u, Status::Done, "x", "y").starts_with('\u{2714}'));
    }

    #[test]
    fn summary_aligns_names() {
        let rows = vec![
            SummaryRow {
                name: "hooks".into(),
                on: true,
                detail: "29 entries".into(),
            },
            SummaryRow {
                name: "system-prompt".into(),
                on: false,
                detail: "not installed".into(),
            },
        ];
        let got = summary(&Caps::plain(80), &rows);
        assert!(got.contains("  hooks          on   29 entries\n"), "{got}");
        assert!(
            got.contains("  system-prompt  off  not installed\n"),
            "{got}"
        );
    }

    #[test]
    fn next_steps_counts_and_lists() {
        let got = next_steps(&Caps::plain(80), [2, 1, 3, 0], &["Open a new terminal."]);
        assert!(got.contains("2 done, 1 unchanged, 3 skipped, 0 failed"));
        assert!(got.contains("Next steps"));
        assert!(got.contains("  Open a new terminal."));
    }

    #[test]
    fn prompt_shows_back_only_after_the_first_step() {
        let c = Caps::plain(80);
        assert_eq!(
            prompt_line(&c, "Add it?", true, false),
            "Add it? [Y/n, s skip, q quit] "
        );
        assert_eq!(
            prompt_line(&c, "Add it?", false, true),
            "Add it? [y/N, b back, s skip, q quit] "
        );
    }

    #[test]
    fn wrap_keeps_blank_lines_and_indents() {
        assert_eq!(wrap("a b\n\nc", 80, 2), "  a b\n\n  c");
    }
}
