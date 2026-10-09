// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The interactive usage view (`playbook usage`). `data` is the snapshot it
//! draws, `worker` keeps it fresh in the background, `app` is the state and
//! keys, `view` draws a frame, and `run` is the terminal loop around them.

pub mod app;
pub mod data;
pub mod fmt;
pub mod panels;
pub mod theme;
pub mod view;
pub mod worker;

use super::query::Range;
use super::run::Paths;
use app::{App, Effect};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::time::Duration;
use worker::{Update, Worker};

/// How long the loop waits for a key before it checks for new data.
const TICK: Duration = Duration::from_millis(200);

/// Runs the view until the user quits. Restores the terminal on every exit,
/// including a panic (ratatui installs the hook).
pub fn run(paths: &Paths, range: Range) -> Result<(), String> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, paths, range);
    ratatui::restore();
    result.map_err(|e| format!("terminal error: {e}"))
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    paths: &Paths,
    range: Range,
) -> std::io::Result<()> {
    let mut app = App::new(range);
    let worker = Worker::spawn(paths.clone(), app.range, app.filter.clone());
    while !app.quit {
        terminal.draw(|frame| view::render(frame, &app))?;
        if event::poll(TICK)? {
            // A resize needs no handling: the next draw reads the new size.
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && app.key(key) == Effect::Requery {
                    worker.ask(app.range, &app.filter);
                }
            }
        }
        while let Ok(update) = worker.updates.try_recv() {
            match update {
                Update::Data(data) => {
                    // A reply for an earlier range may still be in flight.
                    if data.range == app.range && data.filter == app.filter {
                        app.data = Some(*data);
                        app.error = None;
                    }
                }
                Update::Error(e) => app.error = Some(e),
            }
        }
    }
    Ok(())
}
