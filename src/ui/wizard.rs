// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The question loop: one screen per choice, with back, skip and quit, then a
//! summary and a confirm. It reads from any `BufRead` and writes to any
//! `Write`, so tests drive it with scripted input.

use super::render::{self, Choice, Step, SummaryRow};
use super::Caps;
use std::io::{self, BufRead, Write};

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
/// the summary for an answer.
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
    while pos < asked.len() {
        let item = &items[asked[pos]];
        let step = Step {
            index: pos + 1,
            total,
        };
        write!(out, "{}", render::step_screen(caps, step, &item.choice))?;
        loop {
            let default = answers[asked[pos]];
            write!(
                out,
                "{}",
                render::prompt_line(caps, &item.choice.question, default, pos > 0)
            )?;
            out.flush()?;
            let Some(line) = read_line(input) else {
                writeln!(out)?;
                return Ok(Finish::Cancelled);
            };
            match parse_answer(&line, default) {
                Answer::Yes => answers[asked[pos]] = true,
                Answer::No => answers[asked[pos]] = false,
                Answer::Skip => answers[asked[pos]] = item.default,
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
    loop {
        write!(out, "\n{}", render::summary(caps, &rows))?;
        write!(
            out,
            "{}",
            render::prompt_line(caps, "Apply these changes?", true, !asked.is_empty())
        )?;
        out.flush()?;
        let Some(line) = read_line(input) else {
            writeln!(out)?;
            return Ok(Finish::Cancelled);
        };
        match parse_answer(&line, true) {
            Answer::Yes | Answer::Skip => return Ok(Finish::Confirmed(answers)),
            Answer::No | Answer::Quit => return Ok(Finish::Cancelled),
            Answer::Back if !asked.is_empty() => {
                // Walk back to the last asked item and redo from there.
                return run_from_last(caps, items, detail, input, out, answers);
            }
            _ => writeln!(out, "Type y to apply, n to cancel.")?,
        }
    }
}

fn run_from_last(
    caps: &Caps,
    items: &[Item],
    detail: &dyn Fn(&Item, bool) -> String,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    answers: Vec<bool>,
) -> io::Result<Finish> {
    // Keep what was answered as the new defaults and ask again from the start.
    let again: Vec<Item> = items
        .iter()
        .zip(&answers)
        .map(|(i, &a)| Item {
            choice: i.choice.clone(),
            default: a,
            fixed: i.fixed,
        })
        .collect();
    run(caps, &again, detail, input, out)
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
