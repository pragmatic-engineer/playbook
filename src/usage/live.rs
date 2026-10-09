// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The live stream behind `/api/live`: Server-Sent Events over a chunked
//! response, one thread per open stream. A shared cache means several streams
//! (or several tabs) trigger one ingest per tick, not one each.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
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

/// A write that has not finished after this long means the client stopped
/// reading with the socket still open: the stream ends and its slot is freed.
pub const WRITE_DEADLINE: Duration = Duration::from_secs(30);
/// Writers still blocked on an abandoned socket. Past this many, new streams
/// get a 503, so a client that opens and stalls streams cannot pile up threads.
pub const MAX_STUCK: usize = 16;

/// The clocks of one stream. Defaults are the production values; the
/// `PLAYBOOK_LIVE_*_MS` variables shorten them so tests need not wait.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub tick: Duration,
    pub lifetime: Duration,
    pub write_deadline: Duration,
    /// How long a request that finds every slot taken waits for one to free
    /// before it gets a 503: a client that just left is noticed within a tick
    /// or two, so a quick reopen must not be refused for that.
    pub reopen_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing::with_tick(TICK)
    }
}

impl Timing {
    fn with_tick(tick: Duration) -> Self {
        Timing {
            tick,
            lifetime: MAX_LIFETIME,
            write_deadline: WRITE_DEADLINE,
            reopen_grace: tick * 2,
        }
    }

    /// The defaults, with each clock overridden by its `PLAYBOOK_LIVE_*_MS`
    /// variable when that holds a whole number of milliseconds.
    pub fn from_env() -> Self {
        let ms = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_millis)
        };
        let mut t = Timing::with_tick(ms("PLAYBOOK_LIVE_TICK_MS").unwrap_or(TICK));
        if let Some(d) = ms("PLAYBOOK_LIVE_WRITE_DEADLINE_MS") {
            t.write_deadline = d;
        }
        if let Some(d) = ms("PLAYBOOK_LIVE_REOPEN_GRACE_MS") {
            t.reopen_grace = d;
        }
        t
    }
}

const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nTransfer-Encoding: chunked\r\n\r\n";

type Cached = Option<(Instant, Result<Arc<String>, String>)>;

#[derive(Default)]
pub struct Live {
    open: AtomicUsize,
    stuck: Arc<AtomicUsize>,
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

    /// A handle on the count of writers blocked on an abandoned socket.
    pub fn stuck(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.stuck)
    }

    pub fn try_acquire(self: &Arc<Self>, max: usize) -> Option<Slot> {
        if self.stuck.load(Ordering::SeqCst) >= MAX_STUCK {
            return None;
        }
        let taken = self
            .open
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .is_ok();
        taken.then(|| Slot(Arc::clone(self)))
    }

    /// Like `try_acquire`, but a full house gets up to `grace` to free a slot
    /// first, polling often so a freed slot is taken at once.
    pub fn acquire_within(self: &Arc<Self>, max: usize, grace: Duration) -> Option<Slot> {
        let until = Instant::now() + grace;
        loop {
            if let Some(slot) = self.try_acquire(max) {
                return Some(slot);
            }
            let now = Instant::now();
            if now >= until {
                return None;
            }
            std::thread::sleep((until - now).min(Duration::from_millis(25)));
        }
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

/// `stream`, with the writes done on their own thread so a client that stops
/// reading cannot hold the stream: if a write is still pending after
/// `timing.write_deadline` the stream ends with `TimedOut` (freeing the
/// caller's slot) and the blocked writer is counted in `stuck` until its write
/// finally fails or completes. The socket cannot be closed from here, which is
/// why `Live::try_acquire` limits `stuck`.
pub fn stream_guarded<W: Write + Send + 'static>(
    out: W,
    mut next: impl FnMut() -> Result<Arc<String>, String>,
    timing: Timing,
    stuck: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    type Job = (Vec<u8>, mpsc::Sender<std::io::Result<()>>);
    let (jobs, inbox) = mpsc::channel::<Job>();
    let abandoned = Arc::new(AtomicBool::new(false));
    let (flag, count) = (Arc::clone(&abandoned), Arc::clone(&stuck));
    std::thread::Builder::new()
        .name("usage-live-writer".to_string())
        .spawn(move || {
            let mut out = out;
            while let Ok((bytes, reply)) = inbox.recv() {
                let done = out.write_all(&bytes).and_then(|()| out.flush());
                let failed = done.is_err();
                let _ = reply.send(done);
                if flag.load(Ordering::SeqCst) {
                    count.fetch_sub(1, Ordering::SeqCst);
                    return;
                }
                if failed {
                    return;
                }
            }
        })?;
    let started = Instant::now();
    let send = |bytes: Vec<u8>| -> std::io::Result<()> {
        let (reply, answer) = mpsc::channel();
        jobs.send((bytes, reply))
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        match answer.recv_timeout(timing.write_deadline) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Count first, then flag: the writer decrements only if flagged.
                stuck.fetch_add(1, Ordering::SeqCst);
                abandoned.store(true, Ordering::SeqCst);
                Err(std::io::ErrorKind::TimedOut.into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    };
    send(HEAD.as_bytes().to_vec())?;
    loop {
        let text = match next() {
            Ok(body) => event("live", &body),
            Err(e) => event("problem", &serde_json::json!({ "error": e }).to_string()),
        };
        send(text.into_bytes())?;
        if started.elapsed() + timing.tick >= timing.lifetime {
            return send(b"0\r\n\r\n".to_vec());
        }
        std::thread::sleep(timing.tick);
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
        assert_eq!(live.open.load(Ordering::SeqCst), MAX_STREAMS);
        drop(held);

        assert_eq!(live.open.load(Ordering::SeqCst), 0);
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

    /// A writer that blocks inside `write` until released, like a socket whose
    /// peer stopped reading and whose buffers are full.
    struct Stalled(Arc<(Mutex<bool>, std::sync::Condvar)>);

    impl Write for Stalled {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let (lock, cv) = &*self.0;
            let mut released = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*released {
                released = cv.wait(released).unwrap_or_else(|e| e.into_inner());
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn fast() -> Timing {
        Timing {
            tick: Duration::from_millis(2),
            lifetime: Duration::from_secs(60),
            write_deadline: Duration::from_millis(80),
            reopen_grace: Duration::from_millis(50),
        }
    }

    #[test]
    fn a_client_that_stops_reading_ends_the_stream_and_the_writer_is_counted_until_it_unblocks() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let stuck = Arc::new(AtomicUsize::new(0));

        let result = stream_guarded(
            Stalled(Arc::clone(&gate)),
            || Ok(Arc::new("{}".to_string())),
            fast(),
            Arc::clone(&stuck),
        );

        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(
            stuck.load(Ordering::SeqCst),
            1,
            "the writer is still blocked"
        );
        // The peer finally reads or dies: the blocked write returns and the count drops.
        *gate.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        gate.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(10);
        while stuck.load(Ordering::SeqCst) != 0 {
            assert!(Instant::now() < deadline, "the stuck count never dropped");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_guarded_stream_writes_the_same_bytes_as_the_plain_one() {
        let shared = Arc::new(Mutex::new(Vec::new()));
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let timing = Timing {
            tick: Duration::from_millis(5),
            lifetime: Duration::from_millis(18),
            ..fast()
        };
        let mut n = 0;
        stream_guarded(
            Sink(Arc::clone(&shared)),
            || {
                n += 1;
                Ok(Arc::new(format!("{{\"n\":{n}}}")))
            },
            timing,
            Arc::new(AtomicUsize::new(0)),
        )
        .unwrap();

        let mut plain = Vec::new();
        let mut m = 0;
        stream(
            &mut plain,
            || {
                m += 1;
                Ok(Arc::new(format!("{{\"n\":{m}}}")))
            },
            timing.tick,
            timing.lifetime,
        )
        .unwrap();
        let got = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(got.starts_with(&plain[..plain.len().min(40)]));
        assert!(got.ends_with(b"0\r\n\r\n"));
    }

    #[test]
    fn a_client_that_went_away_ends_a_guarded_stream_with_an_error() {
        let result = stream_guarded(
            Broken(2),
            || Ok(Arc::new("{}".to_string())),
            fast(),
            Arc::new(AtomicUsize::new(0)),
        );
        assert!(result.is_err());
    }

    #[test]
    fn too_many_stuck_writers_refuse_new_streams_until_they_clear() {
        let live = Live::new();
        live.stuck.store(MAX_STUCK, Ordering::SeqCst);
        assert!(live.try_acquire(MAX_STREAMS).is_none());
        live.stuck.store(0, Ordering::SeqCst);
        assert!(live.try_acquire(MAX_STREAMS).is_some());
    }

    #[test]
    fn a_full_house_waits_for_a_slot_to_free_and_then_gives_up() {
        let live = Live::new();
        let held: Vec<Slot> = (0..MAX_STREAMS)
            .map(|_| live.try_acquire(MAX_STREAMS).expect("room"))
            .collect();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            drop(held);
        });
        // Generous grace so a loaded machine still sees the release.
        let first = live.acquire_within(MAX_STREAMS, Duration::from_secs(10));
        assert!(first.is_some());
        releaser.join().unwrap();

        let _all: Vec<Slot> = (1..MAX_STREAMS)
            .map(|_| live.try_acquire(MAX_STREAMS).expect("room"))
            .collect();
        assert!(live
            .acquire_within(MAX_STREAMS, Duration::from_millis(30))
            .is_none());
    }
}
