// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Which account the local Claude Code install is signed in to. Read once per
//! ingest and tagged onto every event; the transcripts do not carry it.

use serde_json::Value;
use std::fs;
use std::path::Path;

/// Label used when no account can be read (not signed in, file missing).
pub const UNKNOWN_ACCOUNT: &str = "unknown";

/// `oauthAccount.emailAddress` from the given `~/.claude.json`, or
/// `UNKNOWN_ACCOUNT`. Never an error: a user who has not finished onboarding
/// must still be able to ingest.
pub fn account_label(claude_json: &Path) -> String {
    fs::read_to_string(claude_json)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| {
            v.get("oauthAccount")?
                .get("emailAddress")?
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| UNKNOWN_ACCOUNT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/usage/account")
            .join(name)
    }

    #[test]
    fn reads_the_email_address() {
        assert_eq!(account_label(&fixture("claude.json")), "dev@example.com");
    }

    #[test]
    fn missing_oauth_account_is_unknown_not_an_error() {
        assert_eq!(account_label(&fixture("no-account.json")), UNKNOWN_ACCOUNT);
    }

    #[test]
    fn missing_file_is_unknown() {
        assert_eq!(
            account_label(&fixture("does-not-exist.json")),
            UNKNOWN_ACCOUNT
        );
    }
}
