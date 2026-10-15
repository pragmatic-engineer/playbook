// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The HTTP seam. Production uses `ureq` over rustls; tests inject a map.

use std::time::Duration;

/// Fetches a URL's body. One method, so tests can swap in canned responses.
pub trait Fetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// Release assets are a few MB; this bounds a hostile or broken response.
const MAX_BODY_BYTES: u64 = 256 * 1024 * 1024;

/// The time limit for a release download.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The real fetcher: HTTPS via rustls, no system OpenSSL. `local_http` is for
/// tests: it allows plain http and ignores proxy environment variables.
/// `timeout` bounds the whole request.
pub struct HttpFetcher {
    pub local_http: bool,
    pub timeout: Duration,
}

impl Fetcher for HttpFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .https_only(!self.local_http);
        if self.local_http {
            config = config.proxy(None);
        }
        let agent: ureq::Agent = config.build().into();
        let mut response = agent
            .get(url)
            .header(
                "User-Agent",
                concat!("playbook/", env!("CARGO_PKG_VERSION")),
            )
            .header(
                "Accept",
                "application/vnd.github+json, application/octet-stream",
            )
            .call()
            .map_err(|err| format!("{url}: {err}"))?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()
            .map_err(|err| format!("{url}: {err}"))
    }
}

#[cfg(test)]
pub mod fake {
    use super::Fetcher;
    use std::collections::HashMap;

    /// A fetcher over canned bodies; an unknown URL is a 404-style error.
    #[derive(Default)]
    pub struct FakeFetcher(pub HashMap<String, Vec<u8>>);

    impl Fetcher for FakeFetcher {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| format!("{url}: HTTP 404"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers one request on a local port with the given raw HTTP response.
    fn serve_once(response: &'static str) -> (u16, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the fetcher never connected");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).unwrap();
        });
        (port, handle)
    }

    #[test]
    fn http_fetcher_reads_a_body_from_a_local_server() {
        let (port, handle) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        let body = HttpFetcher {
            local_http: true,
            timeout: DOWNLOAD_TIMEOUT,
        }
        .get(&format!("http://127.0.0.1:{port}/x"))
        .unwrap();
        handle.join().unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn http_fetcher_reports_an_http_error() {
        let (port, handle) =
            serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let err = HttpFetcher {
            local_http: true,
            timeout: DOWNLOAD_TIMEOUT,
        }
        .get(&format!("http://127.0.0.1:{port}/x"))
        .unwrap_err();
        handle.join().unwrap();
        assert!(err.contains("404"), "{err}");
    }

    #[test]
    fn the_production_fetcher_refuses_plain_http() {
        let err = HttpFetcher {
            local_http: false,
            timeout: DOWNLOAD_TIMEOUT,
        }
        .get("http://127.0.0.1:9/x")
        .unwrap_err();
        assert!(err.to_lowercase().contains("http"), "{err}");
    }
}
