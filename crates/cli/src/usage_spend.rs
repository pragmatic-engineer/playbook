// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The server spend limit: reading the Claude Code login token and asking
//! Anthropic for the limit and the amount billed (ADR 0052).
//!
//! The token is held in a [`Secret`] that cannot be printed, cloned or
//! serialized and is overwritten when dropped. It goes to one host over TLS,
//! with no redirect and no proxy. It is never logged, written to disk, put in
//! an argument list or put in the environment of a child. Every failure becomes
//! a reason class, and the text of an HTTP error, which can hold the URL, is
//! never shown.

use crate::usage::command::SpendAction;
use crate::usage::spend::{self, Parsed, Reason};
use crate::usage::spend_fetch::{self, Outcome, SpendFetcher};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The only address the token is sent to.
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

/// The beta flag the endpoint asks for.
const BETA: &str = "oauth-2025-04-20";

/// The keychain item Claude Code keeps its login in.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// The program that reads the keychain, by absolute path.
const SECURITY: &str = "/usr/bin/security";

/// The file Claude Code keeps its login in, under its config folder.
const CREDENTIALS_FILE: &str = ".credentials.json";

/// The environment variable that moves Claude Code's config folder.
const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// The most the keychain program may print before the read is refused.
const KEYCHAIN_MAX_BYTES: u64 = 8 * 1024;

/// The most a credentials file may hold before the read is refused.
const FILE_MAX_BYTES: u64 = 64 * 1024;

/// The time the keychain program gets before it is killed.
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// The pause between checks on the keychain program.
const POLL: Duration = Duration::from_millis(10);

/// The time the whole request gets.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The most of a reply that is read.
const BODY_LIMIT: u64 = 64 * 1024;

/// An expiry at or above this is in milliseconds, below it in seconds.
const MILLIS_FROM: i64 = 100_000_000_000;

const MILLIS_PER_SECOND: i64 = 1000;

/// The state key of the acknowledgement.
const ACK_KEY: &str = "usage/spend-ack";

/// Bumped when the notice changes in a way that needs a new yes.
const NOTICE_VERSION: &str = "1";

/// Where the token is read from first on this machine.
#[cfg(target_os = "macos")]
const SOURCE_NAME: &str = "keychain";
#[cfg(not(target_os = "macos"))]
const SOURCE_NAME: &str = "file";

/// What the user is told before the feature turns on.
const NOTICE: &str = "playbook will read your Claude Code login token from the macOS Keychain (item 'Claude Code-credentials') or ~/.claude/.credentials.json, and send it at most once per cache window (60 seconds by default) to https://api.anthropic.com/api/oauth/usage to read your spend limit. The token is never printed, logged, stored by playbook or passed to another program. This endpoint is undocumented and may change or stop working. Anthropic's terms may not allow using your login outside Claude Code; see docs/internals/04-usage-dashboard.md. If macOS asks for keychain access, choose Allow, not Always Allow. Turn it off with: playbook usage spend disable.";

/// A login token. It has no `Debug`, `Display`, `Clone` or `Serialize`, and
/// its bytes are overwritten when it is dropped.
pub struct Secret(String);

impl Secret {
    fn new(token: String) -> Secret {
        Secret(token)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe_string(&mut self.0);
    }
}

/// Overwrites `bytes` with zeros in a way the compiler may not remove.
fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

fn wipe_string(text: &mut String) {
    // SAFETY: zero bytes are valid UTF-8, so the string stays valid.
    wipe(unsafe { text.as_bytes_mut() });
    text.clear();
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<OAuth>,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<serde_json::Value>,
}

/// The access token in a credentials document. Only the token and its expiry
/// are read; the refresh token and everything else are skipped. A missing
/// token or a document of the wrong shape is `NoToken`. A token past its
/// expiry is `Unauthorized`, without a request. A missing expiry is accepted,
/// since the server will say; one of the wrong type is `NoToken`.
fn parse_credentials(text: &str, now: i64) -> Result<Secret, Reason> {
    let creds: Credentials = serde_json::from_str(text).map_err(|_| Reason::NoToken)?;
    let oauth = creds.oauth.ok_or(Reason::NoToken)?;
    let token = oauth
        .access_token
        .filter(|t| !t.trim().is_empty())
        .ok_or(Reason::NoToken)?;
    let secret = Secret::new(token);
    match oauth.expires_at {
        None | Some(serde_json::Value::Null) => Ok(secret),
        Some(serde_json::Value::Number(n)) => {
            let at = n.as_i64().ok_or(Reason::NoToken)?;
            let seconds = if at >= MILLIS_FROM {
                at / MILLIS_PER_SECOND
            } else {
                at
            };
            if seconds <= now {
                Err(Reason::Unauthorized)
            } else {
                Ok(secret)
            }
        }
        Some(_) => Err(Reason::NoToken),
    }
}

/// Where a token comes from.
pub trait TokenSource: Send + Sync {
    fn token(&self, now: i64) -> Result<Secret, Reason>;
}

/// The macOS keychain, through the `security` program.
pub struct Keychain {
    program: PathBuf,
    timeout: Duration,
}

impl Keychain {
    fn real() -> Keychain {
        Keychain {
            program: PathBuf::from(SECURITY),
            timeout: KEYCHAIN_TIMEOUT,
        }
    }

    /// The text the program prints, trimmed of its newline. The program gets
    /// the item name and nothing else: no shell, an empty environment, no
    /// input, and its errors are discarded.
    fn read(&self) -> Result<String, Reason> {
        let mut child = Command::new(&self.program)
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Reason::NoToken)?;
        let stdout = child.stdout.take().ok_or(Reason::NoToken)?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(KEYCHAIN_MAX_BYTES + 1).read_to_end(&mut bytes);
            bytes
        });
        let deadline = Instant::now() + self.timeout;
        let finished = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status.success()),
                Ok(None) if Instant::now() >= deadline => break None,
                Ok(None) => std::thread::sleep(POLL),
                Err(_) => break Some(false),
            }
        };
        if finished.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            // A process the program started may still hold the pipe open, so
            // the reader is left to finish by itself instead of waited for.
            return Err(Reason::Timeout);
        }
        let mut bytes = reader.join().unwrap_or_default();
        let outcome = match finished {
            None => Err(Reason::Timeout),
            Some(false) => Err(Reason::NoToken),
            Some(true) if bytes.len() as u64 > KEYCHAIN_MAX_BYTES => Err(Reason::NoToken),
            Some(true) => Ok(()),
        };
        if let Err(reason) = outcome {
            wipe(&mut bytes);
            return Err(reason);
        }
        match String::from_utf8(bytes) {
            Ok(mut text) => {
                let kept = text.trim_end().len();
                text.truncate(kept);
                Ok(text)
            }
            Err(e) => {
                let mut bytes = e.into_bytes();
                wipe(&mut bytes);
                Err(Reason::NoToken)
            }
        }
    }
}

impl TokenSource for Keychain {
    fn token(&self, now: i64) -> Result<Secret, Reason> {
        let mut text = self.read()?;
        let result = parse_credentials(&text, now);
        wipe_string(&mut text);
        result
    }
}

/// Claude Code's credentials file.
pub struct CredentialsFile {
    path: PathBuf,
}

impl CredentialsFile {
    fn real(home: &Path) -> CredentialsFile {
        let folder = std::env::var_os(CONFIG_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".claude"), PathBuf::from);
        CredentialsFile {
            path: folder.join(CREDENTIALS_FILE),
        }
    }

    /// The file's text. It is opened without following a link and checked on
    /// the open handle, so a symlink, a folder or an oversized file is refused.
    #[cfg(unix)]
    fn read(&self) -> Result<String, Reason> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)
            .map_err(|_| Reason::NoToken)?;
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err(Reason::NoToken);
        }
        let mut bytes = Vec::new();
        if file
            .take(FILE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > FILE_MAX_BYTES
        {
            wipe(&mut bytes);
            return Err(Reason::NoToken);
        }
        String::from_utf8(bytes).map_err(|e| {
            let mut bytes = e.into_bytes();
            wipe(&mut bytes);
            Reason::NoToken
        })
    }

    #[cfg(not(unix))]
    fn read(&self) -> Result<String, Reason> {
        Err(Reason::NoToken)
    }
}

impl TokenSource for CredentialsFile {
    fn token(&self, now: i64) -> Result<Secret, Reason> {
        let mut text = self.read()?;
        let result = parse_credentials(&text, now);
        wipe_string(&mut text);
        result
    }
}

/// Tries each source in turn. A source that has nothing passes to the next;
/// any other answer, an expired token included, is final.
pub struct Sources(Vec<Box<dyn TokenSource>>);

impl TokenSource for Sources {
    fn token(&self, now: i64) -> Result<Secret, Reason> {
        let mut last = Reason::NoToken;
        for source in &self.0 {
            match source.token(now) {
                Err(Reason::NoToken) => last = Reason::NoToken,
                other => return other,
            }
        }
        Err(last)
    }
}

/// The sources on this machine, in order.
fn real_sources(home: &Path) -> Sources {
    let mut sources: Vec<Box<dyn TokenSource>> = Vec::new();
    if cfg!(target_os = "macos") {
        sources.push(Box::new(Keychain::real()));
    }
    sources.push(Box::new(CredentialsFile::real(home)));
    Sources(sources)
}

/// Asks the server over HTTPS with the login token.
pub struct UreqSpendFetcher {
    url: String,
    tokens: Box<dyn TokenSource>,
    timeout: Duration,
    /// Plain HTTP to a loopback address, for tests only.
    local_http: bool,
}

impl UreqSpendFetcher {
    /// The fetcher for the real endpoint. It reads no token until asked.
    pub fn real(home: &Path) -> UreqSpendFetcher {
        UreqSpendFetcher {
            url: ENDPOINT.to_string(),
            tokens: Box::new(real_sources(home)),
            timeout: REQUEST_TIMEOUT,
            local_http: false,
        }
    }

    #[cfg(test)]
    fn loopback(url: String, tokens: Box<dyn TokenSource>, timeout: Duration) -> UreqSpendFetcher {
        UreqSpendFetcher {
            url,
            tokens,
            timeout,
            local_http: true,
        }
    }
}

/// The class of a failed request. The error's own text is never used: it can
/// hold the URL.
fn classify(error: &ureq::Error) -> Reason {
    use ureq::Error;
    match error {
        Error::StatusCode(401 | 403) => Reason::Unauthorized,
        Error::StatusCode(429) => Reason::RateLimited,
        Error::Timeout(_) => Reason::Timeout,
        Error::BodyExceedsLimit(_) => Reason::Schema,
        Error::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            Reason::Timeout
        }
        _ => Reason::Network,
    }
}

impl SpendFetcher for UreqSpendFetcher {
    fn fetch(&self, now: i64) -> Outcome {
        let secret = self.tokens.token(now)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(!self.local_http)
            .max_redirects(0)
            .proxy(None)
            .timeout_global(Some(self.timeout))
            .build()
            .into();
        let sent = agent
            .get(&self.url)
            .header("Authorization", format!("Bearer {}", secret.expose()))
            .header("anthropic-beta", BETA)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "identity")
            .header(
                "User-Agent",
                concat!("playbook/", env!("CARGO_PKG_VERSION")),
            )
            .call();
        drop(secret);
        let mut response = sent.map_err(|e| classify(&e))?;
        if !response.status().is_success() {
            return Err(Reason::Network);
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .map_err(|e| classify(&e))?;
        match spend::parse(&body, now) {
            Ok(Parsed::Figure(figure)) => Ok(Some(figure)),
            Ok(Parsed::Empty) => Ok(None),
            Err(_) => Err(Reason::Schema),
        }
    }
}

/// What `acknowledged` finds in the state store: the notice version and the
/// token source the user agreed to.
fn ack_value(now: i64) -> String {
    format!("{NOTICE_VERSION}|{SOURCE_NAME}@{now}")
}

/// Whether the user confirmed the current notice for this machine's token
/// source. A different version or source counts as no. The store is read
/// without creating it.
pub fn acknowledged(home: &Path) -> bool {
    let root = crate::common::paths::playbook_root_from(home);
    crate::state::list_read_only(&root, ACK_KEY)
        .ok()
        .and_then(|rows| rows.into_iter().find(|(k, _)| k == ACK_KEY))
        .is_some_and(|(_, value)| {
            let (head, _) = value.split_once('@').unwrap_or((&value, ""));
            head == format!("{NOTICE_VERSION}|{SOURCE_NAME}")
        })
}

fn enabled(home: &Path) -> bool {
    crate::config::resolve_valid("usage.spend.enabled", home, None)
        .ok()
        .is_some_and(|(value, _, _)| value == serde_json::Value::Bool(true))
}

fn set_enabled(home: &Path, on: bool) -> Result<(), String> {
    crate::config::write::set(
        crate::config::write::Tier::Global,
        "usage.spend.enabled",
        serde_json::Value::Bool(on),
        home,
        None,
    )
    .map_err(|e| e.to_string())
}

/// What the commands need from the machine, so tests can stand in for it.
pub struct Env<'a> {
    pub home: &'a Path,
    pub data_dir: &'a Path,
    pub now: i64,
    /// Whether a person can be asked.
    pub interactive: bool,
}

/// Runs `usage spend enable|disable|status`. The notice goes to `out` before
/// anything is asked. `ask` returns the answer to the question.
pub fn run(
    action: &SpendAction,
    env: &Env,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<String, String> {
    match action {
        SpendAction::Enable { yes } => enable(*yes, env, out, ask),
        SpendAction::Disable => disable(env),
        SpendAction::Status => Ok(status(env)),
    }
}

fn enable(
    yes: bool,
    env: &Env,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<String, String> {
    if enabled(env.home) && acknowledged(env.home) {
        return Ok(
            "The server spend limit is already on. Turn it off with: playbook usage spend disable"
                .to_string(),
        );
    }
    let _ = writeln!(out, "{NOTICE}");
    if !yes {
        if !env.interactive {
            return Err(
                "nothing was changed: read the notice above, then run `playbook usage spend enable --yes`"
                    .to_string(),
            );
        }
        if !ask() {
            return Ok("Nothing was changed.".to_string());
        }
    }
    set_enabled(env.home, true)?;
    let root = crate::common::paths::playbook_root_from(env.home);
    crate::state::set(&root, ACK_KEY, &ack_value(env.now))
        .map_err(|e| format!("could not store the acknowledgement: {e}"))?;
    Ok("The server spend limit is on. The next usage report asks for it.".to_string())
}

fn disable(env: &Env) -> Result<String, String> {
    set_enabled(env.home, false)?;
    let root = crate::common::paths::playbook_root_from(env.home);
    crate::state::delete(&root, ACK_KEY)
        .map_err(|e| format!("could not clear the acknowledgement: {e}"))?;
    spend_fetch::clear_cache(env.data_dir);
    Ok(
        "The server spend limit is off. The acknowledgement and the cached figure are removed."
            .to_string(),
    )
}

/// The state, without reading the token.
fn status(env: &Env) -> String {
    let cache = match spend_fetch::cache_status(env.data_dir, env.now) {
        None => "none".to_string(),
        Some((age, None)) => format!("ok, {age} seconds old"),
        Some((age, Some(reason))) => format!("{}, {age} seconds old", reason.key()),
    };
    format!(
        "usage.spend.enabled: {}\nacknowledged: {}\ntoken source: {SOURCE_NAME}\nlast result: {cache}",
        enabled(env.home),
        if acknowledged(env.home) { "yes" } else { "no" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const NOW: i64 = 1_788_400_000;
    const FAKE_SECRET: &str = "TEST-SECRET-NOT-A-REAL-TOKEN";
    const OK_BODY: &str = r#"{"extra_usage":{"monthly_limit":50000,"used_credits":12345,"decimal_places":2,"currency":"USD","utilization":24.69},"spend":{"severity":"normal"}}"#;

    /// A token source that hands out the fake secret and counts its calls.
    struct Fake {
        calls: Arc<AtomicUsize>,
        answer: Result<(), Reason>,
    }

    impl TokenSource for Fake {
        fn token(&self, _now: i64) -> Result<Secret, Reason> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.map(|()| Secret::new(FAKE_SECRET.to_string()))
        }
    }

    fn fake_token() -> Box<dyn TokenSource> {
        Box::new(Fake {
            calls: Arc::new(AtomicUsize::new(0)),
            answer: Ok(()),
        })
    }

    fn response(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Answers one request with `reply` and returns the request it saw.
    fn serve(reply: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/api/oauth/usage",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the fetcher never connected");
            let mut seen = Vec::new();
            let mut chunk = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = std::io::Read::read(&mut stream, &mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&chunk[..n]);
            }
            let _ = stream.write_all(reply.as_bytes());
            String::from_utf8_lossy(&seen).into_owned()
        });
        (url, handle)
    }

    fn fetch_from(reply: String) -> (Outcome, String) {
        let (url, handle) = serve(reply);
        let fetcher = UreqSpendFetcher::loopback(url, fake_token(), Duration::from_millis(500));
        let outcome = fetcher.fetch(NOW);
        (outcome, handle.join().unwrap())
    }

    #[test]
    fn a_good_reply_gives_the_figure_and_the_request_is_exactly_what_the_notice_says() {
        let (outcome, request) = fetch_from(response("200 OK", "", OK_BODY));

        let figure = outcome.unwrap().unwrap();
        assert_eq!(figure.limit_usd(), Some(500.0));
        let lower = request.to_lowercase();
        assert!(
            request.starts_with("GET /api/oauth/usage HTTP/1.1"),
            "{request}"
        );
        assert!(lower.contains(&format!(
            "authorization: bearer {}",
            FAKE_SECRET.to_lowercase()
        )));
        assert!(lower.contains("anthropic-beta: oauth-2025-04-20"));
        assert!(lower.contains("accept-encoding: identity"));
        assert_eq!(request.matches("GET ").count(), 1);
    }

    #[test]
    fn the_endpoint_is_one_https_address() {
        assert_eq!(ENDPOINT, "https://api.anthropic.com/api/oauth/usage");
        let real = UreqSpendFetcher::real(Path::new("/nonexistent-home"));
        assert!(!real.local_http);
        assert_eq!(real.url, ENDPOINT);
        assert_eq!(real.timeout, REQUEST_TIMEOUT);
    }

    #[test]
    fn a_cached_fetch_leaves_the_fake_secret_out_of_the_cache_file() {
        let (url, handle) = serve(response("200 OK", "", OK_BODY));
        let dir = scratch_dir("spend-e2e");
        std::fs::create_dir_all(&dir).unwrap();
        let fetcher = UreqSpendFetcher::loopback(url, fake_token(), Duration::from_millis(500));
        let cache = spend_fetch::CachedFetcher::new(fetcher, &dir, 60);

        let figure = cache.get(NOW).unwrap().unwrap();
        handle.join().unwrap();

        let file = std::fs::read_to_string(dir.join(spend_fetch::CACHE_FILE)).unwrap();
        assert!(!file.contains(FAKE_SECRET) && !file.to_lowercase().contains("bearer"));
        assert_eq!(cache.cached(NOW + 1), Some(Ok(Some(figure))));
    }

    #[test]
    fn a_reply_with_no_limit_is_no_figure() {
        let (outcome, _) = fetch_from(response("200 OK", "", "{}"));

        assert_eq!(outcome, Ok(None));
    }

    #[test]
    fn each_http_failure_maps_to_its_class() {
        let cases = [
            ("401 Unauthorized", Reason::Unauthorized),
            ("403 Forbidden", Reason::Unauthorized),
            ("429 Too Many Requests", Reason::RateLimited),
            ("500 Internal Server Error", Reason::Network),
        ];
        for (status, reason) in cases {
            let headers = if status.starts_with("429") {
                "Retry-After: 30\r\n"
            } else {
                ""
            };
            let (outcome, _) = fetch_from(response(status, headers, ""));
            assert_eq!(outcome, Err(reason), "{status}");
        }
    }

    #[test]
    fn a_garbage_reply_is_a_schema_failure() {
        let (outcome, _) = fetch_from(response("200 OK", "", "<html>not json</html>"));

        assert_eq!(outcome, Err(Reason::Schema));
    }

    #[test]
    fn an_oversized_reply_is_refused() {
        let body = format!(r#"{{"pad":"{}"}}"#, "x".repeat(BODY_LIMIT as usize));
        let (outcome, _) = fetch_from(response("200 OK", "", &body));

        assert_eq!(outcome, Err(Reason::Schema));
    }

    #[test]
    fn a_redirect_is_not_followed_and_the_token_goes_nowhere_else() {
        let (other_url, other) = serve(response("200 OK", "", OK_BODY));
        let port = other_url
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_string();
        let (outcome, _) = fetch_from(response(
            "302 Found",
            &format!("Location: http://127.0.0.1:{port}/steal\r\n"),
            "",
        ));
        let probe = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));
        drop(probe);
        let seen = other.join().unwrap();

        assert_eq!(outcome, Err(Reason::Network));
        assert!(
            !seen.contains(FAKE_SECRET),
            "the second server saw the token"
        );
        assert!(!seen.to_lowercase().contains("authorization"), "{seen}");
    }

    #[test]
    fn a_server_that_never_answers_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/api/oauth/usage",
            listener.local_addr().unwrap().port()
        );
        let fetcher = UreqSpendFetcher::loopback(url, fake_token(), Duration::from_millis(500));
        let started = Instant::now();

        let outcome = fetcher.fetch(NOW);

        assert_eq!(outcome, Err(Reason::Timeout));
        assert!(started.elapsed() < Duration::from_secs(6));
        drop(listener);
    }

    #[test]
    fn a_reset_connection_is_a_network_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/api/oauth/usage",
            listener.local_addr().unwrap().port()
        );
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(stream);
        });
        let fetcher = UreqSpendFetcher::loopback(url, fake_token(), Duration::from_millis(500));

        let outcome = fetcher.fetch(NOW);
        handle.join().unwrap();

        assert!(matches!(outcome, Err(Reason::Network | Reason::Timeout)));
    }

    #[test]
    fn no_token_means_no_request_and_the_production_fetcher_refuses_plain_http() {
        let calls = Arc::new(AtomicUsize::new(0));
        let none = Box::new(Fake {
            calls: calls.clone(),
            answer: Err(Reason::NoToken),
        });
        let (url, _handle) = serve(response("200 OK", "", OK_BODY));
        let fetcher = UreqSpendFetcher::loopback(url.clone(), none, Duration::from_millis(500));
        assert_eq!(fetcher.fetch(NOW), Err(Reason::NoToken));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let strict = UreqSpendFetcher {
            url,
            tokens: fake_token(),
            timeout: Duration::from_millis(500),
            local_http: false,
        };
        assert_eq!(strict.fetch(NOW), Err(Reason::Network));
    }

    #[test]
    fn the_fake_secret_is_in_no_failure_value() {
        let reasons = [
            Reason::Unauthorized,
            Reason::Network,
            Reason::Timeout,
            Reason::Schema,
        ];
        for (status, _) in [("401 Unauthorized", 0), ("500 Internal Server Error", 0)] {
            let (outcome, _) = fetch_from(response(status, "", ""));
            let text = format!("{outcome:?}");
            assert!(!text.contains(FAKE_SECRET), "{text}");
        }
        for r in reasons {
            assert!(!r.line().contains(FAKE_SECRET));
        }
    }

    #[test]
    fn the_credentials_parser_reads_only_the_token_and_checks_the_expiry() {
        let doc = |extra: &str| {
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"{FAKE_SECRET}","refreshToken":"OTHER"{extra}}}}}"#
            )
        };
        let future_ms = (NOW + 3600) * MILLIS_PER_SECOND;
        let past_ms = (NOW - 3600) * MILLIS_PER_SECOND;

        let ok = parse_credentials(&doc(&format!(r#","expiresAt":{future_ms}"#)), NOW).unwrap();
        assert_eq!(ok.expose(), FAKE_SECRET);
        assert!(parse_credentials(&doc(""), NOW).is_ok());
        assert!(parse_credentials(&doc(&format!(r#","expiresAt":{}"#, NOW + 60)), NOW).is_ok());
        assert_eq!(
            parse_credentials(&doc(&format!(r#","expiresAt":{past_ms}"#)), NOW).err(),
            Some(Reason::Unauthorized)
        );
        assert_eq!(
            parse_credentials(&doc(r#","expiresAt":"soon""#), NOW).err(),
            Some(Reason::NoToken)
        );
        for bad in [
            "{}",
            "[]",
            "not json",
            r#"{"claudeAiOauth":{}}"#,
            r#"{"claudeAiOauth":{"accessToken":7}}"#,
            r#"{"claudeAiOauth":{"accessToken":"  "}}"#,
        ] {
            assert_eq!(
                parse_credentials(bad, NOW).err(),
                Some(Reason::NoToken),
                "{bad}"
            );
        }
    }

    #[test]
    fn wiping_overwrites_every_byte() {
        let mut bytes = FAKE_SECRET.as_bytes().to_vec();

        wipe(&mut bytes);

        assert!(bytes.iter().all(|b| *b == 0));
        assert_eq!(bytes.len(), FAKE_SECRET.len());
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    const SHORT: Duration = Duration::from_millis(300);

    #[cfg(unix)]
    #[test]
    fn the_keychain_source_reads_the_token_from_the_program_output() {
        let dir = scratch_dir("spend-kc-ok");
        let json = format!(r#"{{"claudeAiOauth":{{"accessToken":"{FAKE_SECRET}"}}}}"#);
        let program = script(&dir, "security", &format!("echo '{json}'"));
        let source = Keychain {
            program,
            timeout: Duration::from_secs(5),
        };

        let secret = source.token(NOW).unwrap();

        assert_eq!(secret.expose(), FAKE_SECRET);
    }

    #[cfg(unix)]
    #[test]
    fn the_keychain_program_gets_only_the_item_name_and_an_empty_environment() {
        let dir = scratch_dir("spend-kc-env");
        let log = dir.join("seen.txt");
        let program = script(
            &dir,
            "security",
            &format!(
                "echo \"$@\" > '{}'; env >> '{}'; exit 44",
                log.display(),
                log.display()
            ),
        );
        let source = Keychain {
            program,
            timeout: Duration::from_secs(5),
        };

        let result = source.token(NOW);

        assert_eq!(result.err(), Some(Reason::NoToken));
        let seen = std::fs::read_to_string(&log).unwrap();
        assert!(
            seen.starts_with("find-generic-password -s Claude Code-credentials -w"),
            "{seen}"
        );
        assert!(
            !seen.contains("HOME="),
            "the environment was not cleared: {seen}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_hanging_or_noisy_keychain_program_fails_soft() {
        let dir = scratch_dir("spend-kc-bad");
        let missing = Keychain {
            program: dir.join("absent"),
            timeout: SHORT,
        };
        assert_eq!(missing.token(NOW).err(), Some(Reason::NoToken));

        let hang = Keychain {
            program: script(&dir, "hang", "sleep 30"),
            timeout: SHORT,
        };
        let started = Instant::now();
        assert_eq!(hang.token(NOW).err(), Some(Reason::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));

        let big = script(&dir, "big", "head -c 20000 /dev/zero | tr '\\0' 'a'");
        let noisy = Keychain {
            program: big,
            timeout: Duration::from_secs(5),
        };
        assert_eq!(noisy.token(NOW).err(), Some(Reason::NoToken));
    }

    #[cfg(unix)]
    #[test]
    fn the_credentials_file_is_read_and_a_symlink_is_refused() {
        let dir = scratch_dir("spend-file");
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.json");
        std::fs::write(
            &real,
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{FAKE_SECRET}"}}}}"#),
        )
        .unwrap();
        let direct = CredentialsFile { path: real.clone() };
        assert_eq!(direct.token(NOW).unwrap().expose(), FAKE_SECRET);

        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let linked = CredentialsFile { path: link };
        assert_eq!(linked.token(NOW).err(), Some(Reason::NoToken));

        let folder = CredentialsFile { path: dir.clone() };
        assert_eq!(folder.token(NOW).err(), Some(Reason::NoToken));
        let gone = CredentialsFile {
            path: dir.join("absent.json"),
        };
        assert_eq!(gone.token(NOW).err(), Some(Reason::NoToken));
    }

    #[test]
    fn the_sources_fall_through_on_nothing_and_stop_on_anything_else() {
        let calls = Arc::new(AtomicUsize::new(0));
        let make = |answer| -> Box<dyn TokenSource> {
            Box::new(Fake {
                calls: calls.clone(),
                answer,
            })
        };

        let falls = Sources(vec![make(Err(Reason::NoToken)), make(Ok(()))]);
        assert!(falls.token(NOW).is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        calls.store(0, Ordering::SeqCst);
        let stops = Sources(vec![make(Err(Reason::Unauthorized)), make(Ok(()))]);
        assert_eq!(stops.token(NOW).err(), Some(Reason::Unauthorized));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct Machine {
        home: PathBuf,
        data: PathBuf,
    }

    impl Machine {
        fn new(tag: &str) -> Machine {
            let home = scratch_dir(tag);
            let data = home.join("data");
            std::fs::create_dir_all(&data).unwrap();
            Machine { home, data }
        }

        fn env(&self, interactive: bool) -> Env<'_> {
            Env {
                home: &self.home,
                data_dir: &self.data,
                now: NOW,
                interactive,
            }
        }

        fn run(
            &self,
            action: SpendAction,
            interactive: bool,
            answer: bool,
        ) -> (Result<String, String>, String) {
            let mut out = Vec::new();
            let result = run(&action, &self.env(interactive), &mut out, &mut || answer);
            (result, String::from_utf8(out).unwrap())
        }
    }

    #[test]
    fn the_notice_names_the_risks_and_the_way_out() {
        for phrase in [
            "undocumented",
            "never printed",
            "playbook usage spend disable",
            "Always Allow",
        ] {
            assert!(NOTICE.contains(phrase), "{phrase}");
        }
        assert!(!NOTICE.contains('\u{2014}') && !NOTICE.contains('\u{2013}'));
    }

    #[test]
    fn by_default_nothing_is_on_or_acknowledged() {
        let m = Machine::new("spend-default");

        let (status, _) = m.run(SpendAction::Status, false, false);

        let text = status.unwrap();
        assert!(text.contains("usage.spend.enabled: false"), "{text}");
        assert!(text.contains("acknowledged: no"), "{text}");
        assert!(!enabled(&m.home) && !acknowledged(&m.home));
    }

    #[test]
    fn enable_without_a_yes_and_without_a_person_changes_nothing_and_shows_the_notice() {
        let m = Machine::new("spend-noyes");

        let (result, shown) = m.run(SpendAction::Enable { yes: false }, false, true);

        assert!(result.is_err());
        assert!(shown.contains("undocumented"));
        assert!(!enabled(&m.home) && !acknowledged(&m.home));
    }

    #[test]
    fn enable_asks_a_person_and_a_no_changes_nothing() {
        let m = Machine::new("spend-no");

        let (result, shown) = m.run(SpendAction::Enable { yes: false }, true, false);

        assert_eq!(result.unwrap(), "Nothing was changed.");
        assert!(shown.contains("undocumented"));
        assert!(!enabled(&m.home));
    }

    #[test]
    fn enable_with_a_yes_turns_it_on_and_shows_the_notice_once() {
        let m = Machine::new("spend-yes");

        let (first, shown_first) = m.run(SpendAction::Enable { yes: true }, false, false);
        let (second, shown_second) = m.run(SpendAction::Enable { yes: true }, false, false);

        assert!(first.is_ok() && second.unwrap().contains("already on"));
        assert!(shown_first.contains("undocumented"));
        assert!(shown_second.is_empty());
        assert!(enabled(&m.home) && acknowledged(&m.home));
    }

    #[test]
    fn a_key_set_without_the_acknowledgement_counts_as_not_acknowledged() {
        let m = Machine::new("spend-keyonly");
        set_enabled(&m.home, true).unwrap();

        assert!(enabled(&m.home));
        assert!(!acknowledged(&m.home));
    }

    #[test]
    fn an_acknowledgement_for_another_notice_version_or_source_counts_as_none() {
        let m = Machine::new("spend-stale-ack");
        let root = crate::common::paths::playbook_root_from(&m.home);
        for value in [
            format!("0|{SOURCE_NAME}@{NOW}"),
            format!("{NOTICE_VERSION}|other@{NOW}"),
            "garbage".to_string(),
        ] {
            crate::state::set(&root, ACK_KEY, &value).unwrap();
            assert!(!acknowledged(&m.home), "{value}");
        }
        crate::state::set(&root, ACK_KEY, &ack_value(NOW)).unwrap();
        assert!(acknowledged(&m.home));
    }

    #[test]
    fn disable_turns_the_key_off_and_removes_the_acknowledgement_and_the_cache() {
        let m = Machine::new("spend-disable");
        m.run(SpendAction::Enable { yes: true }, false, false)
            .0
            .unwrap();
        let cache = m.data.join(spend_fetch::CACHE_FILE);
        std::fs::write(&cache, "{}").unwrap();

        let (result, _) = m.run(SpendAction::Disable, false, false);

        assert!(result.is_ok());
        assert!(!enabled(&m.home) && !acknowledged(&m.home));
        assert!(!cache.exists());
    }

    #[test]
    fn the_status_never_contains_a_token_and_reports_the_last_failure() {
        let m = Machine::new("spend-status");
        m.run(SpendAction::Enable { yes: true }, false, false)
            .0
            .unwrap();
        let entry = format!(
            r#"{{"fetched_at":{},"error":"unauthorized","figure":null}}"#,
            NOW - 30
        );
        std::fs::write(m.data.join(spend_fetch::CACHE_FILE), entry).unwrap();

        let (status, _) = m.run(SpendAction::Status, false, false);

        let text = status.unwrap();
        assert!(text.contains("usage.spend.enabled: true"), "{text}");
        assert!(text.contains("acknowledged: yes"), "{text}");
        assert!(text.contains("unauthorized, 30 seconds old"), "{text}");
        assert!(!text.contains(FAKE_SECRET));
    }
}
