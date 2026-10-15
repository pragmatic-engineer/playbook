# Async and concurrency

Picking a model, asyncio, threads and processes. A Ruff rule named here needs its group or code in `lint.select`. A rule marked (preview) also needs `preview = true`.

## Contents

- Picking a model: rules 1 to 3
- asyncio: rules 4 to 11
- Threads and processes: rules 12 to 15

## Picking a model

1. **Write the code sequentially first, and add concurrency for a measured bottleneck.**
   Why: concurrency adds races and timing bugs. Pay for it only where the profile shows waiting or CPU work that can run in parallel.

2. **Match the model to the work.**
   - Many network or disk waits, with async libraries: `asyncio`.
   - Waits on blocking libraries: a `ThreadPoolExecutor`.
   - CPU-bound pure Python: a `ProcessPoolExecutor`, or `InterpreterPoolExecutor` (3.14+).

   Why: on the default CPython build the GIL lets one thread run Python code at a time. Threads overlap waits, and only processes or subinterpreters run Python code in parallel.

3. **Treat the free-threaded build (3.13+, supported from 3.14) as opt-in.** Check that every C extension supports it, and protect shared state with explicit locks. Do not rely on the internal locks of `dict` and `list`.

## asyncio

4. **Start the program with `asyncio.run(main())`,** once, at the entry point.

5. **Do not block the event loop.** No `time.sleep`, no `requests`, no sync file or database calls, and no long CPU work inside `async def`. Use the async library, or move the call off the loop with `asyncio.to_thread`.

   ```python
   import asyncio
   import time

   def read_sensor() -> int:
       time.sleep(0.01)  # stands in for a blocking driver call
       return 42

   async def main() -> int:
       # wrong: read_sensor() here stalls every other task
       return await asyncio.to_thread(read_sensor)

   assert asyncio.run(main()) == 42
   ```

   Why: one blocking call stops every task on the loop for its whole duration.
   Check: the `ASYNC` rules, for example `ASYNC251` for `time.sleep`, `ASYNC210` for a blocking HTTP call and `ASYNC230` for `open()` in an async function. Run tests with `PYTHONASYNCIODEBUG=1` to log slow callbacks.

6. **Run related tasks in an `asyncio.TaskGroup` (3.11+).** It waits for every task, and on the first failure it cancels the rest and raises an `ExceptionGroup`.

   ```python
   import asyncio

   async def fetch(n: int) -> int:
       await asyncio.sleep(0)
       return n * 2

   async def main() -> list[int]:
       async with asyncio.TaskGroup() as tg:
           tasks = [tg.create_task(fetch(n)) for n in range(3)]
       return [t.result() for t in tasks]

   assert asyncio.run(main()) == [0, 2, 4]
   ```

   The asyncio docs describe both `gather` and `TaskGroup`. Real Python and this skill pick `TaskGroup`, because `gather` leaves the other tasks running after one fails. Use `gather(..., return_exceptions=True)` only when you want every result, failed or not.

7. **Keep a reference to every task you create.** A bare `asyncio.create_task(...)` can be garbage collected before it finishes. Use a `TaskGroup`, or store the task and remove it in a done callback.
   Check: `RUF006`.

8. **Put a timeout on every network call and wait.** Use `async with asyncio.timeout(seconds):` (3.11+), and catch `TimeoutError` outside the block.
   Why: without a timeout, a peer that stops answering holds the task until the process ends.

9. **Let cancellation through.** Clean up in `finally` or `async with`. If you catch `asyncio.CancelledError`, re-raise it.
   Why: `TaskGroup` and `asyncio.timeout` work by cancelling tasks, and a swallowed cancellation breaks both.
   Note: `CancelledError` inherits from `BaseException`, so `except Exception` does not catch it.

10. **Bound the work in flight.** Use an `asyncio.Semaphore` or a bounded `asyncio.Queue(maxsize=...)` when you fan out over a large input.
    Why: ten thousand tasks open ten thousand connections at once.

11. **Share one client per program,** such as one `httpx.AsyncClient` or `aiohttp.ClientSession`, opened with `async with` at startup. Do not open one per request.
    Why: the client holds the connection pool.

## Threads and processes

12. **Use `concurrent.futures` executors in a `with` block.** Do not manage `threading.Thread` objects by hand for a pool of jobs.
    Why: the block waits for the workers and shuts the pool down when an exception is raised.

13. **Share as little as you can between threads.** Pass work through `queue.Queue` and return results through futures. Guard state you must share with a `threading.Lock` in a `with` block.

14. **Do not wait on a future from inside the same pool.** A task that waits for another task in a full pool deadlocks.

15. **Give a process pool picklable work and an importable entry point.** Use module-level functions, no lambdas, and the `if __name__ == "__main__":` guard. Since Python 3.14 no platform uses `fork` as the default start method, so code that relied on inherited globals breaks.
