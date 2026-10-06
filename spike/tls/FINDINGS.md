# Spike: ureq 3.4.2 + rustls + platform-verifier

Branch `spike/tls-platform-verifier` (from origin/main, never pushed). Scratch crate: `spike/tls/` (own `[workspace]`, not part of the main crate). Host: macOS 15 aarch64, rustc 1.97.0 (repo pin). Helper scripts for the CA and container tests are in `spike/tls/tools/`.

## Verdicts

| # | Premise | Verdict |
|---|---------|---------|
| 1 | Builds and runs on macOS aarch64 with the feature set; modest new crate count; ring (not aws-lc-rs) | CONFIRMED |
| 2a | Static musl build for x86_64 and aarch64 | CONFIRMED for cross-build from the Mac (with caveats); CI recipe expected to work, not run |
| 2b | Linux root discovery and the no-certs failure mode | CONFIRMED (code read and run in a container) |
| 3a | macOS honours a custom CA via keychain trust settings | UNVERIFIED (code read only, keychain untouched). SSL_CERT_FILE is NOT honoured on macOS (REFUTED if anyone assumes it is) |
| 3b | Linux honours a custom CA via SSL_CERT_FILE | CONFIRMED (run in a container with no /etc/ssl) |
| 4 | `Proxy::try_from_env()` honours HTTPS_PROXY and NO_PROXY | CONFIRMED |
| 5 | Release size delta | CONFIRMED: +2.17 MB (+32%) when the client is linked in; 0 when the dependency is unused |

## Feature names and how to turn it on

ureq 3.4.2 `Cargo.toml` features: `rustls = [rustls-no-provider, _ring, rustls-webpki-roots]`, `platform-verifier = ["dep:rustls-platform-verifier"]` (optional dep `rustls-platform-verifier 0.7.0` req, resolved to 0.7.1, `default-features = false`). The `rustls` feature selects the ring crypto provider (`_ring = ["rustls?/ring"]`); aws-lc-rs only appears in the private `_doc` feature.

The feature alone does nothing: the agent must opt in at runtime (`ureq-3.4.2/src/lib.rs` lines 250 to 273, `src/tls/rustls.rs` lines 186 to 192):

```rust
Agent::config_builder()
    .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
    .build()
    .into()
```

Without the feature, `RootCerts::PlatformVerifier` with rustls panics ("Rustls + PlatformVerifier requires feature: platform-verifier"). The default root store with the `rustls` feature is `RootCerts::WebPki` (bundled Mozilla roots), so the platform verifier is off unless configured. Side effect: the `rustls` feature also pulls `webpki-roots` (an unused ~data crate when PlatformVerifier is selected).

## 1. macOS build and run

```
$ cargo build --release   (spike/tls)
   cargo build (32 crates compiled)
   Finished `release` profile [optimized] target(s) in 13.74s
$ ./target/release/tls-spike
TLS outcome: OK, status 200 OK, tag_name=v0.17.0
```

`cargo tree` (aarch64-apple-darwin), abridged: ureq 3.4.2 -> rustls 0.23.45 (-> ring 0.17.14, rustls-webpki 0.103.15, rustls-pki-types, subtle, zeroize), rustls-platform-verifier 0.7.1 (-> security-framework 3.7.0, core-foundation 0.10.1, their -sys crates), ureq-proto 0.6.4 (-> http, httparse, base64), webpki-roots 1.0.9, utf8-zero, percent-encoding. No aws-lc-rs / aws-lc-sys anywhere.

New crates relative to the main crate's normal (non-dev) dependency graph, by crate name (main has 44 distinct crates on this target, apart from itself):
- macOS: 24 new: base64 bytes cfg-if core-foundation core-foundation-sys getrandom http httparse once_cell percent-encoding ring rustls rustls-pki-types rustls-platform-verifier rustls-webpki security-framework security-framework-sys subtle untrusted ureq ureq-proto utf8-zero webpki-roots zeroize.
- Linux (x86_64-unknown-linux-musl): 22 new: same minus the 4 Apple crates (core-foundation, core-foundation-sys, security-framework, security-framework-sys), plus `rustls-native-certs 0.8.4` and `openssl-probe 0.2.1`.
- `cc`, `shlex`, `find-msvc-tools`, `libc`, `log`, `itoa`, `bitflags` are already in the main graph (rusqlite `bundled` needs `cc`).

C and asm build steps: exactly one new build script, `ring`'s (no other new crate has a build.rs that compiles C). On aarch64-apple-darwin it compiled 24 objects: 13 C files (aes_nohw, curve25519, ecp_nistz, gfp_p256, gfp_p384, p256-nistz, p256, montgomery, montgomery_inv, limbs, mem, poly1305, constant_time_test) and 11 pregenerated `.S` asm files (aesv8-armx, aesv8-gcm-armv8, armv8-mont, chacha-armv8, chacha20_poly1305_armv8, ghash-neon-armv8, ghashv8-armx, p256-armv8-asm, sha256-armv8, sha512-armv8, vpaes-armv8). For x86_64-unknown-linux-musl ring produced 30 objects, for aarch64-unknown-linux-musl 24. Existing C already in the build: rusqlite's bundled `sqlite3.c`. ring needs only a plain C compiler (no nasm, no cmake, no go; contrast aws-lc-rs which would need cmake/bindgen or prebuilt artefacts).

## 2a. Static musl build from this Mac

`rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl` worked. No `musl-gcc` or musl cross toolchain exists here, and Apple's CLT clang fails (`stddef.h` not found under `-nostdlibinc`). Homebrew LLVM clang (`/opt/homebrew/opt/llvm/bin/clang`, already installed) works:

aarch64:
```
$ CC_aarch64_unknown_linux_musl=/opt/homebrew/opt/llvm/bin/clang \
  CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
  cargo build --release --target aarch64-unknown-linux-musl
   Finished `release` profile [optimized] target(s) in 29.70s
$ file .../aarch64-unknown-linux-musl/release/tls-spike
ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, not stripped     (3.8 MB, opt-level z)
```
No sysroot needed: ring's `build.rs` (lines 594 to 603) deliberately adds `-nostdlibinc` and `RING_CORE_NOSTDLIBINC` for linux-musl on every arch except x86_64, "to allow cross-compiling without a target sysroot".

x86_64: fails by default (`assert.h` then `string.h`, `stdlib.h` not found) because ring only skips libc headers for non-x86_64. With three tiny stub headers on `-isystem` (`tools/stub-headers/`: assert.h, string.h, stdlib.h, declarations only) it builds:
```
$ CC_x86_64_unknown_linux_musl=/opt/homebrew/opt/llvm/bin/clang \
  CFLAGS_x86_64_unknown_linux_musl="-isystem <stub-dir>" \
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
  cargo build --release --target x86_64-unknown-linux-musl
   Finished `release` profile [optimized] target(s) in 19.95s
ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, not stripped     (3.5 MB)
```
That is a spike hack, not a recipe. I could not run the x86_64 binary (no x86 Linux here); only the aarch64 one was run (see 2b, 3b).

`.github/workflows/release.yml` (read, not executed): both musl legs build natively on a same-arch runner with `apt-get install musl-tools`, `CC_<target>=musl-gcc`, `CARGO_TARGET_<T>_LINKER=rust-lld`, and then assert `static-pie linked|statically linked` via `file`. ring with musl-gcc on a native runner has full libc headers, so the extra stubs above are not needed there. Expected to work unchanged; the `file` static assertion matched on both of my builds. I did not run that CI recipe itself.

## 2b. Linux root discovery (source read)

On Linux (any unix that is not Android or Apple) `rustls-platform-verifier-0.7.1/src/verification/others.rs` `Verifier::new_inner` calls `rustls_native_certs::load_native_certs()` (0.8.4) once per verifier construction (so roots are re-read from disk each time a TLS config is built), adds parsable certs to a `RootCertStore`, then builds a `WebPkiServerVerifier`. No webpki-roots fallback on Linux.

`rustls-native-certs-0.8.4/src/lib.rs` `load_native_certs`:
- If `SSL_CERT_DIR` (colon-separated list) is non-empty, or `SSL_CERT_FILE` is set: load ONLY from those. They REPLACE the system store, they do not augment it. A set-but-missing path is not an error here: it yields an io/pem error entry and zero certs.
- Else `unix.rs` -> `openssl_probe::probe()`: first existing file of `/etc/ssl/certs/ca-certificates.crt` (Debian/Ubuntu), `/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem` (RHEL7+), `/etc/pki/tls/certs/ca-bundle.crt`, `/etc/ssl/ca-bundle.pem` (openSUSE), `/etc/pki/tls/cacert.pem`, `/etc/ssl/cert.pem` (Alpine), `/opt/etc/ssl/certs/ca-certificates.crt`, `/etc/ssl/certs/cacert.pem`; plus every existing dir of `/etc/ssl/certs`, `/etc/pki/tls/certs`, `/etc/security/certificates` (all files in them are parsed, deduped). (`openssl-probe`'s own SSL_CERT_FILE/DIR handling is only consulted when the existing path exists.)
- Per-file parse errors are logged (`log::warn!`) and ignored; the call only fails if the final store is empty.

No certificates at all: `rustls::Error::General("No CA certificates were loaded from the system")` raised from `Verifier::new` (others.rs lines 104 to 108), surfaced by ureq as `ureq::Error::Rustls(...)` during agent TLS setup, before any handshake. Run (aarch64 musl binary in a `FROM scratch` container, which has no /etc/ssl at all):
```
$ podman run --rm tls-spike:test
TLS outcome: ERROR
  Display: rustls: unexpected error: No CA certificates were loaded from the system
  Debug:   Rustls(General("No CA certificates were loaded from the system"))
```
Same result with `SSL_CERT_FILE=/nope.pem` (nonexistent file). Verified positive paths with a real bundle (macOS `/etc/ssl/cert.pem` copied and bind-mounted):
```
-v sysbundle.pem:/etc/ssl/certs/ca-certificates.crt:ro  ->  TLS outcome: OK, status 200 OK, tag_name=v0.17.0
-v sysbundle.pem:/etc/ssl/cert.pem:ro                   ->  TLS outcome: OK, status 200 OK, tag_name=v0.17.0
```

Error-type note for the client: a failed certificate check surfaces differently from the no-roots case. Untrusted issuer comes back as `ureq::Error::Io` wrapping `InvalidCertificate(UnknownIssuer)`, Display `io: invalid peer certificate: UnknownIssuer`. No-roots is `ureq::Error::Rustls(General(..))`. A client that wants a friendly "your system has no CA bundle" message must match both.

## 3. Custom CA

3a. macOS (code read, `verification/apple.rs`): the verifier does not load any file or env var. It builds a `SecTrust` with `SecPolicy::create_ssl(SERVER, hostname)` and calls `evaluate_with_error()`, i.e. the OS trust evaluation, which consults the system roots plus admin and user keychain trust settings. A CA the user added to the login or System keychain and marked trusted for SSL is therefore expected to be honoured; this is Apple's documented SecTrust behaviour, not something I proved, because I was told not to touch the keychain. UNVERIFIED. What I DID prove: `SSL_CERT_FILE` has no effect on macOS:
```
$ tls-spike https://localhost:8443/            # local TLS server, leaf signed by a throwaway CA
  Display: io: invalid peer certificate: UnknownIssuer
$ SSL_CERT_FILE=.../ca.pem tls-spike https://localhost:8443/
  Display: io: invalid peer certificate: UnknownIssuer     # unchanged: env var ignored on macOS
```
`new_with_extra_roots` exists in the crate but ureq 3.4.2 does not expose it (it only calls `Verifier::new(provider)`), so there is no in-process way to add a CA with PlatformVerifier. Adding one means either the keychain (macOS) or `SSL_CERT_FILE`/`SSL_CERT_DIR` (Linux) or `RootCerts::Specific` (a different, non-platform mode).

3b. Linux, run in a `FROM scratch` container (aarch64 musl binary, no /etc/ssl at all), podman machine (Docker not installed, podman was available), local HTTPS server on the Mac with a throwaway CA, `host.containers.internal` in the leaf SAN:
```
-e SSL_CERT_FILE=/certs/bundle.pem  https://host.containers.internal:8443/  ->  TLS outcome: OK, status 200 OK, tag_name=v9.9.9-local
-e SSL_CERT_DIR=/certsdir           https://host.containers.internal:8443/  ->  TLS outcome: OK, status 200 OK, tag_name=v9.9.9-local
-e SSL_CERT_FILE=/certs/bundle.pem  (api.github.com)                        ->  io: invalid peer certificate: UnknownIssuer
```
Last line plus this one prove SSL_CERT_FILE REPLACES the system store (real Debian-path bundle mounted AND `SSL_CERT_FILE=<custom CA only>` -> github fails with UnknownIssuer). So a corporate user must point SSL_CERT_FILE at a bundle that contains the public roots too (or use a dir with both). Worth documenting.

## 4. Proxy

`ureq-3.4.2/src/config.rs:948`: the default `Config` sets `proxy: Proxy::try_from_env()`, so a plain `Agent::config_builder()` honours the env with no extra code. `proxy.rs:222`: tries `ALL_PROXY`, `all_proxy`, `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy` in that order (first that parses wins, uppercase before lowercase; scheme of the target is not consulted, so `HTTP_PROXY` also applies to https URLs, unlike curl). `NO_PROXY` (exact host, `*.x`, `.x`, `*`) is read automatically; lowercase `no_proxy` is also read (`proxy.rs:542`, not separately run). socks proxies need the `socks-proxy` feature, not enabled here; an `http://` proxy uses CONNECT. Run with a local logging CONNECT proxy on 127.0.0.1:8080 that answers 502:
```
HTTPS_PROXY=http://127.0.0.1:8080
  Proxy::try_from_env() = Some(Proxy { proto: Http, uri: http://127.0.0.1:8080/******, from_env: true })
  TLS outcome: ERROR  Display: CONNECT proxy failed: proxy server responded 502/502
  proxy.log: PROXY SAW CONNECT api.github.com:443
https_proxy=http://127.0.0.1:8080          -> same (proxy saw CONNECT api.github.com:443)
HTTPS_PROXY=... NO_PROXY=api.github.com    -> TLS outcome: OK, status 200 OK, tag_name=v0.17.0 (bypassed)
HTTPS_PROXY=... NO_PROXY=.github.com       -> TLS outcome: OK (bypassed)
```
Credentials in the proxy URL are masked (`******`) in the Debug output. I did not test a proxy that actually tunnels (so the platform verifier on the tunnelled handshake is not exercised); the verifier is independent of how the socket was obtained, so I expect it to behave the same.

## 5. Release binary size (main crate, `cargo build --release`, default profile, macOS aarch64, not stripped)

```
baseline (origin/main Cargo.toml)                                       6,918,336 bytes (6.6 MiB)
+ ureq {rustls, platform-verifier} added to [dependencies], UNUSED       6.6 MiB  (unchanged: an unreferenced crate is not linked; 25 extra crates compiled, 15.9 s)
+ same dependency AND a call that builds the PlatformVerifier agent
  and GETs a URL linked into main()                                      9,139,312 bytes (8.7 MiB)   delta +2,220,976 bytes (+32%)
```
The "linked" figure is the meaningful one. The playbook crate has no `[profile.release]` customisation; the spike crate uses `opt-level = "z"` and is not comparable (2.9 MB). With LTO/`strip`/`opt-level = "s"` the delta would shrink but I did not measure it. I made the Cargo.toml and `src/main.rs` edits only in the worktree to measure and reverted both with `git checkout` (no change committed). Sanity check: the instrumented main binary printed `Ok(200)` for the GitHub URL.

## Cleanup and environment notes

- Podman machine was stopped (it was `Never` up before I started it), the temporary image removed, the two python servers killed (`pgrep` shows none). No keychain, global git or cargo config changes.
- `rustup target add` installed `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` into the pinned 1.97.0 toolchain (still installed).
- `cargo` fetched `rustls-platform-verifier 0.7.1`, `rustls-native-certs 0.8.4`, `openssl-probe 0.2.1`, `security-framework 3.7.0` and others into `~/.cargo/registry`.
- The `git` binary is invoked as `/usr/bin/git` here because the environment's command wrapper refused bare `git` calls from this worktree.
