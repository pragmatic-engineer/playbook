// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The live stream behind `/api/live`: Server-Sent Events over a chunked
//! response, one thread per open stream. A shared cache means several streams
//! (or several tabs) trigger one ingest per tick, not one each.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const EVENT_STREAM: &str = "text/event-stream";
/// Streams open at once; the next one gets a 503.
pub const MAX_STREAMS: usize = 4;
pub const TICK: Duration = Duration::from_secs(2);
/// A stream ends after this long and the page reconnects.
pub const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// Younger than a tick, so each tick is fresh, yet streams ticking together share one.
const CACHE_TTL: Duration = Duration::from_millis(1500);

const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nTransfer-Encoding: chunked\r\n\r\n";

type Cached = Option<(Instant, Result<Arc<String>, String>)>;

#[derive(Default)]
pub struct Live {
    open: AtomicUsize,
    cache: Mutex<Cached>,
}

/// One open stream's place. Dropping it frees the place.
pub struct Slot(Arc<Live>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Live {
    pub fn new() -> Arc<Live> {
        Arc::new(Live::default())
    }

    pub fn try_acquire(self: &Arc<Self>, max: usize) -> Option<Slot> {
        let taken = self
            .open
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .is_ok();
        taken.then(|| Slot(Arc::clone(self)))
    }

    pub fn open_streams(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }

    /// The latest summary (or failure), computed at most once per `CACHE_TTL`
    /// however many streams ask, so a busy database is not retried by each. The lock is held while computing, so a second stream waits
    /// for the first instead of ingesting again.
    pub fn summary(
        &self,
        compute: impl FnOnce() -> Result<String, String>,
    ) -> Result<Arc<String>, String> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, answer)) = cache.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return answer.clone();
            }
        }
        let answer = compute().map(Arc::new);
        *cache = Some((Instant::now(), answer.clone()));
        answer
    }
}

fn chunk(payload: &str) -> String {
    format!("{:x}\r\n{payload}\r\n", payload.len())
}

fn event(name: &str, data: &str) -> String {
    chunk(&format!("event: {name}\ndata: {data}\n\n"))
}

/// Writes the response head, then one event now and one per `tick` until
/// `lifetime` passes (then a clean end of body). A write error means the client
/// left: it ends the stream and frees the caller's slot.
pub fn stream<W: Write>(
    mut out: W,
    mut next: impl FnMut() -> Result<Arc<String>, String>,
    tick: Duration,
    lifetime: Duration,
) -> std::io::Result<()> {
    let started = Instant::now();
    out.write_all(HEAD.as_bytes())?;
    loop {
        let text = match next() {
            Ok(body) => event("live", &body),
            Err(e) => event("problem", &serde_json::json!({ "error": e }).to_string()),
        };
        out.write_all(text.as_bytes())?;
        out.flush()?;
        if started.elapsed() + tick >= lifetime {
            out.write_all(b"0\r\n\r\n")?;
            return out.flush();
        }
        std::thread::sleep(tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken(usize);

    impl Write for Broken {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.0 == 0 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.0 -= 1;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn slots_are_capped_and_freed_on_drop() {
        let live = Live::new();

        let held: Vec<Slot> = (0..MAX_STREAMS)
            .map(|_| live.try_acquire(MAX_STREAMS).expect("room"))
            .collect();
        assert!(live.try_acquire(MAX_STREAMS).is_none());
        assert_eq!(live.open_streams(), MAX_STREAMS);
        drop(held);

        assert_eq!(live.open_streams(), 0);
        assert!(live.try_acquire(MAX_STREAMS).is_some());
    }

    #[test]
    fn concurrent_streams_share_one_computation_until_the_cache_expires() {
        let live = Live::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let ask = |live: Arc<Live>, runs: Arc<AtomicUsize>| {
            std::thread::spawn(move || {
                live.summary(|| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(150));
                    Ok("one".to_string())
                })
                .unwrap()
            })
        };

        let (a, b) = (
            ask(Arc::clone(&live), Arc::clone(&runs)),
            ask(Arc::clone(&live), Arc::clone(&runs)),
        );
        let (a, b) = (a.join().unwrap(), b.join().unwrap());

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&a, &b));
        std::thread::sleep(CACHE_TTL + Duration::from_millis(50));
        assert_eq!(
            live.summary(|| Ok("three".to_string())).unwrap().as_str(),
            "three"
        );
    }

    #[test]
    fn a_failure_is_shared_briefly_too_and_then_retried() {
        let live = Live::new();
        let mut runs = 0;

        let first = live.summary(|| {
            runs += 1;
            Err("db busy".to_string())
        });
        let second = live.summary(|| {
            runs += 1;
            Ok("late".to_string())
        });

        assert_eq!(first.unwrap_err(), "db busy");
        assert_eq!(second.unwrap_err(), "db busy");
        assert_eq!(runs, 1, "a busy database is not retried by every stream");
        std::thread::sleep(CACHE_TTL + Duration::from_millis(50));
        assert_eq!(
            live.summary(|| Ok("ok".to_string())).unwrap().as_str(),
            "ok"
        );
    }

    #[test]
    fn events_are_framed_as_chunks_with_a_byte_length() {
        let framed = event("live", "{\"r\":\"é\"}");

        // "event: live\ndata: {"r":"é"}\n\n" is 30 bytes (é is two), 0x1e.
        assert_eq!(framed, "1e\r\nevent: live\ndata: {\"r\":\"é\"}\n\n\r\n");
    }

    #[test]
    fn the_stream_sends_the_head_an_event_per_tick_and_ends_cleanly_at_its_lifetime() {
        let mut out = Vec::new();
        let mut n = 0;

        stream(
            &mut out,
            || {
                n += 1;
                Ok(Arc::new(format!("{{\"n\":{n}}}")))
            },
            Duration::from_millis(5),
            Duration::from_millis(18),
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/event-stream"));
        assert!(text.contains("event: live\ndata: {\"n\":1}\n\n"));
        assert!(text.contains("{\"n\":2}"));
        assert!(text.ends_with("0\r\n\r\n"));
    }

    #[test]
    fn a_client_that_went_away_ends_the_stream_with_an_error() {
        let result = stream(
            Broken(2),
            || Ok(Arc::new("{}".to_string())),
            Duration::from_millis(1),
            Duration::from_secs(60),
        );

        assert!(result.is_err());
    }

    #[test]
    fn a_failing_summary_is_reported_as_an_event_not_a_dropped_stream() {
        let mut out = Vec::new();

        stream(
            &mut out,
            || Err("db busy".to_string()),
            Duration::from_millis(1),
            Duration::from_millis(1),
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("event: problem\ndata: {\"error\":\"db busy\"}"));
    }
}
