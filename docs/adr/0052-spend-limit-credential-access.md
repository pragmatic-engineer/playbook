# ADR-0052: Read the Claude Code Login Token to Fetch the Server Spend Limit

- **Status:** Accepted
- **Date created:** 2026-10-20
- **Date modified:** 2026-10-20
- **Builds on:** [ADR 0018](0018-tls-platform-verifier-not-adopted.md), which kept the binary small and recorded that the only network calls were to GitHub. [ADR 0020](0020-usage-terminal-ui.md), which set the template for reporting a binary size change.

## Context

`playbook usage` shows a cost it estimates from session transcripts. It cannot show how much the server billed against the monthly spend limit an organization admin set. The numbers differ whenever another machine, another seat or an unpriced model is involved.

Two token free sources already exist and ship first, in the same plan:

- Claude Code caches `cachedUsageUtilization` in `~/.claude.json`, and playbook reads it.
- The statusline input carries a `spend_limit`, but only behind a gateway.

Neither is enough for every account. The cache only changes while Claude Code runs, and its `spend` shape was never seen on an enterprise machine.

The maintainer probed `https://api.anthropic.com/api/oauth/usage` by hand, with the header `anthropic-beta: oauth-2025-04-20` and the Claude Code OAuth login token, on one Max 20x account. The reply held `extra_usage` (`monthly_limit`, `used_credits`, `utilization`, `decimal_places`), `spend` with a percent and a severity, `spend_limit_reached`, `five_hour` and `seven_day` (both null on that plan) and `member_dashboard_available`. Anthropic does not document this endpoint or these fields. Everything below about the reply comes from that one probe.

Reading the token means reading a credential that belongs to another application. That is hard to take back in terms of trust, and a later maintainer will not guess the reasons, so it is recorded here. The maintainer approved the live fetch and accepted the terms of service risk on 2026-10-20.

## Decision

1. **One call, one host.** When the feature is on, playbook sends the Claude Code OAuth access token in one `GET https://api.anthropic.com/api/oauth/usage` with `anthropic-beta: oauth-2025-04-20`. The host and path are constants. No environment variable and no config key changes them. The token goes nowhere else.

2. **Off by default, two steps to turn on.** The key `usage.spend.enabled` (default `false`) lives in the global tier, like `update.mode`, so a repo or org file cannot turn it on. Setting the key is not enough: `playbook usage spend enable` must also store an acknowledgement in the state table, after it prints a notice and gets `--yes` or an interactive `y`. A key set by `config import` therefore reads nothing. The acknowledgement names the notice version and the token source, so a changed notice or source asks again. `playbook usage spend disable` clears the key, the acknowledgement and the cache file. `playbook usage spend status` prints the state and never reads the token. There is no setup part and no migration.

3. **Where the token is read from.** On macOS, `/usr/bin/security find-generic-password -s "Claude Code-credentials" -w`, by absolute path, with the environment cleared, no shell, stdin closed, stdout read up to 8 KiB, stderr discarded, and a 3 second limit before the process is killed. On Linux and as the fallback, `$CLAUDE_CONFIG_DIR/.credentials.json` or `~/.claude/.credentials.json`, refused when it is a symlink. Only the access token and its expiry are read. An expired token is not sent. Windows is out of scope. The keychain item name and the JSON key names are not in Anthropic's public documentation, so they are unverified until the maintainer checks them without printing the secret. An API key, Bedrock, Vertex or Foundry login has no OAuth token, so the read is skipped before any credential is touched. `usage.plan` set to `api` or `cloud` skips it too.

4. **How the token is handled.** It is held in a `Secret` type with no `Debug`, `Display`, `Clone` or `Serialize`. It is overwritten with zero bytes on drop with `std::ptr::write_volatile`, with no new crate. It is never printed, logged, written to disk (the cache and every error text included), placed in an argument list, or put in the environment of a child.

5. **How the request is made.** The `ureq` agent already in the binary, with `https_only(true)`, no redirects, no proxy, a total timeout of 5 seconds, `Accept-Encoding: identity`, and a body cap of 64 KiB on the decoded body. Errors map to a reason class and the `ureq` error text is never shown, since it can contain the URL. No new dependency is added.

6. **Fail soft.** No token, 401, 403, 429, 5xx, a timeout, a DNS or TLS error, a changed schema and a non-OAuth login all become one line, `spend limit unavailable (<reason>)`, and never a panic, a non-zero exit or a hook failure.

7. **Cache and rate.** The parsed numbers and `fetched_at` are cached for `usage.spend.cacheSeconds` (default 60, from 30 to 600) in `<data>/usage-spend.json` through `write_atomic_private`. A failure caches a short negative entry, 5 minutes, or 15 for 401 and 403, so a bad token cannot cause a retry storm. There is one request per refresh window and no retry inside a call.

8. **Where it may run.** Only the usage worker thread and the one-shot `--summary` and `--json`, through `Host::spend_fetcher()`, which returns nothing by default. Hooks, the statusline, `warm`, `collect`, session start and `pbtop` never call it, and `pbtop` keeps its size.

## Alternatives considered

- **Token free sources only.** Kept as the base: they ship first and stay useful if this record is ever withdrawn. Rejected as the whole answer because the cache may not hold the spend limit for every account.
- **Live fetch only, before anything else.** Rejected. It makes `playbook usage` depend on the network and adds a credential path before anyone knows the cache is not enough.
- **The `security-framework` or `keyring` crates.** Rejected. They add dependencies and size against [ADR 0018](0018-tls-platform-verifier-not-adopted.md). The `security` program costs a possible keychain prompt and no new code.
- **`CLAUDE_CODE_OAUTH_TOKEN`.** Rejected. It is documented as an inference only token from `claude setup-token`, so it probably cannot read usage. Unverified.
- **Scraping the `/usage` command.** Rejected. It is interactive only and has no machine readable form.

## Consequences

These are the risks accepted on 2026-10-20.

- **Terms of service.** Anthropic's legal page (code.claude.com/docs/en/legal-and-compliance, "Authentication and credential use") restricts collecting or intermediating Claude.ai credentials. It offers no exception for a local tool that reads its own user's token. Using the token this way may not be permitted, and an account could be restricted or closed. The feature ships in a distributed binary, so the risk falls on every user who turns it on, not only the maintainer. It is off by default, turning it on takes an explicit command, and the notice and the usage guide say this in plain words. The user decides. This reading of the page was taken from the plan and was not checked again for this record.
- **An undocumented endpoint.** It can change or stop without notice. Parsing is tolerant, a bad reply becomes `schema`, and the token free sources keep working.
- **The secret in memory.** The token exists in process memory and in a pipe buffer for the length of one call and is then overwritten. It is not protected against a core dump or swap, and no `setrlimit` is added so as not to add a dependency.
- **A keychain prompt.** The first read by `/usr/bin/security` may show a macOS prompt. Choosing "Always Allow" may let every script that runs `security` read that item without asking. Unverified.
- **No proxy.** Environment proxies are not used for this call, since a proxy that inspects TLS could read a bearer token. Behind a corporate proxy the line reads `spend limit unavailable (network)`, and the guide says why.
- **A second network host.** [ADR 0018](0018-tls-platform-verifier-not-adopted.md) said the only network calls were to GitHub. That stays true unless the user runs `playbook usage spend enable`.
- **Binary size.** The release binary size before and after is recorded in the pull request, in bytes, as [ADR 0020](0020-usage-terminal-ui.md) did.
- **Not verified:** the shape of the reply on any plan except Max 20x; whether `used_credits` counts one member or the whole organization; whether the server billing month follows the UTC calendar month; the keychain item name and the JSON key names; Windows.
