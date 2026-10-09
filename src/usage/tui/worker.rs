// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Keeps the view fresh without blocking the screen. A background thread owns
//! the database connection: it ingests new transcript lines and re-reads the
//! store on a timer, and at once when the range or filter changes, with the
//! same `ingest_new` the web live stream calls.

use super::data::{self, Data};
use crate::common::time::now_secs;
use crate::usage::query::Range;
use crate::usage::run::{ingest_new, Paths};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often the store is re-read when nothing else asks.
pub const REFRESH: Duration = Duration::from_secs(2);

pub enum Request {
    /// Show this range and filter now, then keep refreshing them.
    Query { range: Range, filter: String },
}

pub enum Update {
    Data(Box<Data>),
    Error(String),
}

pub struct Worker {
    requests: Sender<Request>,
    pub updates: Receiver<Update>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(paths: Paths, range: Range, filter: String) -> Worker {
        let (requests, request_rx) = mpsc::channel::<Request>();
        let (update_tx, updates) = mpsc::channel::<Update>();
        let handle = std::thread::spawn(move || {
            let mut query = (range, filter);
            loop {
                let update = match ingest_new(&paths) {
                    Ok((conn, _)) => match data::load(&conn, query.0, &query.1, now_secs()) {
                        Ok(d) => Update::Data(Box::new(d)),
                        Err(e) => Update::Error(e),
                    },
                    Err(e) => Update::Error(e),
                };
                if update_tx.send(update).is_err() {
                    return;
                }
                match request_rx.recv_timeout(REFRESH) {
                    Ok(Request::Query { range, filter }) => query = (range, filter),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Worker {
            requests,
            updates,
            handle: Some(handle),
        }
    }

    pub fn ask(&self, range: Range, filter: &str) {
        let _ = self.requests.send(Request::Query {
            range,
            filter: filter.to_string(),
        });
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the request channel ends the loop after its current pass; the
        // handle is dropped without a join so quitting never waits on an ingest.
        drop(self.handle.take());
    }
}
