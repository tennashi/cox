// The triggers of the folder of the stage, drawn by a page of the stage:
// read as JSON from `cox://api/triggers`, and again every two seconds, as
// Cox's own tabs follow what they show. They are listed under the event
// they run on, in the order they run in, each with how it last ran, where
// it is kept, and whether it runs in this folder, which its switch turns;
// a drag of a row orders them, and a click on one opens its card. Those
// on a schedule say when they run next, and where their chat works when
// that is elsewhere. In the home, each project's follow, under its name.
// A trigger Cox cannot read says why, and with none, the page says how
// one is written.

const API = "cox://api/triggers";
const EVERY = 2000;

/// The events a trigger runs on, as the headings name them.
const EVENTS = { "turn.end": "Turn end", schedule: "Schedule" };

/// The name of the event `on`, as Cox tells it: `turn.end`, or, for one
/// that says more, as `{schedule: {cron}}`, its key.
const event = (on) => (typeof on === "string" ? on : Object.keys(on)[0]);

const app = document.getElementById("app");
const sheet = document.getElementById("sheet");
let told = null;
let toldText = null;
/// The trigger whose card is open, by its folder and file.
let carded = null;
/// The row being dragged, while one is.
let dragged = null;

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

async function load(url = API) {
  let text = null;
  try {
    const answer = await fetch(url, { cache: "no-store" });
    text = answer.ok ? await answer.text() : null;
  } catch {
    text = null;
  }
  if (text === toldText) return;
  toldText = text;
  told = text ? JSON.parse(text) : null;
  if (!dragged) draw();
}

/// Asks Cox to change how the triggers are arranged, and draws them so.
function change(what) {
  load(`${API}/set/${encodeURIComponent(JSON.stringify(what))}`);
}

/// How long ago `at`, in seconds since the epoch, was.
function ago(at) {
  const seconds = Math.max(0, told.now - at);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours > 1 ? "s" : ""} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days > 1 ? "s" : ""} ago`;
}

/// How long until `at`, in seconds since the epoch.
function until(at) {
  const seconds = Math.max(0, at - told.now);
  if (seconds < 60) return "now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `in ${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `in ${hours} hour${hours > 1 ? "s" : ""}`;
  const days = Math.round(hours / 24);
  return `in ${days} day${days > 1 ? "s" : ""}`;
}

/// When a trigger on a schedule runs next.
const next = (t) => (t.schedule.due ? "Due" : t.schedule.next ? until(t.schedule.next) : "");

/// How the trigger last ran, in words, for its dot and its card.
function status(t) {
  if (!t.ran) return "Never ran";
  if (t.schedule) return `Ran · ${ago(t.ran.at)}`;
  const how = { passed: "Passed", failed: "Found something", broken: "Could not run" }[t.ran.state] ?? t.ran.state;
  return `${how} · ${ago(t.ran.at)}`;
}

/// What the trigger does, on its row, from the folder it is listed in.
function does(t, folder) {
  const first = t.text.split("\n").find((line) => line.trim()) ?? "";
  if (t.schedule) {
    const where = t.schedule.works === folder ? "" : ` · in ${t.schedule.works}`;
    return `${t.schedule.every}${where}`;
  }
  const when = t.changed ? " · when files changed" : "";
  if (t.by === "shell") return `run: ${first}${when}`;
  return `agent: new${when}`;
}

const dot = (t) => `<span class="dot ${esc(t.ran?.state ?? "")}" title="${esc(status(t))}"></span>`;
const scope = (t) => (t.scope === "project" ? "Project" : "Global");

function row(folder, t) {
  const handle = t.schedule ? `<span class="handle"></span>` : `<span class="handle" title="Drag to reorder">⠿</span>`;
  const coming = t.schedule ? `<span class="next" title="Next run">${esc(next(t))}</span>` : "";
  return `<div class="row${t.enabled ? "" : " off"}" data-folder="${esc(folder)}" data-at="${esc(t.at)}" data-on="${esc(event(t.on[0]))}" data-file="${esc(t.file)}">
    ${handle}
    ${dot(t)}
    <span class="name">${esc(t.name)}</span>
    <span class="does">${esc(does(t, folder))}</span>
    <span class="last">${esc(t.ran ? ago(t.ran.at) : "")}</span>
    ${coming}
    <span class="scope" title="${esc(t.file)}">${scope(t)}</span>
    <span class="switch${t.enabled ? " on" : ""}" title="${t.enabled ? "On" : "Off"}"></span>
    <span class="more">›</span>
  </div>`;
}

function folder(f) {
  const events = {};
  for (const t of f.triggers) {
    (events[event(t.on[0])] ??= []).push(t);
  }
  // The events Cox knows first, in its order.
  const known = Object.keys(EVENTS);
  const place = (on) => (known.includes(on) ? known.indexOf(on) : known.length);
  const sections = Object.entries(events)
    .sort(([one], [other]) => place(one) - place(other))
    .map(([on, ts]) => `<h2>${esc(EVENTS[on] ?? on)}</h2><div class="rows">${ts.map((t) => row(f.folder, t)).join("")}</div>`)
    .join("");
  return sections || `<div class="empty">None.</div>`;
}

function empty() {
  return `<div class="empty">
    <p>No trigger runs here. A trigger is a Markdown file in <code>.cox/triggers</code> here, or in <code>${esc(told.workflow)}</code> for every project:</p>
    <pre>---
on: turn.end
when:
  changed: "**/*.rs"
by: new
---

Read this turn's diff. If anything could be simpler, say where and how.</pre>
    <p>One on a schedule opens a chat at the times its cron says, and goes on its text:</p>
    <pre>---
on:
  schedule:
    cron: "0 9 * * 1-5"
by: new
allow: [fetch:*, search:*]
---

Find out what changed in Rust since {last}.</pre>
  </div>`;
}

function draw() {
  if (!told) {
    app.innerHTML = `<div class="empty">Cox did not answer with the triggers.</div>`;
    return;
  }
  const unread = told.unread.map((why) => `<div class="unread">${esc(why)}</div>`).join("");
  const any = told.folders.some((f) => f.triggers.length);
  let body = "";
  if (!any) {
    body = empty();
  } else if (told.folders.length === 1) {
    body = folder(told.folders[0]);
  } else {
    body = told.folders
      .map((f) => `<div class="folder">${esc(f.name ?? "Home")}<span>${esc(f.folder)}</span></div>${folder(f)}`)
      .join("");
  }
  app.innerHTML = `<h1>Triggers</h1>${unread}${body}`;
  drawCard();
}

/// The trigger of `file` in `folder`, as Cox last told.
function find(folder, file) {
  return told?.folders.find((f) => f.folder === folder)?.triggers.find((t) => t.file === file);
}

function drawCard() {
  const t = carded && find(carded.folder, carded.file);
  if (!t) {
    sheet.hidden = true;
    sheet.innerHTML = "";
    return;
  }
  const codes = (list) => list.map((item) => `<code>${esc(item)}</code>`).join(" ");
  const by = t.schedule ? "A new chat" : t.by === "shell" ? "Shell" : "A new agent";
  const sketches = { skip: "Skip", only: "Only" }[t.sketch] ?? "Any";
  const changed = t.changed ? codes(t.changed) : "Anything";
  const asksYou = t.schedule ? "Nothing: it asks you" : "Reading and tests";
  const allowed = t.by === "new"
    ? `<span class="label">Allowed</span><span class="value wide">${t.allow.length ? codes(t.allow) : asksYou}</span>`
    : "";
  const rejected = t.reject.length ? `<span class="label">Rejected</span><span class="value wide">${codes(t.reject)}</span>` : "";
  const when = t.schedule
    ? `<span class="label">Runs</span><span class="value" title="${esc(t.schedule.cron)}">${esc(t.schedule.every)} <code>${esc(t.schedule.cron)}</code></span>
        <span class="label">Next</span><span class="value">${esc(next(t))}</span>
        <span class="label">Works in</span><span class="value wide">${esc(t.schedule.works)}</span>`
    : `<span class="label">Changed</span><span class="value">${changed}</span>
        <span class="label">Sketches</span><span class="value">${sketches}</span>`;
  const said = t.ran?.said ? `<div class="section">LAST RUN</div><pre class="said">${esc(t.ran.said)}</pre>` : "";
  sheet.innerHTML = `<div class="card" role="dialog">
    <div class="top">
      ${dot(t)}
      <span class="title">${esc(t.name)}</span>
      <span class="where" title="${esc(t.file)}">${scope(t)}</span>
      <span class="status">${esc(status(t))}</span>
      <span class="close" title="Close">×</span>
    </div>
    <div class="body">
      <div class="settings">
        <span class="label">When</span><span class="value">${esc(t.on.map((on) => EVENTS[event(on)] ?? event(on)).join(", "))}</span>
        <span class="label">By</span><span class="value">${by}</span>
        ${when}
        ${allowed}
        ${rejected}
      </div>
      <div class="section">DO</div>
      <div class="text${t.by === "shell" ? " command" : ""}">${esc(t.text)}</div>
      ${said}
    </div>
    <div class="foot"><a href="${esc(`cox://open${t.file}`)}">Open file</a></div>
  </div>`;
  sheet.hidden = false;
}

function close() {
  carded = null;
  drawCard();
}

app.addEventListener("click", (event) => {
  const row = event.target.closest(".row");
  if (!row || event.target.closest(".handle")) return;
  const { folder, file, at } = row.dataset;
  if (event.target.closest(".switch")) {
    const t = find(folder, file);
    if (t) change({ folder: at, file, on: !t.enabled });
    return;
  }
  carded = { folder, file };
  drawCard();
});

sheet.addEventListener("click", (event) => {
  if (event.target === sheet || event.target.closest(".close")) close();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && carded) close();
});

// Ordering: a row dragged by its handle and let go over another goes
// before it, among the triggers of the same folder and event; those on a
// schedule run at their times, and have no order. It goes by the
// pointer, not by the page's drag and drop, which a page Cox draws
// without a window of its own does not get.
const over = (event) => {
  const row = document.elementFromPoint(event.clientX, event.clientY)?.closest(".row");
  const same = (key) => row.dataset[key] === dragged.dataset[key];
  return row && row !== dragged && same("folder") && same("on") ? row : null;
};

app.addEventListener("pointerdown", (event) => {
  const handle = event.target.closest(".handle");
  if (!handle || !handle.textContent) return;
  event.preventDefault();
  dragged = handle.closest(".row");
  dragged.classList.add("dragging");
  handle.setPointerCapture(event.pointerId);
});

app.addEventListener("pointermove", (event) => {
  if (!dragged) return;
  const row = over(event);
  for (const marked of app.querySelectorAll(".row.over")) {
    if (marked !== row) marked.classList.remove("over");
  }
  row?.classList.add("over");
});

app.addEventListener("pointerup", (event) => {
  if (!dragged) return;
  const row = over(event);
  const moved = dragged;
  dragged = null;
  if (!row) {
    draw();
    return;
  }
  const folder = moved.dataset.folder;
  const files = told.folders
    .find((f) => f.folder === folder)
    .triggers.filter((t) => t.at === folder)
    .map((t) => t.file);
  const order = files.filter((file) => file !== moved.dataset.file);
  order.splice(order.indexOf(row.dataset.file), 0, moved.dataset.file);
  change({ folder, order });
});

load();
setInterval(load, EVERY);
