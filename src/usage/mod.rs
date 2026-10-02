// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Usage tracking: a `UsageSource` adapter turns one coding agent's local
//! session history into normalized events. Aggregation and storage never see
//! an agent's own file format, so a second source only implements the trait.

pub mod account;
pub mod aggregate;
pub mod claude_code;
pub mod db;
pub mod ingest;
pub mod pricing;
pub mod run;
pub mod summary;

/// One assistant message's token usage and derived cost. `event_id` is the
/// source's own unique message id, the dedup key (a message can span several
/// transcript lines carrying identical usage).
#[derive(Debug, Clone, PartialEq)]
pub struct UsageEvent {
    pub event_id: String,
    pub timestamp: i64,
    pub session_id: String,
    pub account: String,
    pub model: String,
    pub effort: String,
    pub repo: String,
    pub branch: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Skill,
    Agent,
}

impl ToolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolKind::Skill => "skill",
            ToolKind::Agent => "agent",
        }
    }
}

/// One `Skill` or `Agent` tool call. `event_id` is the tool_use id.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocationEvent {
    pub event_id: String,
    pub timestamp: i64,
    pub session_id: String,
    pub account: String,
    pub kind: ToolKind,
    pub name: String,
}

/// Everything a source yields for one scan.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Events {
    pub usage: Vec<UsageEvent>,
    pub tools: Vec<ToolInvocationEvent>,
}

/// An agent's local history, read as normalized events. Events carry an empty
/// `account`; ingest tags it once per run.
pub trait UsageSource {
    /// Stable name keying this source's watermark.
    fn name(&self) -> &str;
    /// Events with `timestamp >= watermark` (epoch seconds). The boundary is
    /// inclusive: a re-read of the boundary second is absorbed by dedup.
    fn events_since(&self, watermark: i64) -> Result<Events, String>;
}
