// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Heap budget accounting and the counting global allocator (see ADR 0051).
//! It counts Rust heap allocations only, not mmap, C malloc or thread stacks.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

const MEMORY_LIMIT_MESSAGE: &[u8] = b"memory limit reached\n";
#[cfg(unix)]
const STDERR_FD: i32 = 2;

/// A byte budget shared by every thread. `used` never passes `cap`.
pub(crate) struct Budget {
    used: AtomicUsize,
    cap: AtomicUsize,
}

impl Budget {
    pub(crate) const fn new(cap: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            cap: AtomicUsize::new(cap),
        }
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }

    pub(crate) fn set_cap(&self, cap: usize) {
        self.cap.store(cap, Ordering::SeqCst);
    }

    /// Reserves `bytes`, or returns false and leaves the counter unchanged.
    pub(crate) fn try_reserve(&self, bytes: usize) -> bool {
        let previous = self.used.fetch_add(bytes, Ordering::SeqCst);
        let within = previous
            .checked_add(bytes)
            .is_some_and(|total| total <= self.cap.load(Ordering::SeqCst));
        if !within {
            self.used.fetch_sub(bytes, Ordering::SeqCst);
        }
        within
    }

    pub(crate) fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::SeqCst);
    }

    /// Adjusts a reservation from `old` to `new` bytes, with the same cap check on growth.
    pub(crate) fn try_resize(&self, old: usize, new: usize) -> bool {
        if new > old {
            self.try_reserve(new - old)
        } else {
            self.release(old - new);
            true
        }
    }
}

thread_local! {
    // const-initialised with no destructor, so touching it never allocates.
    static IN_LIMIT_EXIT: Cell<bool> = const { Cell::new(false) };
}

/// Writes the limit message and exits 4. Allocation free, so it is safe to
/// call from inside the allocator.
#[cfg(unix)]
pub(crate) fn exit_over_memory_limit() -> ! {
    if IN_LIMIT_EXIT.with(|flag| flag.replace(true)) {
        // SAFETY: _exit takes no pointers and never returns.
        unsafe { libc::_exit(i32::from(crate::EXIT_RESOURCE_LIMIT)) }
    }
    // SAFETY: the pointer and length describe a static byte string. A failed
    // write is ignored because the exit code still reports the limit.
    unsafe {
        libc::write(
            STDERR_FD,
            MEMORY_LIMIT_MESSAGE.as_ptr().cast(),
            MEMORY_LIMIT_MESSAGE.len(),
        );
        libc::_exit(i32::from(crate::EXIT_RESOURCE_LIMIT))
    }
}

/// Windows has no allocation-free write path here, so this fallback writes
/// through std. The cap still ends the process with exit code 4.
#[cfg(not(unix))]
pub fn exit_over_memory_limit() -> ! {
    if !IN_LIMIT_EXIT.with(|flag| flag.replace(true)) {
        let _ = std::io::Write::write_all(&mut std::io::stderr(), MEMORY_LIMIT_MESSAGE);
    }
    std::process::exit(i32::from(crate::EXIT_RESOURCE_LIMIT))
}

static BUDGET: Budget = Budget::new(usize::MAX);

/// Sets the heap cap, in bytes, for the whole process.
pub(crate) fn set_heap_cap(cap_bytes: usize) {
    BUDGET.set_cap(cap_bytes);
}

/// Wraps the system allocator and exits 4 when an allocation would pass the cap.
pub(crate) struct CountingAllocator;

// SAFETY: every call forwards to `System` with the caller's layout and
// pointer, so System's guarantees carry over. The accounting only adds counter
// updates and an allocation-free exit.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !BUDGET.try_reserve(layout.size()) {
            exit_over_memory_limit();
        }
        // SAFETY: the caller upholds GlobalAlloc::alloc's contract for `layout`.
        let ptr = unsafe { System.alloc(layout) };
        if ptr.is_null() {
            BUDGET.release(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !BUDGET.try_reserve(layout.size()) {
            exit_over_memory_limit();
        }
        // SAFETY: the caller upholds GlobalAlloc::alloc_zeroed's contract for `layout`.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if ptr.is_null() {
            BUDGET.release(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller passes a pointer and layout from a prior alloc.
        unsafe { System.dealloc(ptr, layout) };
        BUDGET.release(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !BUDGET.try_resize(layout.size(), new_size) {
            exit_over_memory_limit();
        }
        // SAFETY: the caller upholds GlobalAlloc::realloc's contract.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if new_ptr.is_null() {
            BUDGET.try_resize(new_size, layout.size());
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    const MIB: usize = 1 << 20;
    const CYCLES: usize = 10_000;
    const BLOCK_BYTES: usize = 4096;
    const GROWN_BYTES: usize = 8192;
    const SHRUNK_BYTES: usize = 1024;
    const BUDGET_BYTES: usize = 16 * MIB;
    const THREADS: usize = 8;
    const BLOCKS_PER_THREAD: usize = 4;

    #[test]
    fn counter_returns_to_its_start_after_alloc_free_and_realloc_cycles() {
        // Arrange
        let budget = Budget::new(BUDGET_BYTES);
        let start = budget.used();
        // Act
        for _ in 0..CYCLES {
            assert!(budget.try_reserve(BLOCK_BYTES));
            assert!(budget.try_resize(BLOCK_BYTES, GROWN_BYTES));
            assert!(budget.try_resize(GROWN_BYTES, SHRUNK_BYTES));
            budget.release(SHRUNK_BYTES);
        }
        // Assert
        assert_eq!(budget.used(), start);
    }

    #[test]
    fn an_allocation_up_to_the_budget_still_succeeds_after_many_cycles() {
        // Arrange
        let budget = Budget::new(BUDGET_BYTES);
        for _ in 0..CYCLES {
            assert!(budget.try_reserve(BLOCK_BYTES));
            budget.release(BLOCK_BYTES);
        }
        // Act
        let granted = budget.try_reserve(BUDGET_BYTES);
        // Assert
        assert!(granted);
    }

    #[test]
    fn a_reservation_one_byte_over_the_budget_is_refused_and_not_counted() {
        // Arrange
        let budget = Budget::new(BUDGET_BYTES);
        // Act
        let granted = budget.try_reserve(BUDGET_BYTES + 1);
        // Assert
        assert!(!granted);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn eight_threads_never_get_more_than_the_budget() {
        // Arrange
        let budget = Budget::new(BUDGET_BYTES);
        let granted = AtomicUsize::new(0);
        // Act
        thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..BLOCKS_PER_THREAD {
                        if budget.try_reserve(MIB) {
                            granted.fetch_add(MIB, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        // Assert
        assert_eq!(granted.load(Ordering::SeqCst), BUDGET_BYTES);
        assert_eq!(budget.used(), BUDGET_BYTES);
    }
}
