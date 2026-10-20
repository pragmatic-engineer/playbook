# ADR-0051: Bundle the anydoc Converter as the pbdoc Helper

- **Status:** Accepted
- **Date created:** 2026-10-21
- **Date modified:** 2026-10-21
- **Supersedes:** [ADR 0027](0027-anydoc-as-optional-local-tool.md), which made anydoc an optional tool that the user installs from npm and playbook finds on `PATH`.

## Context

[ADR 0027](0027-anydoc-as-optional-local-tool.md) left document conversion to a Node package that the user had to install (`npm install --global @firecrawl/anydoc`, Node 20 or later). On the maintainer's machine it was not on `PATH` on 2026-10-19, so nothing converted. The maintainer has decided that conversion ships with playbook: no Node and no extra install step. It is offline and on by default.

anydoc is also published as a Rust library (crates.io `anydoc` 0.2.4, MIT). It has no output cap, no wall clock timeout and no memory cap, and it reached about 613 MB peak on a 150 KB hostile file. So the converter must run in a child process that carries every cap.

Measured on 2026-10-19 (aarch64-apple-darwin, toolchain 1.99.1, the repo's release profile: `opt-level = "s"`, fat LTO, `codegen-units = 1`, strip). The probe was a throwaway `pbdoc` crate in a scratch worktree, then removed.

| Measure | Value |
|---|---|
| `pbdoc` stripped release binary | 7,233,008 bytes (6.9 MiB) |
| `pbdoc` gzip -9 | 3,695,855 bytes (3.5 MiB) |
| Crates in the normal and build closure | 103 on aarch64-apple-darwin and x86_64-unknown-linux-musl, 105 on Windows |
| New `[[package]]` entries in the workspace `Cargo.lock` | 72 (189 to 262, minus `pbdoc` itself) |
| Release build of `pbdoc`, cold | 1 min 45 s |
| Network crates in the closure (hyper, reqwest, ureq, rustls, openssl, native-tls, h2, http, tokio, mio, curl, socket) | none |
| Licences in the closure | all permissive. The one LGPL-2.1 crate is triple-licensed with MIT and Apache-2.0. |
| anydoc Cargo features | none. No binary target, and no OCR or HTTP code. `NeedsOcr` is only an error variant. |
| Smoke run on a CSV | correct GFM table, 4.2 MB max RSS |
| playbook v0.38.0 release binaries | 13,767,696 bytes (aarch64-apple-darwin), 15,303,456 bytes (x86_64 musl) |

## What changed from ADR 0027

- **Bundled, not optional.** `pbdoc` ships next to `playbook` on every macOS and Linux install.
- **Built from the crate, not npm.** playbook builds `pbdoc` from the pinned `anydoc` crate. No Node and no third party wrapper run on the user's machine.
- **Offline by construction.** The helper links no network code, and a test fails if that changes.
- **On by default.** `docs.convert` stays `auto`, and `auto` now acts whenever a converter is found, which is every macOS and Linux install.
- **The size baseline moved.** ADR 0027 compared against a 5.5 MB playbook binary. The v0.38.0 binary is 13.8 MB (13,767,696 bytes), and the playbook binary does not grow here.

## Decision

Build a separate `pbdoc` helper crate in this workspace and ship it next to `playbook` (approach B below). The playbook binary does not depend on anydoc.

### CLI contract

```
pbdoc [--max-input-bytes N] [--max-memory-mb N] -o <out.md> -- <input>
pbdoc --version            prints "pbdoc <playbook version>"
pbdoc --converter-version  prints "anydoc 0.2.4"
exit 0 ok | 1 conversion failed | 2 usage | 3 needs OCR | 4 resource limit
stderr: one line, no document text
```

Any other flag exits 2. No flag enables OCR, a URL, an API key or a hosted mode. A bundled `pbdoc` that exits 2 makes playbook say the helper is probably out of date and to run `playbook update`.

### Lookup order

1. `docs.anydocPath`, when set. It is an explicit override.
2. The bundled `pbdoc` next to the playbook binary.
3. The first `anydoc` on `PATH`.

### Limits

The limits are the ADR 0027 caps, now enforced inside the helper and by playbook.

- **Input size.** A file over 100 MiB (104,857,600 bytes) is refused from `stat` alone, before any byte is read, whatever `docs.maxBytes` says. playbook applies `min(docs.maxBytes, 100 MiB)` and `pbdoc` applies the same ceiling.
- **Zip containers** (docx, xlsx, pptx, odt, ods, odp, epub). The central directory is read first. The file is refused with exit 4 when it declares more than 10,000 entries, more than 256 MiB (268,435,456 bytes) of uncompressed data, or an entry of 1 MiB or more with a compression ratio above 100. anydoc's own limits stay underneath.
- **Memory,** in layers, all against `docs.maxMemoryMb` (default 1024, clamped to 4096):
  - Heap cap. A counting global allocator covers Rust heap allocations only. It does not see direct `mmap`, C `malloc` or thread stacks. Over the cap, the helper writes a short message and exits 4 without allocating.
  - Self watchdog. A thread reads the helper's own resident size every 20 ms and exits 4 when it is over the cap. This covers memory the heap cap cannot see.
  - Linux kernel limit. playbook sets `RLIMIT_AS` to the cap plus 512 MiB for the bundled child.
  - macOS: pending the probe.
  - One rayon thread, so a PDF cannot fan out stacks and parallel allocation spikes.
- **Outer watchdog.** playbook polls the child's RSS every 100 ms and kills the process group over the cap.
- **Time.** 30 seconds, then the process group is killed.
- **Output.** At most 200,000 characters kept (`docs.maxOutputChars`). `RLIMIT_FSIZE` of 64 MiB caps what the child can write.
- **Display.** Output over `docs.displayChars` (default 24,000 characters) is served in bounded, framed slices, with a section index and the first slice printed. The rest is read with `--offset` and `--limit`, or `--section`. Document text stays framed as untrusted data.

A kernel-refused allocation in Rust aborts the process. So playbook reports a bundled child that dies on SIGABRT or SIGSEGV (or SIGKILL after a limit was set) as a safety limit hit, not as a plain failure.

### How offline is enforced

- No network crate is in the helper's dependency closure. A test reads `cargo metadata` and fails on a deny list of network and OCR crates.
- The CLI has no flag that could turn on OCR, a URL or a hosted mode.
- `sandbox-exec` on macOS and `unshare --net` on Linux stay as defence in depth when available.
- The scrubbed child environment stays: proxy variables and API keys are removed.
- A second test fails if the playbook crate's closure includes anydoc, so the converter code cannot reach the playbook binary.

### Size budget

`pbdoc` must stay at or under 8 MiB (8,388,608 bytes) as a binary and 4.5 MiB (4,718,592 bytes) as a tarball. That is about 16% over the measured size. The release build fails over either number. The playbook binary does not grow.

### Supply chain

- Exact pins: `anydoc =0.2.4` and `zip =8.6.0` in the `pbdoc` manifest.
- `cargo audit` and osv-scan already run on the whole lockfile and cover the new crates.
- Dependabot gets an `anydoc` group, so an anydoc bump lands alone and its review includes the benchmark re-run.
- `NOTICE-pbdoc` lists every crate in the helper's closure with its licence. A test fails when a closure crate is missing from it, when a crate has no option on the permissive licence allowlist, or when a crate is on the network or OCR deny list. A Dependabot bump that adds a crate therefore cannot ship silently.

### What happens to the existing surfaces

- `docs.anydocPath` stays and is an override. `doc status` says when it wins over a bundled helper, and that playbook does not check an override is offline.
- `doc status` reports `converter: pbdoc (bundled)` with the helper version and the anydoc version, and warns when the helper version differs from playbook's. When nothing is found it tells the user to reinstall playbook or run `playbook update`.
- `playbook deps` stops presenting anydoc as an optional npm install and reports the bundled helper.
- The Read hint hook fires whenever a converter is found and `docs.convert` is `auto`.
- `docs.convert` stays `auto`, with `off` to turn the feature off.

## Install paths

- `install.sh` fetches, verifies (SHA256 line and attestation) and installs `pbdoc` into the same directory as `playbook`. A missing asset (an older release, or Windows) prints a warning and continues. A checksum or attestation failure is fatal.
- Homebrew installs `pbdoc` into the same `bin` as `playbook`, through a resource in the tap formula.
- `playbook update` replaces `pbdoc` after the playbook binary, checks its SHA256 line, and keeps the old helper if that step fails. A Homebrew install is left to brew.
- `playbook uninstall` removes `pbdoc`, and the `rm` guard protects it.
- Windows has no `doc` command and no helper. The feature stays off there.

**npm migration note.** Users who installed `@firecrawl/anydoc` for ADR 0027 can remove it with `npm uninstall --global @firecrawl/anydoc`. playbook does not remove it. While both exist, the bundled helper is used. `doc status` prints this note when it sees both.

## Alternatives considered

- **A. Keep ADR 0027.** Ruled out by the maintainer decision.
- **B. A separate `pbdoc` helper crate shipped next to `playbook` (chosen).** The playbook binary stays the same size. A memory blowup stays in a child process that has every cap. The helper's closure has no network crate, and a test proves it. The cost is one new asset per unix target, plus install and update paths for it, all following the `pbtop` precedent ([ADR 0038](0038-one-release-line.md) decision 9, [ADR 0041](0041-retire-playbook-telemetry.md)).
- **C. Link anydoc into playbook behind a Cargo feature that is on for release.** In-process conversion would put the 613 MB blowup inside playbook. A variant runs playbook itself as the child (`playbook __convert`). That keeps the isolation, but the child would link playbook's HTTP and TLS stack, which `playbook update` needs. Then "the converter has no network code" could no longer be checked structurally. It would also add about 6 to 7 MB to the binary that every hook event runs [unverified, not measured as a delta]. Rejected.

## Consequences

- 72 more crates in the lockfile to audit and keep current.
- About 1 min 45 s more release build time per target.
- Four more release assets, one `pbdoc-<version>-<target>.tar.gz` per unix target, so the release has 14 assets, not 10.
- The Homebrew formula in the tap needs a change before the first release that ships `pbdoc`.
- A release that ships `pbdoc` needs the install and update paths merged first, so no user ends up with a stale helper.
- The memory cap on macOS rests on the heap cap and the self watchdog because the probe found no kernel limit that macOS enforces.
- Document conversion still does not exist on Windows, and scanned PDFs still return a plain message and are not sent to a hosted service.
