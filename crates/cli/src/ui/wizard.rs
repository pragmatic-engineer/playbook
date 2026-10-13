// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The question loop: one screen per choice, with back, skip and quit, then a
//! summary and a confirm. It reads from any `BufRead` and writes to any
//! `Write`, so tests drive it with scripted input.

use super::render::{self, Choice, Step, SummaryRow};
use super::Caps;
use std::io::{self, BufRead, Write};

/// The most screens one wizard run shows.
pub const MAX_ITEMS: usize = 32;

/// What a typed line means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Back,
    Skip,
    Quit,
    /// Not understood: ask again.
    Unknown,
}

/// Read one answer. An empty line takes the default.
pub fn parse_answer(line: &str, default_yes: bool) -> Answer {
    match line.trim().to_ascii_lowercase().as_str() {
        "" => {
            if default_yes {
                Answer::Yes
            } else {
                Answer::No
            }
        }
        "y" | "yes" => Answer::Yes,
        "n" | "no" => Answer::No,
        "b" | "back" => Answer::Back,
        "s" | "skip" => Answer::Skip,
        "q" | "quit" => Answer::Quit,
        _ => Answer::Unknown,
    }
}

/// One wizard item: the screen and where the answer starts.
pub struct Item {
    pub choice: Choice,
    /// The answer when the user presses Enter or skips.
    pub default: bool,
    /// Already decided by a flag: shown in the summary, never asked.
    pub fixed: Option<bool>,
}

/// How the wizard ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    /// The user confirmed. One answer per item, in order.
    Confirmed(Vec<bool>),
    /// The user quit, or input ended before the confirm. Nothing should change.
    Cancelled,
}

fn read_line(input: &mut dyn BufRead) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

/// Run every screen, then the summary and confirm. `detail` words each row of
/// the summary for an answer. Enter keeps the current answer, which starts as
/// the item's default. `s` restores the default. Back at the summary returns
/// to the last question.
pub fn run(
    caps: &Caps,
    items: &[Item],
    detail: &dyn Fn(&Item, bool) -> String,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> io::Result<Finish> {
    let asked: Vec<usize> = (0..items.len())
        .filter(|&i| items[i].fixed.is_none())
        .collect();
    let total = asked.len() + 1; // the summary is the last step
    let mut answers: Vec<bool> = items.iter().map(|i| i.fixed.unwrap_or(i.default)).collect();
    let mut pos = 0;
    // pos in 0..asked.len() is a question, pos == asked.len() is the summary.
    loop {
        if pos < asked.len() {
            let idx = asked[pos];
            let item = &items[idx];
            let step = Step {
                index: pos + 1,
                total,
            };
            write!(out, "{}", render::step_screen(caps, step, &item.choice))?;
            loop {
                write!(
                    out,
                    "{}",
                    render::prompt_line(caps, &item.choice.question, answers[idx], pos > 0, true)
                )?;
                out.flush()?;
                let Some(line) = read_line(input) else {
                    writeln!(out)?;
                    return Ok(Finish::Cancelled);
                };
                match parse_answer(&line, answers[idx]) {
                    Answer::Yes => answers[idx] = true,
                    Answer::No => answers[idx] = false,
                    Answer::Skip => answers[idx] = item.default,
                    Answer::Back if pos > 0 => {
                        pos -= 1;
                        break;
                    }
                    Answer::Back => {
                        writeln!(out, "This is the first question.")?;
                        continue;
                    }
                    Answer::Quit => return Ok(Finish::Cancelled),
                    Answer::Unknown => {
                        writeln!(out, "Type y, n, s, b or q.")?;
                        continue;
                    }
                }
                pos += 1;
                break;
            }
            continue;
        }
        let rows: Vec<SummaryRow> = items
            .iter()
            .zip(&answers)
            .map(|(i, &on)| SummaryRow {
                name: i.choice.id.clone(),
                on,
                detail: detail(i, on),
            })
            .collect();
        write!(
            out,
            "\n{}",
            render::summary(
                caps,
                &rows,
                Step {
                    index: total,
                    total
                }
            )
        )?;
        write!(
            out,
            "{}",
            render::prompt_line(caps, "Apply these changes?", true, !asked.is_empty(), false)
        )?;
        out.flush()?;
        let Some(line) = read_line(input) else {
            writeln!(out)?;
            return Ok(Finish::Cancelled);
        };
        match parse_answer(&line, true) {
            Answer::Yes => return Ok(Finish::Confirmed(answers)),
            Answer::No | Answer::Quit => return Ok(Finish::Cancelled),
            Answer::Back if !asked.is_empty() => pos = asked.len() - 1,
            _ => writeln!(out, "Type y to apply, n to cancel, b to go back.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn item(id: &str, default: bool, fixed: Option<bool>) -> Item {
        Item {
            choice: Choice {
                id: id.into(),
                title: id.into(),
                explain: "e".into(),
                accept: vec!["a".into()],
                decline: vec!["d".into()],
                recommended: default,
                question: format!("Do {id}?"),
            },
            default,
            fixed,
        }
    }

    fn drive(items: &[Item], script: &str) -> (Finish, String) {
        let mut out = Vec::new();
        let mut input = Cursor::new(script.as_bytes().to_vec());
        let f = run(
            &Caps::plain(80),
            items,
            &|_, on| if on { "yes".into() } else { "no".into() },
            &mut input,
            &mut out,
        )
        .unwrap_or(Finish::Cancelled);
        (f, String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn answers_parse() {
        assert_eq!(parse_answer("", true), Answer::Yes);
        assert_eq!(parse_answer("", false), Answer::No);
        assert_eq!(parse_answer(" B ", true), Answer::Back);
        assert_eq!(parse_answer("skip", true), Answer::Skip);
        assert_eq!(parse_answer("x", true), Answer::Unknown);
    }

    #[test]
    fn enter_takes_defaults_and_confirms() {
        let items = [item("a", true, None), item("b", false, None)];
        let (f, out) = drive(&items, "\n\n\n");
        assert_eq!(f, Finish::Confirmed(vec![true, false]));
        assert!(out.contains("Step 1 of 3"));
        assert!(out.contains("Step 2 of 3"));
        assert!(out.contains("Summary of your choices"));
    }

    #[test]
    fn back_returns_to_the_previous_screen() {
        let items = [item("a", true, None), item("b", true, None)];
        let (f, out) = drive(&items, "y\nb\nn\nn\ny\n");
        // a=y, back to a, a=n, b=n, confirm
        assert_eq!(f, Finish::Confirmed(vec![false, false]));
        assert_eq!(out.matches("Step 1 of 3").count(), 2);
    }

    #[test]
    fn back_on_the_first_screen_is_refused() {
        let items = [item("a", true, None)];
        let (f, out) = drive(&items, "b\ny\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![true]));
        assert!(out.contains("This is the first question."));
    }

    #[test]
    fn skip_keeps_the_default() {
        let items = [item("a", false, None)];
        let (f, _) = drive(&items, "s\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![false]));
    }

    #[test]
    fn quit_and_end_of_input_cancel() {
        let items = [item("a", true, None)];
        assert_eq!(drive(&items, "q\n").0, Finish::Cancelled);
        assert_eq!(drive(&items, "").0, Finish::Cancelled);
        assert_eq!(drive(&items, "y\n").0, Finish::Cancelled);
    }

    #[test]
    fn back_at_the_summary_returns_to_the_last_question_only() {
        let items = [item("a", true, None), item("b", true, None)];
        // a=y, b=y, back from summary, b=n, confirm
        let (f, out) = drive(&items, "y\ny\nb\nn\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![true, false]));
        assert_eq!(out.matches("Step 1 of 3").count(), 1);
        assert_eq!(out.matches("Step 2 of 3").count(), 2);
    }

    #[test]
    fn skip_at_the_summary_does_not_apply() {
        let items = [item("a", true, None)];
        let (f, out) = drive(&items, "y\ns\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![true]));
        assert!(out.contains("Type y to apply"));
        assert!(!out.contains("Apply these changes? [Y/n, s skip"));
    }

    #[test]
    fn back_with_every_item_fixed_reprompts() {
        let items = [item("a", true, Some(true))];
        let (f, out) = drive(&items, "b\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![true]));
        assert!(out.contains("Type y to apply"));
    }

    #[test]
    fn skip_restores_the_default_after_going_back() {
        let items = [item("a", false, None), item("b", true, None)];
        // a=y, b=y, back, back, skip a (default false), b=y, confirm
        let (f, _) = drive(&items, "y\ny\nb\nb\ns\n\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![false, true]));
    }

    #[test]
    fn declining_the_confirm_cancels() {
        let items = [item("a", true, None)];
        assert_eq!(drive(&items, "y\nn\n").0, Finish::Cancelled);
    }

    #[test]
    fn a_fixed_item_is_not_asked_but_appears_in_the_summary() {
        let items = [item("a", true, Some(false)), item("b", true, None)];
        let (f, out) = drive(&items, "\n\n");
        assert_eq!(f, Finish::Confirmed(vec![false, true]));
        assert!(out.contains("Step 1 of 2"));
        assert!(!out.contains("Do a?"));
        assert!(out.contains("a  off"));
    }

    #[test]
    fn unknown_input_asks_again() {
        let items = [item("a", true, None)];
        let (f, out) = drive(&items, "huh\ny\ny\n");
        assert_eq!(f, Finish::Confirmed(vec![true]));
        assert!(out.contains("Type y, n, s, b or q."));
    }
}
