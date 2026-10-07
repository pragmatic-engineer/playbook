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
<div class="tabs" id="tabs" role="tablist" aria-label="Dashboard sections"></div>
<div class="period" id="period"></div>
<div class="muted" id="status">Loading…</div>
<noscript><p>This page needs JavaScript to load its data.</p></noscript>
<div id="panel" role="tabpanel" tabindex="0"></div>
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
.tabs { display:flex; gap:.25rem; margin:1rem 0 0; border-bottom:1px solid var(--line); overflow-x:auto; }
.tabs button { min-height:2.25rem; padding:.35rem .9rem; border:0; border-bottom:3px solid transparent; background:transparent; color:var(--muted); font:inherit; cursor:pointer; white-space:nowrap; }
.tabs button[aria-selected="true"] { color:var(--fg); border-bottom-color:var(--bar); font-weight:600; }
.tabs button:focus-visible, th button:focus-visible, td button:focus-visible { outline:2px solid var(--bar); outline-offset:-2px; }
th button, td button.link { padding:0; border:0; background:transparent; color:inherit; font:inherit; font-weight:inherit; cursor:pointer; text-align:inherit; }
td button.link { text-decoration:underline; }
tr.selected td { background:var(--line); }
.hint { margin:1rem 0; }
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
const TABS = [["overview","Overview"],["breakdowns","Breakdowns"],["sessions","Sessions"]];
const TAB_PATH = { overview: "/api/data", breakdowns: "/api/data", sessions: "/api/sessions" };
const TAB_KEY = "playbook-usage-tab";
const TABLES = [["day","By day (UTC)"],["week","By week (UTC, starting Monday)"],["model","By model"],["repo","By repo"],["branch","By branch"],["effort","By effort"],["account","By account"],["agent","By agent"]];
const NUMBERS = [["messages","Messages"],["input_tokens","Input"],["output_tokens","Output"],["cache_creation_tokens","Cache write"],["cache_read_tokens","Cache read"],["cost_usd","Cost (USD)"]];
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

function readTab() {
  let saved = "";
  try { saved = localStorage.getItem(TAB_KEY) || ""; } catch (e) {}
  return TABS.some(t => t[0] === saved) ? saved : "overview";
}
let tab = readTab();

function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function fmt(key, value) {
  return key === "cost_usd" ? Number(value).toFixed(4) : Number(value).toLocaleString();
}
function pad(n) { return String(n).padStart(2, "0"); }
function fmtTime(seconds) {
  const d = new Date(seconds * 1000);
  return d.getUTCFullYear() + "-" + pad(d.getUTCMonth() + 1) + "-" + pad(d.getUTCDate()) + " " + pad(d.getUTCHours()) + ":" + pad(d.getUTCMinutes()) + ":" + pad(d.getUTCSeconds());
}
function fmtDuration(seconds) {
  const h = Math.floor(seconds / 3600), m = Math.floor(seconds % 3600 / 60), s = seconds % 60;
  if (h > 0) return h + "h " + pad(m) + "m";
  return m > 0 ? m + "m " + pad(s) + "s" : s + "s";
}
// The only place markup is parsed: server-rendered SVG, labels escaped there.
function drawCharts(box, markup) {
  box.innerHTML = markup;
  box.querySelectorAll("svg").forEach(svg => {
    const wrap = el("div", undefined, "chart-wrap");
    svg.replaceWith(wrap);
    wrap.appendChild(svg);
  });
}

const sorts = {};
function compare(a, b) {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b), undefined, { numeric: true });
}
// A column is { key, label, text?, show? }. Headers sort the rows client side.
function table(id, title, columns, rows, onPick, selected) {
  const wrap = el("div");
  if (title) wrap.appendChild(el("h2", title));
  const scroll = el("div", undefined, "scroll");
  wrap.appendChild(scroll);
  function draw(focusKey) {
    const sort = sorts[id];
    const ordered = rows.slice();
    if (sort) ordered.sort((a, b) => compare(a[sort.key], b[sort.key]) * sort.dir);
    const t = el("table");
    const head = el("tr");
    columns.forEach(c => {
      const active = sort && sort.key === c.key;
      const th = el("th");
      th.setAttribute("aria-sort", active ? (sort.dir > 0 ? "ascending" : "descending") : "none");
      const button = el("button", c.label + (active ? (sort.dir > 0 ? " ▲" : " ▼") : ""));
      button.type = "button";
      button.dataset.key = c.key;
      button.addEventListener("click", () => {
        sorts[id] = active ? { key: c.key, dir: -sort.dir } : { key: c.key, dir: c.text ? 1 : -1 };
        draw(c.key);
      });
      th.appendChild(button);
      head.appendChild(th);
    });
    t.appendChild(head);
    ordered.forEach(r => {
      const tr = el("tr");
      columns.forEach((c, i) => {
        const text = c.show ? c.show(r[c.key], r) : c.text ? String(r[c.key]) : fmt(c.key, r[c.key]);
        const td = el("td");
        if (i === 0 && onPick) {
          const link = el("button", text, "link");
          link.type = "button";
          if (selected && selected() === r.id) link.setAttribute("aria-current", "true");
          td.appendChild(link);
        } else {
          td.textContent = text;
        }
        tr.appendChild(td);
      });
      if (onPick) {
        if (selected && selected() === r.id) tr.className = "selected";
        tr.addEventListener("click", () => onPick(r));
      }
      t.appendChild(tr);
    });
    scroll.replaceChildren(t);
    if (focusKey) {
      const again = Array.from(t.querySelectorAll("th button")).find(b => b.dataset.key === focusKey);
      if (again) again.focus();
    }
  }
  draw();
  return wrap;
}
function breakdown(id, title, first, rows) {
  const columns = [{ key: "key", label: first, text: true }].concat(NUMBERS.map(n => ({ key: n[0], label: n[1] })));
  return table(id, title, columns, rows);
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
function renderPeriod(r) {
  document.getElementById("period").textContent = rangeName(r.key) + ": " +
    (r.start ? r.start + " to " + r.end + " (UTC)" : "no usage recorded yet");
}

function renderTabs() {
  const box = document.getElementById("tabs");
  box.replaceChildren();
  TABS.forEach(t => {
    const button = el("button", t[1]);
    button.type = "button";
    button.id = "tab-" + t[0];
    button.setAttribute("role", "tab");
    button.setAttribute("aria-selected", String(t[0] === tab));
    button.setAttribute("aria-controls", "panel");
    button.tabIndex = t[0] === tab ? 0 : -1;
    button.addEventListener("click", () => selectTab(t[0]));
    box.appendChild(button);
  });
  document.getElementById("panel").setAttribute("aria-labelledby", "tab-" + tab);
}
function selectTab(key, focus) {
  if (key !== tab) {
    tab = key;
    try { localStorage.setItem(TAB_KEY, tab); } catch (e) {}
    sessionsView = null;
    document.getElementById("panel").replaceChildren();
    document.getElementById("status").textContent = "Loading…";
    renderTabs();
    refresh();
  }
  if (focus) document.getElementById("tabs").querySelector('[aria-selected="true"]').focus();
}
function onTabKey(event) {
  const at = TABS.findIndex(t => t[0] === tab);
  const last = TABS.length - 1;
  const next = { ArrowRight: at === last ? 0 : at + 1, ArrowLeft: at === 0 ? last : at - 1, Home: 0, End: last }[event.key];
  if (next === undefined) return;
  event.preventDefault();
  selectTab(TABS[next][0], true);
}

function renderOverview(data) {
  renderPeriod(data.range);
  const totals = el("div", undefined, "totals");
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
  const charts = el("div", undefined, "charts");
  drawCharts(charts, data.charts.cost_by_day + data.charts.cost_by_model);
  document.getElementById("panel").replaceChildren(totals, charts);
}
function renderBreakdowns(data) {
  renderPeriod(data.range);
  const box = el("div");
  TABLES.forEach(t => box.appendChild(breakdown(t[0], t[1], t[0], data.groups[t[0]])));
  const counts = (id, title, first, rows) => table(id, title, [{ key: "name", label: first, text: true }, { key: "count", label: first === "skill" ? "Uses" : "Dispatches" }], rows);
  if (data.skills.length) box.appendChild(counts("skills", "Skills", "skill", data.skills));
  if (data.agents.length) box.appendChild(counts("agents", "Agents", "agent", data.agents));
  document.getElementById("panel").replaceChildren(box);
}

let sessionsView = null;
let selectedId = "";
const SESSION_COLUMNS = [
  { key: "start", label: "Start (UTC)", show: v => fmtTime(v) },
  { key: "duration", label: "Duration", show: v => fmtDuration(v) },
  { key: "repo", label: "Repo", text: true },
  { key: "branch", label: "Branch", text: true },
  { key: "agent", label: "Agent", text: true },
  { key: "model", label: "Model", text: true, show: (v, r) => r.models.length > 1 ? v + " +" + (r.models.length - 1) : v },
  { key: "messages", label: "Messages" },
  { key: "tokens", label: "Tokens" },
  { key: "cost_usd", label: "Cost (USD)" },
];
function renderSessions(data) {
  renderPeriod(data.range);
  if (!sessionsView) {
    sessionsView = { list: el("div"), detail: el("div") };
    sessionsView.detail.appendChild(el("p", "Select a session to see its messages.", "muted hint"));
    document.getElementById("panel").replaceChildren(sessionsView.list, sessionsView.detail);
    if (selectedId) loadDetail(selectedId);
  }
  const list = sessionsView.list;
  list.replaceChildren();
  if (!data.sessions.length) {
    list.appendChild(el("p", "No sessions in this range.", "muted hint"));
    return;
  }
  const note = data.more > 0
    ? "Showing the newest " + data.sessions.length + " of " + data.total + " sessions."
    : data.total + " sessions, newest first.";
  list.appendChild(el("p", note, "muted hint"));
  list.appendChild(table("sessions", "", SESSION_COLUMNS, data.sessions, pick => {
    selectedId = pick.id;
    renderSessions(data);
    loadDetail(pick.id);
  }, () => selectedId));
}
async function loadDetail(id) {
  const detail = sessionsView && sessionsView.detail;
  if (!detail) return;
  detail.replaceChildren(el("p", "Loading session…", "muted hint"));
  try {
    const data = await getJson("/api/session?id=" + encodeURIComponent(id));
    if (id !== selectedId || !sessionsView || sessionsView.detail !== detail) return;
    const box = el("div");
    box.appendChild(el("h2", "Session " + id));
    box.appendChild(el("p", [data.repo, data.branch, data.agent].filter(Boolean).join(" · "), "muted"));
    const chart = el("div", undefined, "charts");
    drawCharts(chart, data.chart);
    box.appendChild(chart);
    if (data.omitted > 0) box.appendChild(el("p", "Showing the newest " + data.messages.length + " of " + data.total + " messages.", "muted hint"));
    box.appendChild(table("timeline", "Messages", [
      { key: "time", label: "Time (UTC)", show: v => fmtTime(v) },
      { key: "model", label: "Model", text: true },
      { key: "tokens", label: "Tokens" },
      { key: "cost_usd", label: "Cost (USD)" },
    ], data.messages));
    detail.replaceChildren(box);
  } catch (error) {
    if (id !== selectedId || !sessionsView || sessionsView.detail !== detail) return;
    if (error.unauthorized) { stopPolling(document.getElementById("status"), "The session token was rejected. " + NEED_LINK); return; }
    detail.replaceChildren(el("p", "Could not load the session: " + error.message, "muted hint"));
  }
}

const RENDER = { overview: renderOverview, breakdowns: renderBreakdowns, sessions: renderSessions };
let timer = null;
let inFlight = 0;
function stopPolling(status, message) {
  if (timer !== null) clearInterval(timer);
  timer = null;
  status.textContent = message;
}
async function getJson(path) {
  const response = await fetch(path, { headers: { 'X-Playbook-Token': TOKEN } });
  if (response.status === 401) {
    const rejected = new Error("the session token was rejected");
    rejected.unauthorized = true;
    throw rejected;
  }
  if (!response.ok) throw new Error("HTTP " + response.status);
  return response.json();
}
// Only the active tab's data is fetched. An answer is dropped when the
// viewer switched tab or range after asking for it.
async function refresh() {
  const status = document.getElementById("status");
  if (!TOKEN) { stopPolling(status, "No session token. " + NEED_LINK); return; }
  const asked = range;
  const askedTab = tab;
  const stale = () => asked !== range || askedTab !== tab;
  inFlight++;
  try {
    const data = await getJson(TAB_PATH[askedTab] + '?range=' + encodeURIComponent(asked));
    if (stale()) return;
    RENDER[askedTab](data);
    status.textContent = "Updated " + new Date().toLocaleTimeString() + ". Refreshes every " + (REFRESH_MS / 1000) + " seconds.";
  } catch (error) {
    if (error.unauthorized) { stopPolling(status, "The session token was rejected. " + NEED_LINK); return; }
    if (!stale()) status.textContent = "Could not refresh: " + error.message;
  } finally {
    inFlight--;
  }
}
// A slow server must not have its answers dropped by the next poll.
function poll() {
  if (inFlight === 0) refresh();
}
renderRanges();
renderTabs();
document.getElementById("tabs").addEventListener("keydown", onTabKey);
refresh();
timer = setInterval(poll, REFRESH_MS);
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_polls_with_the_token_at_the_declared_interval() {
        assert!(JS.contains("fetch(path, { headers: { 'X-Playbook-Token': TOKEN } })"));
        assert!(JS.contains("getJson(TAB_PATH[askedTab] + '?range=' + encodeURIComponent(asked))"));
        assert!(JS.contains(&format!("const REFRESH_MS = {REFRESH_MS};")));
        assert!(JS.contains("setInterval(poll, REFRESH_MS)"));
        assert_eq!(JS.matches("fetch(").count(), 1, "one place sends the token");
    }

    #[test]
    fn only_the_active_tab_is_fetched_and_each_tab_has_a_path() {
        assert!(JS.contains(
            "{ overview: \"/api/data\", breakdowns: \"/api/data\", sessions: \"/api/sessions\" }"
        ));
        assert!(JS.contains("const TABS = [[\"overview\",\"Overview\"],[\"breakdowns\",\"Breakdowns\"],[\"sessions\",\"Sessions\"]];"));
        assert!(JS.contains("\"/api/session?id=\" + encodeURIComponent(id)"));
        assert_eq!(JS.matches("RENDER[askedTab](data)").count(), 1);
    }

    #[test]
    fn tabs_are_accessible_buttons_with_arrow_keys_and_a_remembered_choice() {
        for needle in [
            "setAttribute(\"role\", \"tab\")",
            "setAttribute(\"aria-selected\"",
            "ArrowRight",
            "ArrowLeft",
            "Home: 0",
            "button.tabIndex = t[0] === tab ? 0 : -1;",
            "try { localStorage.setItem(TAB_KEY, tab); } catch (e) {}",
            "try { saved = localStorage.getItem(TAB_KEY) || \"\"; } catch (e) {}",
        ] {
            assert!(JS.contains(needle), "{needle}");
        }
        assert!(HTML.contains("role=\"tablist\""));
        assert!(HTML.contains("role=\"tabpanel\""));
        assert!(!JS.contains("location.hash =") && !JS.contains("pushState"));
    }

    #[test]
    fn tables_sort_on_header_clicks_with_numbers_compared_as_numbers() {
        assert!(JS.contains("th.setAttribute(\"aria-sort\""));
        assert!(
            JS.contains("if (typeof a === \"number\" && typeof b === \"number\") return a - b;")
        );
        assert!(JS.contains("{ numeric: true }"));
    }

    #[test]
    fn a_poll_never_drops_a_slow_answer_and_a_switch_drops_the_old_one() {
        assert!(JS.contains("if (inFlight === 0) refresh();"));
        assert!(JS.contains("const stale = () => asked !== range || askedTab !== tab;"));
        assert!(JS.contains("if (stale()) return;"));
        assert!(JS.contains("if (!stale()) status.textContent"));
        assert!(!JS.contains("latest"), "polls must not invalidate answers");
    }

    #[test]
    fn a_bad_or_unreadable_saved_range_falls_back_to_30d() {
        assert!(
            JS.contains("try { saved = localStorage.getItem(RANGE_KEY) || \"\"; } catch (e) {}")
        );
        assert!(JS.contains("try { localStorage.setItem(RANGE_KEY, range); } catch (e) {}"));
        assert!(JS.contains("? saved : \"30d\";"));
    }

    #[test]
    fn every_element_the_script_looks_up_exists_in_the_page() {
        for chunk in JS.split("getElementById(\"").skip(1) {
            let id = chunk.split('"').next().unwrap();
            assert!(
                HTML.contains(&format!("id=\"{id}\"")),
                "#{id} is missing from the HTML"
            );
        }
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
        assert!(JS.contains("box.innerHTML = markup;"));
        assert!(!JS.contains("outerHTML") && !JS.contains("insertAdjacentHTML"));
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
