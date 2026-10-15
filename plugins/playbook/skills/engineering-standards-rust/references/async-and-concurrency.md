# Async and concurrency

Threads, locks, channels and async tasks, with Tokio as the example runtime. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Blocking and locks: rules 1 to 6
- Tasks and cancellation: rules 7 to 12
- Sharing between threads: rules 13 to 16

## Blocking and locks

1. **Do not block a runtime thread.** Inside `async fn`, avoid `std::fs`, `std::thread::sleep`, blocking clients and long CPU loops. Use `tokio::fs`, `tokio::time::sleep` and async clients.
   Why: a task gives up the thread only at `.await`, and one blocking call stalls every task on that thread.

2. **Move blocking work off the async threads.** Use `spawn_blocking` for short blocking I/O, `rayon` for heavy CPU work, and a dedicated `std::thread` for work that runs forever. Limit CPU jobs on `spawn_blocking` with a semaphore.
   Why: the blocking pool is large and made for I/O. A runtime cannot abort a blocking job, and shutdown waits for it.

3. **Use `std::sync::Mutex` for plain data, even in async code.** Use `tokio::sync::Mutex` only when the guard must live across an `.await`, usually around an I/O resource.
   Why: the async mutex is slower, and a short lock with low contention is fine.

4. **Do not hold a sync lock guard or a `RefCell` borrow across `.await`.** This covers `std::sync` and `parking_lot` guards. A `tokio::sync` guard may cross an `.await` (rule 3). Lock in a small sync function, or end the guard's scope with a block before the await.

   ```rust
   // wrong: the guard is alive at the await
   let mut map = state.lock().unwrap();
   map.insert(key, value);
   client.notify().await;
   // right
   {
       let mut map = state.lock().unwrap();
       map.insert(key, value);
   }
   client.notify().await;
   ```

   Why: a std guard makes the future `!Send`, and a `Send` guard can deadlock against another task on the same thread.
   Check: `clippy::await_holding_lock`, `clippy::await_holding_refcell_ref`.
   Canonical's guide prefers a scope block over `drop(guard)` in all code. This skill asks for the block before an `.await`, because it shows where the guard ends.

5. **Bind the lock result before `match` or `if let`.** A temporary in the scrutinee lives through the whole `match` body. In the 2024 edition, an `if let` temporary is dropped before the `else` branch, and it still lives through the `then` branch.
   Why: locking the same lock again inside the body deadlocks.
   Check: `clippy::significant_drop_in_scrutinee` (nursery), and rustc `if_let_rescope` during an edition migration.

6. **Decide what to do on a poisoned `Mutex`.** `lock().unwrap()` passes the first panic on, and it is the usual choice. Recover with `into_inner()` only when a panic mid-update cannot break the data.
   Why: poisoning means a holder panicked, maybe halfway through a change. The Tokio mutex does not poison.

## Tasks and cancellation

7. **Dropping a future cancels it at its current `.await`.** `select!`, `timeout`, `try_join!` and an early return all drop futures. Write each step so that stopping there leaves the state valid.
   Check: `clippy::let_underscore_future` for `let _ = fut;`, which drops a future without running it.

8. **Use only cancel-safe branches in a `select!` loop.** `mpsc::Receiver::recv` and `TcpListener::accept` are cancel safe. `read_exact`, `write_all` and `read_to_end` are not. Pin a long future outside the loop and poll it by reference.
   Why: the losing branch is dropped and built again on the next pass, and partial reads and writes are lost.

9. **Track every spawned task.** Keep the `JoinHandle`, or use `JoinSet` or `TaskTracker` (from `tokio-util`). Handle the `JoinError` from a panic or a cancel.
   Why: dropping a `JoinHandle` detaches the task. It keeps running, and its errors and panics go nowhere.

10. **Shut down in three steps.** Detect the signal, broadcast stop with a `CancellationToken` (from `tokio-util`), then wait for tasks with a deadline.
    Why: tasks can flush their work, and the program knows when it is done.

11. **Use bounded channels.** Every `unbounded_channel` needs a written reason.
    Why: an unbounded queue grows until memory runs out under load. A bounded channel pushes back on the sender.

12. **Put a timeout on every await that waits for a remote party.** Use `tokio::time::timeout` or the client's own timeout setting.
    Why: a peer that stops answering holds the task forever.

## Sharing between threads

13. **`tokio::spawn` needs `Send + 'static`.** Move owned data in with `async move` and share it through `Arc`. Keep `Rc`, `RefCell` borrows and sync lock guards out of scope at each `.await`.
    Check: `clippy::future_not_send` (nursery), `clippy::arc_with_non_send_sync`.

14. **Prefer one owner task for a shared I/O resource.** Callers send messages over `mpsc` and get replies on a `oneshot`.
    Why: the owner task needs no lock, and it can process requests in order.

15. **Use scoped threads to borrow local data.** `std::thread::scope` joins every thread before it returns.
    Why: no `'static` bound and no `Arc` to share a borrow.

16. **Prefer a lock or a channel to hand-written atomics.** For a plain counter, `Relaxed` is enough. To publish data, pair `Release` on the store with `Acquire` on the load. Comment the reason for each ordering.
    Sources disagree on the default. Mara Bos (Rust Atomics and Locks) says `SeqCst` is rarely needed and can hide unclear reasoning. A common rule is to start with `SeqCst`. This skill prefers a lock first and a comment on each ordering, because an ordering that is too weak causes bugs that tests rarely catch.
