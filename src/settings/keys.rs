// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Single-sourced allowlist for `settings.shared.json`: the only keys `gen`
//! may keep and `check` may accept, so both agree on what ships.

/// Current `settings.shared.json` top-level keys minus `autoMode` and
/// `enabledPlugins`, both machine-specific plugin/mode state.
pub const SHIPPABLE_KEYS: &[&str] = &[
    "$schema",
    "cleanupPeriodDays",
    "env",
    "includeCoAuthoredBy",
    "includeGitInstructions",
    "permissions",
    "hooks",
    "worktree",
    "statusLine",
    "outputStyle",
    "feedbackSurveyRate",
    "spinnerTipsEnabled",
    "awaySummaryEnabled",
    "autoUpdatesChannel",
    "tui",
    "skipDangerousModePermissionPrompt",
    "editorMode",
    "teammateMode",
    "remoteControlAtStartup",
    "inputNeededNotifEnabled",
    "agentPushNotifEnabled",
    "skipAutoPermissionPrompt",
    "useAutoModeDuringPlan",
];

/// `env` keys safe to ship: telemetry opt-outs and per-install toggles.
/// Dropped as machine/demo/experimental: `IS_DEMO`,
/// `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`, `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`.
pub const SHIPPABLE_ENV: &[&str] = &[
    "CLAUDE_CODE_ENABLE_TELEMETRY",
    "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS",
    "DISABLE_UPGRADE_COMMAND",
    "DISABLE_COST_WARNINGS",
    "DISABLE_AUTOUPDATER",
    "DISABLE_TELEMETRY",
    "ENABLE_TOOL_SEARCH",
    "DO_NOT_TRACK",
];
