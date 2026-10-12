// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The interactive usage view (`playbook usage`). `data` is the snapshot it
//! draws, `worker` keeps it fresh in the background, `app` is the state and
//! keys, `view` draws a frame, and `run` is the terminal loop around them.

pub mod app;
pub mod budget;
pub mod chart;
pub mod data;
pub mod fmt;
pub mod notice;
pub mod panels;
#[cfg(test)]
mod parity;
pub mod scale;
pub mod sessions_view;
#[cfg(test)]
mod snapshots;
pub mod sort;
pub mod theme;
pub mod view;
pub mod worker;

use super::query::Range;
use super::run::Paths;
use app::{App, Effect};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use sessions_view::SessionsView;
use std::time::Duration;
use theme::{ColorDepth, Theme};
use worker::{Update, Worker};

/// How long the loop waits for a key before it checks for new data.
const TICK: Duration = Duration::from_millis(200);

/// The theme to start with: `NO_COLOR` forces `mono`, else the `usage.theme`
/// config key, which is `btop` unless set.
fn start_theme(no_color: bool) -> Theme {
    if no_color {
        return Theme::Mono;
    }
    let home = crate::common::session::home_dir();
    crate::config::resolve_valid("usage.theme", &home, None)
        .ok()
        .and_then(|(value, _, _)| value.as_str().and_then(Theme::parse))
        .unwrap_or_default()
}

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
    app.sources = paths.claude_projects.display().to_string();
    app.week_start = super::entry::configured_week_start();
    // NO_COLOR counts when it is set to anything but empty.
    app.no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    app.theme = start_theme(app.no_color);
    app.depth = ColorDepth::detect(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    );
    let every = Duration::from_secs(super::entry::configured_refresh_seconds());
    app.refresh_every = every;
    let worker = Worker::spawn(paths.clone(), app.range, app.filter.clone(), every);
    let mut sessions = SessionsLoop::new(paths.clone());
    while !app.quit {
        app.area = terminal.size().map(Into::into)?;
        sessions.poll();
        terminal.draw(|frame| match &sessions.view {
            Some(v) => sessions_view::render(
                frame,
                v,
                &app.theme.palette(app.depth),
                crate::common::time::now_secs(),
            ),
            None => view::render(frame, &app),
        })?;
        if event::poll(TICK)? {
            // A resize needs no handling: the next draw reads the new size.
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press && sessions.view.is_some() => {
                    if sessions.key(key) == sessions_view::Action::Quit {
                        app.quit = true;
                    }
                }
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && key.code == ratatui::crossterm::event::KeyCode::Char('p')
                        && !app.editing =>
                {
                    sessions.open();
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.key(key) {
                    Effect::Requery => worker.ask(app.range, &app.filter),
                    Effect::None => {}
                },
                _ => {}
            }
        }
        while let Ok(update) = worker.updates.try_recv() {
            match update {
                Update::Data(data) => {
                    // A reply for an earlier range may still be in flight.
                    if data.range == app.range && data.filter == app.filter {
                        app.refresh(*data);
                    }
                }
                Update::Error(e) => app.error = Some(e),
                Update::Indexing(progress) => app.indexing = progress,
            }
        }
    }
    Ok(())
}

type Loaded = Result<(Vec<crate::sessions::Record>, i64), String>;

/// The Sessions management view and the thread that reads it. A read asks
/// `gh` for each branch, so it runs off the draw loop and the screen never
/// waits on it. At most one read is in flight.
struct SessionsLoop {
    paths: Paths,
    view: Option<SessionsView>,
    reply: Option<std::sync::mpsc::Receiver<Loaded>>,
    last_read: Option<std::time::Instant>,
}

impl SessionsLoop {
    fn new(paths: Paths) -> Self {
        SessionsLoop {
            paths,
            view: None,
            reply: None,
            last_read: None,
        }
    }

    fn open(&mut self) {
        self.view = Some(SessionsView::default());
        self.start();
    }

    fn start(&mut self) {
        if self.reply.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = self.paths.clone();
        std::thread::spawn(move || {
            let home = crate::common::session::home_dir();
            let _ = tx.send(crate::sessions::load(&paths, &home));
        });
        self.reply = Some(rx);
        self.last_read = Some(std::time::Instant::now());
    }

    fn key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> sessions_view::Action {
        let action = match self.view.as_mut() {
            Some(v) => v.key(key),
            None => sessions_view::Action::None,
        };
        match action {
            sessions_view::Action::Close => self.view = None,
            sessions_view::Action::Refresh => self.start(),
            _ => {}
        }
        action
    }

    /// Takes a finished read and starts the next one when the view is open and
    /// the last one is old.
    fn poll(&mut self) {
        let Some(view) = self.view.as_mut() else {
            self.reply = None;
            return;
        };
        if let Some(rx) = &self.reply {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok((records, now)) => view.refresh(records, now),
                    Err(e) => view.fail(e),
                }
                self.reply = None;
            }
        }
        let due = self
            .last_read
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(sessions_view::REFRESH_SECONDS));
        if due {
            self.start();
        }
    }
}
