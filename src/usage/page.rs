// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The dashboard page: three static, same-origin assets (`/`, `/app.css`,
//! `/app.js`) with nothing inline, so the policy can forbid inline scripts and
//! styles outright. The script polls `/api/data` with the session token and
//! rebuilds the page. Dynamic strings reach the DOM only through `textContent`;
//! the one `innerHTML` use is the server's escaped SVG.

pub const REFRESH_MS: u32 = 5000;

/// Scripts and styles load only from this origin. The SVG charts carry no
/// inline `style` attributes, so `style-src` needs no `'unsafe-inline'`.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

pub const HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>playbook usage</title>
<link rel="stylesheet" href="/app.css">
</head>
<body>
<main>
<header class="top">
<h1>playbook usage</h1>
<div class="ranges" id="ranges" role="group" aria-label="Date range"></div>
</header>
<div class="period" id="period"></div>
<div class="muted" id="status">Loading…</div>
<noscript><p>This page needs JavaScript to load its data.</p></noscript>
<div class="totals" id="totals"></div>
<div class="charts" id="charts"></div>
<div id="tables"></div>
</main>
<script src="/app.js"></script>
</body>
</html>
"##;

pub const CSS: &str = r##":root { --bg:#fafafa; --fg:#1b1b1f; --muted:#666; --card:#fff; --line:#ddd; --bar:#3b6fd4; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#16171a; --fg:#e8e8ea; --muted:#9a9aa2; --card:#1f2024; --line:#34353b; --bar:#6b9bf2; }
}
*, *::before, *::after { box-sizing:border-box; }
body { margin:0; padding:1rem 1rem 3rem; background:var(--bg); color:var(--fg); font:14px system-ui, sans-serif; -webkit-text-size-adjust:100%; }
main { max-width:72rem; margin:0 auto; }
.top { display:flex; flex-wrap:wrap; align-items:center; justify-content:space-between; gap:.75rem; }
h1 { font-size:1.3rem; margin:0; }
h2 { font-size:1rem; margin:1.5rem 0 .5rem; }
.muted { color:var(--muted); }
.ranges { display:flex; flex-wrap:wrap; border:1px solid var(--line); border-radius:6px; overflow:hidden; background:var(--card); }
.ranges button { flex:1 1 auto; min-height:2.25rem; padding:.35rem .8rem; border:0; border-left:1px solid var(--line); background:transparent; color:var(--fg); font:inherit; cursor:pointer; }
.ranges button:first-child { border-left:0; }
.ranges button[aria-pressed="true"] { background:var(--bar); color:#fff; }
.ranges button:focus-visible { outline:2px solid var(--bar); outline-offset:-2px; }
.period { margin:.75rem 0 .25rem; font-weight:600; }
.totals { display:grid; grid-template-columns:repeat(auto-fill, minmax(10rem, 1fr)); gap:.75rem; margin:1rem 0; }
.total { min-width:0; background:var(--card); border:1px solid var(--line); border-radius:6px; padding:.5rem .75rem; }
.total b { display:block; font-size:1.1rem; overflow-wrap:anywhere; }
.charts { display:grid; grid-template-columns:repeat(auto-fit, minmax(min(100%, 24rem), 1fr)); gap:1rem; }
.chart-wrap { min-width:0; overflow-x:auto; background:var(--card); border:1px solid var(--line); border-radius:6px; }
.chart { display:block; width:100%; min-width:26rem; height:auto; }
.chart .bar { fill:var(--bar); }
.chart .axis { stroke:var(--line); }
.chart text { fill:var(--fg); font-size:11px; }
.chart .tick { fill:var(--muted); font-size:10px; }
.scroll { overflow-x:auto; -webkit-overflow-scrolling:touch; }
table { border-collapse:collapse; width:100%; background:var(--card); border:1px solid var(--line); }
th, td { padding:.3rem .6rem; text-align:right; border-bottom:1px solid var(--line); white-space:nowrap; }
th:first-child, td:first-child { text-align:left; }
@media (max-width: 40rem) {
  body { padding:.75rem .75rem 2rem; font-size:13px; }
  .top { flex-direction:column; align-items:stretch; }
  .totals { grid-template-columns:repeat(2, minmax(0, 1fr)); gap:.5rem; }
  th, td { padding:.3rem .45rem; }
  td:first-child { max-width:12rem; overflow:hidden; text-overflow:ellipsis; }
}
"##;

pub const JS: &str = r##"const REFRESH_MS = 5000;
const TOKEN_KEY = "playbook-usage-token";
const NEED_LINK = "Open a fresh link with: playbook usage dashboard";
const TABLES = [["day","By day (UTC)"],["week","By week (UTC, starting Monday)"],["model","By model"],["repo","By repo"],["branch","By branch"],["effort","By effort"],["account","By account"]];
const COLUMNS = [["messages","Messages"],["input_tokens","Input"],["output_tokens","Output"],["cache_creation_tokens","Cache write"],["cache_read_tokens","Cache read"],["cost_usd","Cost (USD)"]];
const RANGES = [["30d","Last 30 days"],["60d","Last 60 days"],["90d","Last 90 days"],["month","Current month"],["all","All time"]];
const RANGE_KEY = "playbook-usage-range";

// The link carries the session token in its fragment, which the browser
// never sends to the server. Keep it for reloads and drop it from the address.
function readToken() {
  let token = "";
  try { token = sessionStorage.getItem(TOKEN_KEY) || ""; } catch (e) {}
  const fromLink = location.hash.replace(/^#/, "");
  if (/^[0-9a-f]{64}$/.test(fromLink)) {
    token = fromLink;
    try { sessionStorage.setItem(TOKEN_KEY, token); } catch (e) {}
    history.replaceState(null, "", location.pathname + location.search);
  }
  return token;
}
const TOKEN = readToken();

function readRange() {
  let saved = "";
  try { saved = localStorage.getItem(RANGE_KEY) || ""; } catch (e) {}
  return RANGES.some(r => r[0] === saved) ? saved : "30d";
}
let range = readRange();

function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function fmt(key, value) {
  return key === "cost_usd" ? Number(value).toFixed(4) : Number(value).toLocaleString();
}
function table(title, firstHeader, columns, rows) {
  const wrap = el("div");
  wrap.appendChild(el("h2", title));
  const scroll = el("div", undefined, "scroll");
  const t = el("table");
  const head = el("tr");
  head.appendChild(el("th", firstHeader));
  columns.forEach(c => head.appendChild(el("th", c[1])));
  t.appendChild(head);
  rows.forEach(r => {
    const tr = el("tr");
    tr.appendChild(el("td", r.key !== undefined ? r.key : r.name));
    columns.forEach(c => tr.appendChild(el("td", fmt(c[0], r[c[0]]))));
    t.appendChild(tr);
  });
  scroll.appendChild(t);
  wrap.appendChild(scroll);
  return wrap;
}
function renderRanges() {
  const box = document.getElementById("ranges");
  box.replaceChildren();
  RANGES.forEach(r => {
    const button = el("button", r[1]);
    button.type = "button";
    button.setAttribute("aria-pressed", String(r[0] === range));
    button.addEventListener("click", () => {
      if (r[0] === range) return;
      range = r[0];
      try { localStorage.setItem(RANGE_KEY, range); } catch (e) {}
      renderRanges();
      document.getElementById("status").textContent = "Loading…";
      refresh();
    });
    box.appendChild(button);
  });
}
function rangeName(key) {
  const found = RANGES.find(r => r[0] === key);
  return found ? found[1] : key;
}
function render(data) {
  const r = data.range;
  document.getElementById("period").textContent = rangeName(r.key) + ": " +
    (r.start ? r.start + " to " + r.end + " (UTC)" : "no usage recorded yet");
  const totals = document.getElementById("totals");
  totals.replaceChildren();
  [["Messages", fmt("messages", data.totals.messages)],
   ["Input tokens", fmt("t", data.totals.input_tokens)],
   ["Output tokens", fmt("t", data.totals.output_tokens)],
   ["Cache write tokens", fmt("t", data.totals.cache_creation_tokens)],
   ["Cache read tokens", fmt("t", data.totals.cache_read_tokens)],
   ["Estimated cost (USD)", fmt("cost_usd", data.totals.cost_usd)]].forEach(p => {
    const box = el("div", undefined, "total");
    box.appendChild(el("span", p[0], "muted"));
    box.appendChild(el("b", p[1]));
    totals.appendChild(box);
  });
  const charts = document.getElementById("charts");
  charts.innerHTML = data.charts.cost_by_day + data.charts.cost_by_model;
  charts.querySelectorAll("svg").forEach(svg => {
    const wrap = el("div", undefined, "chart-wrap");
    svg.replaceWith(wrap);
    wrap.appendChild(svg);
  });
  const tables = document.getElementById("tables");
  tables.replaceChildren();
  TABLES.forEach(t => tables.appendChild(table(t[1], t[0], COLUMNS, data.groups[t[0]])));
  if (data.skills.length) tables.appendChild(table("Skills", "skill", [["count","Uses"]], data.skills));
  if (data.agents.length) tables.appendChild(table("Agents", "agent", [["count","Dispatches"]], data.agents));
}
let timer = null;
let latest = 0;
function stopPolling(status, message) {
  if (timer !== null) clearInterval(timer);
  timer = null;
  status.textContent = message;
}
async function refresh() {
  const status = document.getElementById("status");
  if (!TOKEN) { stopPolling(status, "No session token. " + NEED_LINK); return; }
  // A slow answer for a range the user has since left must not overwrite the newer one.
  const mine = ++latest;
  try {
    const response = await fetch('/api/data?range=' + encodeURIComponent(range), { headers: { 'X-Playbook-Token': TOKEN } });
    if (mine !== latest) return;
    if (response.status === 401) { stopPolling(status, "The session token was rejected. " + NEED_LINK); return; }
    if (!response.ok) throw new Error("HTTP " + response.status);
    const data = await response.json();
    if (mine !== latest) return;
    render(data);
    status.textContent = "Updated " + new Date().toLocaleTimeString() + ". Refreshes every " + (REFRESH_MS / 1000) + " seconds.";
  } catch (error) {
    if (mine === latest) status.textContent = "Could not refresh: " + error.message;
  }
}
renderRanges();
refresh();
timer = setInterval(refresh, REFRESH_MS);
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_polls_the_data_endpoint_with_the_token_at_the_declared_interval() {
        assert!(JS.contains("fetch('/api/data?range=' + encodeURIComponent(range), { headers: { 'X-Playbook-Token': TOKEN } })"));
        assert!(JS.contains(&format!("const REFRESH_MS = {REFRESH_MS};")));
        assert!(JS.contains("setInterval(refresh, REFRESH_MS)"));
    }

    #[test]
    fn the_script_takes_the_token_from_the_fragment_and_removes_it_from_the_address() {
        assert!(JS.contains("location.hash"));
        assert!(JS.contains("history.replaceState"));
        assert!(JS.contains("sessionStorage"));
        assert!(JS.contains("playbook usage dashboard"));
    }

    #[test]
    fn the_html_has_nothing_inline_and_loads_only_its_two_own_assets() {
        assert!(!HTML.contains("<style"));
        assert!(!HTML.contains(" style="));
        assert!(!HTML.contains(" onclick") && !HTML.contains(" onload"));
        assert_eq!(HTML.matches("<script").count(), 1);
        assert!(HTML.contains("<script src=\"/app.js\"></script>"));
        assert!(HTML.contains("<link rel=\"stylesheet\" href=\"/app.css\">"));
        assert_eq!(HTML.matches("<link").count(), 1);
    }

    #[test]
    fn no_asset_loads_anything_from_outside() {
        for (name, text) in [("html", HTML), ("css", CSS), ("js", JS)] {
            for forbidden in ["http://", "https://", "@import", "url("] {
                assert!(!text.contains(forbidden), "{name} has {forbidden}");
            }
        }
    }

    #[test]
    fn dynamic_strings_only_reach_the_dom_as_text() {
        // innerHTML is used once, for the server's escaped SVG charts.
        assert_eq!(JS.matches("innerHTML").count(), 1);
        assert!(JS.contains("charts.innerHTML = data.charts"));
    }

    fn directive(name: &str) -> &'static str {
        CONTENT_SECURITY_POLICY
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with(name))
            .unwrap_or_else(|| panic!("no {name} directive"))
    }

    #[test]
    fn the_policy_allows_no_inline_script_or_style_and_no_other_origin() {
        assert_eq!(directive("script-src"), "script-src 'self'");
        assert_eq!(directive("style-src"), "style-src 'self'");
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
        assert_eq!(directive("default-src"), "default-src 'none'");
        assert_eq!(directive("frame-ancestors"), "frame-ancestors 'none'");
        assert_eq!(directive("base-uri"), "base-uri 'none'");
        assert_eq!(directive("form-action"), "form-action 'none'");
    }
}
