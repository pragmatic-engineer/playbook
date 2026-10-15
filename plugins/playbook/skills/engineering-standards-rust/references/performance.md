# Performance

Measure first, then cut allocations and copies on the hot path. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Measure: rules 1 to 5
- Allocations and copies: rules 6 to 11
- Build settings: rules 12 to 14

## Measure

1. **Profile before you optimize.** Change code for speed only where a profile or a benchmark shows the cost. A comment next to the change names the measurement.
   Why: optimized code is harder to read, and most code is not on the hot path. Without the comment, a later cleanup undoes the change.

2. **Benchmark and profile release builds.** A debug build can be 10 to 100 times slower and points at the wrong code.
   Check: `cargo bench`, or `cargo build --release` before you profile.

3. **Write benchmarks with `criterion` or `divan`** on stable Rust, with `harness = false` in the `[[bench]]` table. Wrap inputs in `std::hint::black_box`.
   Why: the built-in `#[bench]` needs nightly, and without `black_box` the compiler can remove the work you meant to measure.

4. **Profile with a sampling profiler.** `perf`, `samply`, `cargo flamegraph` or Instruments for CPU time, and DHAT for allocations. Set `debug = "line-tables-only"` in the release profile (or a separate profiling profile) to get readable stacks.

5. **Change the algorithm or the data structure first.** Micro-tuning comes after.
   Why: the large wins usually come from doing less work.

## Allocations and copies

6. **Size collections up front when you know the size.** Use `Vec::with_capacity`, `String::with_capacity` or `reserve`. When the size comes from input, clamp it to a fixed maximum first (security-and-input, rule 3).
   Check: review. `clippy::vec_init_then_push` catches only a fixed run of pushes after `Vec::new()`.

7. **Reuse a buffer in a loop.** `clear()` keeps the capacity. Read lines into one `String` with `read_line` on a hot path.
   Why: `lines()` and a new `Vec` per pass allocate on every iteration.

8. **Avoid clones and `to_string` on hot paths.** Borrow, or use `Cow` when you change the value only sometimes.
   Check: `clippy::unnecessary_to_owned`, `clippy::redundant_clone` (nursery).

9. **Write into an existing `String` with `write!`.** Avoid `format!` in a loop and `push_str(&format!(...))`.
   Check: `clippy::format_push_string` (pedantic), `clippy::format_collect` (pedantic).

10. **Keep hot types small.** Box a large, rare enum variant. Do not box a collection (`Box<Vec<T>>`).
    Why: an enum is as big as its largest variant, and big values cost a copy on each move.
    Check: `clippy::large_enum_variant`, `clippy::box_collection`, `clippy::large_stack_arrays` (pedantic).

11. **Buffer I/O.** Wrap files and sockets in `BufReader` or `BufWriter`, and call `flush()` and handle its error. For many lines to `stdout`, write to `BufWriter::new(io::stdout().lock())`.
    Why: Rust file I/O is not buffered, `stdout` flushes on each newline, `println!` takes the lock on each call, and an error on the implicit flush at drop is lost.

## Build settings

12. **Tune the release profile one setting at a time, with a benchmark.** `codegen-units = 1` and `lto = "fat"` or `"thin"` often help speed and cost build time. `panic = "abort"` makes the binary smaller and skips unwinding, and code that relies on `catch_unwind` or `Drop` during a panic stops working.
    Why: each setting trades build time or behavior for speed, and the gain differs per program.

13. **Use `-C target-cpu=native` only for binaries that run on the build machine's CPU type.**
    Why: the binary can use instructions that other CPUs lack, and it crashes there.

14. **Switch to a faster hasher only when the profile shows hashing and the keys do not come from users.** Choose it per map. A crate-wide ban of `std::collections::HashMap` with `clippy::disallowed_types` fits only a crate that hashes no user-controlled keys.
    Why: the default `HashMap` hasher resists hash flooding attacks, and faster hashers do not.
