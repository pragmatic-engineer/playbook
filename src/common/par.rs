// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Bounded fan-out over `std::thread::scope`. No runtime, no extra crates: the
//! callers wait on child processes (`gh`, `git`), so a few plain threads are
//! enough. Results come back in input order, so output stays deterministic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Most calls in flight at once: enough to overlap the waits, low enough to
/// stay clear of `gh` rate limits and a loaded machine.
pub const MAX_CONCURRENT: usize = 4;

/// `f` over every item with at most `cap` threads, results in input order.
/// A panic in `f` propagates to the caller, as it would in a plain loop.
pub fn map<I, T, F>(items: &[I], cap: usize, f: F) -> Vec<T>
where
    I: Sync,
    T: Send,
    F: Fn(&I) -> T + Sync,
{
    let workers = cap.max(1).min(items.len());
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let value = f(item);
                *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every slot is filled before the scope ends")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[test]
    fn results_keep_input_order_whatever_the_finish_order() {
        let items: Vec<u64> = (0..12).collect();
        let out = map(&items, 4, |n| {
            std::thread::sleep(Duration::from_millis((12 - n) * 2));
            n * 10
        });
        assert_eq!(out, (0..12).map(|n| n * 10).collect::<Vec<_>>());
    }

    #[test]
    fn no_more_than_the_cap_run_at_once() {
        let live = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items = [0u8; 16];
        map(&items, 3, |_| {
            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(10));
            live.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert!(peak.load(Ordering::SeqCst) >= 2, "work never overlapped");
    }

    #[test]
    fn empty_and_single_inputs_work_without_threads() {
        assert!(map::<u8, u8, _>(&[], 4, |n| *n).is_empty());
        assert_eq!(map(&[5u8], 4, |n| n + 1), vec![6]);
    }

    #[test]
    fn a_cap_of_zero_still_runs_everything() {
        assert_eq!(map(&[1u8, 2, 3], 0, |n| *n), vec![1, 2, 3]);
    }
}
