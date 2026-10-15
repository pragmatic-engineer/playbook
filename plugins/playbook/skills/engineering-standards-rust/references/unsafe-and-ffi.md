# Unsafe and FFI

Keep `unsafe` rare, small, documented and tested. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Unsafe code: rules 1 to 7
- FFI: rules 8 to 12

## Unsafe code

1. **Write `unsafe` only when safe Rust cannot express the code.** FFI, a low-level data structure, or a hot path that a profile proved. "It is faster" without a measurement is not a reason.
   Why: the compiler cannot check unsafe code, and each block asks every reader for trust.
   Check: `#![forbid(unsafe_code)]` in a crate that needs none (rustc `unsafe_code` lint).

2. **Wrap unsafe code in a safe API.** No input to the safe API may cause undefined behavior. Keep the code that upholds the invariant in its own module with private fields.
   Why: safety depends on every line that can touch those fields, and privacy is what limits that set.

3. **Each `unsafe` block has a `// SAFETY:` comment** that says why each precondition holds. Keep blocks small, with one unsafe operation each where you can.

   ```rust
   // SAFETY: `idx < self.len` was checked above, and `self.ptr`
   // points to `self.len` initialized elements.
   let item = unsafe { &*self.ptr.add(idx) };
   ```

   Check: `clippy::undocumented_unsafe_blocks` (restriction), `clippy::multiple_unsafe_ops_per_block` (restriction).
   Sources differ here. Canonical's guide asks for a `// SAFETY:` comment on an `unsafe fn` as well. This skill follows the Rust API Guidelines and Clippy: an `unsafe fn` gets a `# Safety` doc section (rule 4), because callers read it in rustdoc.

4. **Each `pub unsafe fn` has a `# Safety` doc section** that lists what the caller must uphold.
   Check: `clippy::missing_safety_doc`.

5. **Use explicit `unsafe {}` blocks inside an `unsafe fn`.**
   Why: "unsafe to call" and "does unsafe things here" are two separate facts. The 2024 edition warns on the implicit form.
   Check: rustc `unsafe_op_in_unsafe_fn` (allow by default before the 2024 edition, warn from 2024).

6. **A safe function does not dereference a raw pointer it receives.** Make the function `unsafe` or take a reference.
   Check: `clippy::not_unsafe_ptr_arg_deref`.

7. **Run tests under Miri for any crate with unsafe code.** `cargo +nightly miri test`. Mark tests Miri cannot run with `#[cfg_attr(miri, ignore)]`.
   Why: Miri finds out-of-bounds access, use after free and aliasing errors on the paths your tests run. A clean run proves nothing about other paths, and Miri cannot run most FFI calls.

## FFI

8. **Use `#[repr(C)]` on every type that crosses the boundary**, and `CString` and `CStr` for C strings.
   Why: the Rust layout is not specified, and Rust strings have no trailing NUL.

9. **Generate bindings with `bindgen`, `cbindgen` or `cxx`.**
   Why: the compiler does not check hand-written `extern` signatures against the C header, and a wrong signature is undefined behavior.

10. **Do not let a panic cross an `extern "C"` function.** Catch it with `catch_unwind` and return an error code, or declare `extern "C-unwind"` when unwinding across is intended.
    Why: a panic that reaches an `extern "C"` boundary aborts the process.

11. **Make ownership across the boundary explicit.** The side that allocates also frees, with the same allocator. Wrap a foreign resource in a Rust type whose `Drop` releases it.
    Why: this prevents leaks and double frees, including on early return.

12. **Model nullable pointers and callbacks as `Option`.** `Option<&T>`, `Option<Box<T>>` and `Option<extern "C" fn(...)>` have the same size as a pointer, with `None` as null.
