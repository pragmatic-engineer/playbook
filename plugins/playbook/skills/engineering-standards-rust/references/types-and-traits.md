# Types and traits

Use the type system to make wrong states fail to compile, and pick the right traits and dispatch. A lint marked (pedantic), (nursery) or (restriction) is off by default.

## Contents

- Make invalid states unrepresentable: rules 1 to 6
- Construction and conversion: rules 7 to 10
- Common traits: rules 11 to 13
- Generics and trait objects: rules 14 to 18

## Make invalid states unrepresentable

1. **Wrap a primitive that has a meaning in a newtype.** IDs and units get their own type.

   ```rust
   // wrong: easy to swap the two arguments
   fn refund(order: u64, user: u64) { /* ... */ }
   // right
   pub struct OrderId(u64);
   pub struct UserId(u64);
   fn refund(order: OrderId, user: UserId) { /* ... */ }
   ```

   Why: the compiler rejects a mix-up, at no run time cost.

2. **Parse, then trust.** Validate once in the constructor (`TryFrom`, `FromStr` or a `parse` function) and keep the field private. Code that receives the type does not check it again.

   ```rust
   pub struct Email(String);

   impl std::str::FromStr for Email {
       type Err = EmailError;
       fn from_str(s: &str) -> Result<Self, Self::Err> {
           if s.contains('@') { Ok(Self(s.to_owned())) } else { Err(EmailError) }
       }
   }
   ```

   Why: every value of the type is valid, and checks do not repeat across the code.

3. **Use an enum in place of a `bool` parameter or field.**

   ```rust
   // wrong: what does `true` mean at the call site?
   render(page, true);
   // right
   render(page, Layout::Compact);
   ```

   Why: the call site reads clearly, and a third option is a compiler-guided change.
   Check: `clippy::fn_params_excessive_bools` (pedantic) and `clippy::struct_excessive_bools` (pedantic) fire only above 3 bools by default. Set `max-fn-params-bools` and `max-struct-bools` in `clippy.toml` to lower the limit.

4. **Use an enum for a closed set of values.** Parse outside text into it with `FromStr` at the boundary.
   Why: an invalid value cannot reach the code that uses it.

5. **Use typestate when calling in the wrong order must not compile.** Each state is its own type, and a transition takes `self` and returns the next state. When the state is known only at run time, use one enum.
   Why: a wrong-order call becomes a compile error.

6. **Match your own enums exhaustively.** Leave out the `_` arm. In the same spirit, write every field in a struct literal where each field matters, and keep `..Default::default()` for types whose defaults are real values, such as options.
   Why: a new variant or field makes the compiler list every place that must handle it.
   Check: `clippy::wildcard_enum_match_arm` (restriction), `clippy::match_wildcard_for_single_variants` (pedantic).

## Construction and conversion

7. **`new` is the main constructor, `with_*` and `from_*` are the others.** If a no-argument `new` makes sense, also implement `Default`. Do not implement `Default` when no real default exists (an email, an API key).
   Why: `Default` works with `mem::take`, `unwrap_or_default` and `..Default::default()`. A fake default becomes valid data.
   Check: `clippy::new_without_default`.

8. **Use a builder for many optional inputs.** Either a consuming builder (`self`) or a borrowing one (`&mut self`) is fine. Use `&mut self` when callers set options inside `if` blocks.
   Sources differ here. Canonical's guide makes consuming builders the default with a fallible `build()`. The Rust API Guidelines describe both styles. Pick one style per crate.

9. **Implement `From` and `TryFrom`. Do not implement `Into` or `TryInto` directly.** A conversion that can fail or panic is `TryFrom`.
   Why: the standard library gives you `Into` from `From`, and `From` promises it cannot fail.
   Check: `clippy::from_over_into`, and `clippy::fallible_impl_from` (nursery).

10. **Avoid `as` for number conversions that can lose data.** Use `From` to widen and `TryFrom` to narrow.

    ```rust
    // wrong: wraps without warning when len > u32::MAX
    let n = len as u32;
    // right
    let n = u32::try_from(len).map_err(|source| FrameError::TooLong { len, source })?;
    ```

    Why: `as` does not fail. It gives a wrong number in silence.
    Floats have no `From` or `TryFrom` to integers. Check the range and that the value is finite before you cast.
    Check: `clippy::cast_possible_truncation` (pedantic), `clippy::cast_sign_loss` (pedantic), `clippy::cast_possible_wrap` (pedantic), `clippy::cast_precision_loss` (pedantic), `clippy::cast_lossless` (pedantic).

## Common traits

11. **Public types implement the common traits that apply.** `Debug` on every public type, plus `Clone`, `PartialEq` and `Eq`, `Hash`, `Default` and `Display` where they make sense. `Copy` fits small plain values. Derive `PartialOrd` and `Ord` only when the order has a meaning, because the derived order follows field order.
    Why: other crates cannot add these impls later (the orphan rule).
    Check: rustc `missing_debug_implementations` (allow by default), `clippy::derive_partial_eq_without_eq` (nursery).
    No lint checks derive order. Canonical's guide puts `Copy` first, then the rest in alphabetical order. Pick one order per project.

12. **Hide secrets in `Debug`.** Write `Debug` by hand, or wrap the value in a type such as `secrecy::SecretString`.
    Why: `{:?}` ends up in logs and panic messages.

13. **Get `to_string` from `Display`.** Do not write an inherent `to_string` method.
    Check: `clippy::inherent_to_string`.

## Generics and trait objects

14. **Static dispatch by default, `dyn Trait` when it pays.** Use a trait object for mixed types in one collection, for a plugin boundary, or to cut compile time and binary size.
    Why: generics allow inlining. A trait object compiles once and costs a vtable call.

15. **Make a function generic when a second type uses it now.** At a public entry point a loose bound helps the caller (`impl AsRef<Path>`, `impl IntoIterator<Item = T>`). Forward to a private non-generic function to keep the generated code small.
    Sources disagree here. The Rust API Guidelines favour flexible generic parameters. corrode.dev warns that generics cost readability and compile time. This skill takes the narrower rule, because unused flexibility still costs every reader.

16. **Put trait bounds on `impl` blocks, not on struct definitions.**
    Why: a bound on the struct spreads to every use and makes later changes breaking.

17. **Seal a trait that outside crates should use but not implement.** Make it require a `pub trait Sealed` declared inside a private module: `mod private { pub trait Sealed {} }`.
    Why: a non-`pub` supertrait trips the `private_bounds` lint.
    You can then add methods later without a breaking change.

18. **Implement `Deref` only for smart pointer types.** Do not use it to inherit methods from an inner type. Expose `as_inner()` or `AsRef` instead.
    Why: `Deref` changes method lookup in ways a reader does not expect.
