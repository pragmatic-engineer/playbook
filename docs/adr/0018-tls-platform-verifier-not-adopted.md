# ADR-0018: Keep the Bundled Root Certificates, Do Not Adopt the TLS Platform Verifier Yet

- **Status:** Accepted
- **Date created:** 2026-10-09
- **Date modified:** 2026-10-09

## Context

Issue #552 asked what to do with a spike that tried the `platform-verifier` feature of `ureq` (with `rustls` and `ring`). The platform verifier checks server certificates against the operating system trust store, so a corporate CA installed on the machine works without extra setup. Today `playbook update` and the installer talk to GitHub with the bundled Mozilla roots (`RootCerts::WebPki`), which ignore the system store.

The spike is kept at the tag `archive/tls-platform-verifier` (commit `fee60d9`). Its notes are in `spike/tls/FINDINGS.md` at that tag. The numbers below come from there.

## Findings

- **Build:** it builds and runs on macOS aarch64 and cross-builds to static `x86_64` and `aarch64` musl targets. It uses `ring`, not `aws-lc-rs`, so it needs only a plain C compiler. It adds 22 to 24 crates and one new build script (`ring`).
- **Linux custom CA:** `SSL_CERT_FILE` and `SSL_CERT_DIR` work. `SSL_CERT_FILE` replaces the system store. A bundle with only the corporate CA made `api.github.com` fail with `UnknownIssuer`. So a bundle must hold the public roots too.
- **macOS custom CA:** `SSL_CERT_FILE` is ignored. A CA must be trusted in the keychain. This was read in the code and not run.
- **Proxy:** `Proxy::try_from_env()` is the default in `ureq`, so `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` already work today with no code change. `HTTP_PROXY` also applies to https URLs, unlike curl.
- **Size:** linking the client with the platform verifier added about 2.2 MB (+32%) to a binary of 6.9 MB at the time.

## Decision

Do not adopt the platform verifier now.

1. The release binary was just cut from 8.2 MB to 5.5 MB (#641 and #648). A change worth +2 MB, with the same percentage cost, would undo most of that.
2. The need is unproven. No user has reported a custom CA problem. The only network calls are to GitHub for `update`, `release` and the installer.
3. The macOS keychain behaviour is unverified, so the main benefit is not yet shown.

Revisit when a user reports a custom CA or inspecting proxy problem. At that point, re-measure the size with the current release profile first, and test the macOS keychain path on a real machine.

## What works today, and what would change

The current build (`src/update/download.rs`) uses the default `ureq` agent with the bundled roots and reads the proxy from the environment. So:

- **Proxy:** `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` work today. A proxy that only tunnels (CONNECT) works with no change.
- **Custom CA or TLS interception:** it does not work today. `SSL_CERT_FILE` and the macOS keychain are not read, because the bundled roots are used. The supported path is to download the binary from the GitHub release in a browser and put it on `PATH`.
- **If the platform verifier is adopted later:** on Linux, `SSL_CERT_FILE` must point to a bundle that holds the public roots and the corporate CA, because the variable replaces the system store. On macOS the CA must be trusted in the keychain, and `SSL_CERT_FILE` is ignored.

## Alternatives considered

- **Adopt the platform verifier now.** Rejected for the size cost and the unproven need.
- **`RootCerts::Specific` with a user supplied bundle path.** Not built. It would add a config key and a documentation burden for a problem nobody has reported.
- **Switch the HTTP client.** Out of scope, and it would not reduce the size.

## Consequences

- The binary stays at its current size and build time.
- A user behind TLS interception cannot self-update with the built-in command. The manual download is the supported path until this is revisited.
- The two Linux and macOS custom CA rules above are recorded here so the follow-up starts from them.
- The spike branch is deleted. Its commit stays reachable through the tag `archive/tls-platform-verifier`.
