// The triggers of the folder of the stage, drawn by a page of the stage:
// read as JSON from `cox://api/triggers`, and again every two seconds, as
// Cox's own tabs follow what they show. They are listed under the event
// they run on, in the order they run in, each with how it last ran, where
// it is kept, and whether it runs in this folder, which its switch turns;
// a drag of a row orders them, and a click on one opens its card. Those
// on a schedule say when they run next, and where their chat works when
// that is elsewhere. In the home, each project's follow, under its name.
// A trigger Cox cannot read says why, and with none, the page says how
// one is written. A click on a row, or on "New trigger", opens the card
// that edits the trigger (see below).

const API = "cox://api/triggers";
const EVERY = 2000;

/// The events a trigger runs on, as the headings name them.
const EVENTS = {
  "turn.end": "Turn end",
  "goal.met": "Goal met",
  "wiki.written": "Wiki written",
  "sketch.end": "Sketch ended",
  "chat.open": "Chat opened",
  "plan.changed": "Plan changed",
  schedule: "Schedule",
};

/// The name of the event `on`, as Cox tells it: `turn.end`, or, for one
/// that says more, as `{schedule: {cron}}`, its key.
const event = (on) => (typeof on === "string" ? on : Object.keys(on)[0]);

const app = document.getElementById("app");
const sheet = document.getElementById("sheet");
let told = null;
let toldText = null;
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
  if (t.by === "this") return `tell: ${first}${when}`;
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
  const make = `<span class="button" data-do="new" title="A trigger of ${esc(told.folders[0].folder)}">New trigger</span>`;
  const all = told.folders.flatMap((f) => f.triggers);
  const off = all.filter((t) => !t.enabled).length;
  const said = `${all.length === 1 ? "1 trigger" : `${all.length || "No"} triggers`}${off ? ` · ${off} off` : ""}`;
  const where = told.folders[0].folder;
  app.innerHTML = `<div class="head"><h1>Triggers</h1><span class="summary">${said}</span><span class="where" title="${esc(where)}">${esc(where)}</span>${make}</div>${unread}${body}`;
  drawStatus();
}

/// How the trigger of the card open last ran, on its top, as Cox tells
/// it again.
function drawStatus() {
  const t = card?.file && find(card.folder, card.file);
  const top = sheet.querySelector(".top");
  if (!t || !top) return;
  const dot = top.querySelector(".dot");
  dot.className = `dot ${t.ran?.state ?? ""}`;
  dot.title = status(t);
  top.querySelector(".status").textContent = status(t);
}

/// The trigger of `file` in `folder`, as Cox last told.
function find(folder, file) {
  return told?.folders.find((f) => f.folder === folder)?.triggers.find((t) => t.file === file);
}

// The card of a trigger, over the list: what its file says, field by
// field, for the human to change, the fields they changed marked until
// they save them, which writes the file; or a new trigger's, empty, which
// saving makes. Beside it, "Discuss" talks the trigger through with an
// agent, which proposes a change of the card whole, laid on it as the
// human's own would be; and "Try on the last turn" runs the card as it
// stands on what a chat here said last as its turn ended.

/// The fields of a card, as Cox's draft names them.
const FIELDS = ["name", "scope", "on", "pages", "cron", "folder", "changed", "sketch", "by", "allow", "reject", "text"];

/// The card open, while one is: the folder it is shown in, its file, if
/// it has one yet, the draft as saved and as the human changed it, which
/// part of it is open, its talk and how it was tried.
let card = null;

/// `draft` as it would be written: what does not go with its event, or
/// with who runs it, left out.
function sent(draft) {
  const d = { ...draft };
  if (d.on !== "wiki.written") d.pages = [];
  if (d.on === "schedule") {
    d.changed = [];
    d.sketch = null;
    d.by = "new";
  } else {
    d.cron = "";
    d.folder = "";
  }
  if (d.by !== "new") {
    d.allow = [];
    d.reject = [];
  }
  return d;
}

/// The fields of the card the human changed, and did not save.
function changes() {
  if (!card) return new Set();
  const [now, then] = [sent(card.draft), sent(card.saved)];
  return new Set(FIELDS.filter((field) => JSON.stringify(now[field] ?? null) !== JSON.stringify(then[field] ?? null)));
}

/// The fields to mark as changed: none on a new trigger's card, all of
/// whose fields are new.
const marked = () => (card.file ? changes() : new Set());

/// Opens the card of trigger `t`, shown in `folder`, or, without one, a
/// new trigger's.
function openCard(folder, t) {
  const saved = t
    ? structuredClone(t.draft)
    : { name: "", scope: "project", on: "turn.end", pages: [], cron: "", folder: "", changed: [], sketch: null, by: "new", allow: [], reject: [], text: "" };
  card = {
    id: t ? t.file : `new-${Date.now()}`,
    folder,
    file: t?.file ?? null,
    saved,
    draft: structuredClone(saved),
    open: !t || t.by === "shell" ? "settings" : "do",
    talk: null,
    talking: false,
    laid: 0,
    tried: null,
    error: null,
  };
  drawCard();
}

function closeCard() {
  if (!card) return;
  fetch(`${API}/end/${encodeURIComponent(card.id)}`).catch(() => {});
  card = null;
  drawCard();
}

/// Chips to choose one of `options`, as `[value, label]`, for `field`.
function chips(field, options) {
  const now = card.draft[field] ?? "";
  return `<div class="chips" data-field="${field}">${options
    .map(([value, label]) => `<span class="chip${value === now ? " chosen" : ""}" data-value="${esc(value)}">${esc(label)}</span>`)
    .join("")}</div>`;
}

/// The patterns of `field`, `allow` or `reject`, each with its ×, and a +
/// to add one.
function patterns(field) {
  const items = card.draft[field]
    .map((item, at) => `<span class="pattern">${esc(item)}<span class="drop" data-field="${field}" data-at="${at}">×</span></span>`)
    .join("");
  return `<div class="patterns">${items}<input class="add" data-add="${field}" placeholder="+" spellcheck="false"></div>`;
}

/// A line of the open settings: its label and what sets it.
const line = (label, field, what) =>
  `<div class="line" data-of="${field}"><span class="label">${label}</span><div class="field">${what}</div></div>`;

/// What the event is called, the one who runs it, and how sketches go.
const EVENT_NAMES = Object.entries(EVENTS);
const WHO = [["shell", "Shell"], ["this", "This chat's agent"], ["new", "A new agent"]];
const SKETCHES = [["", "Any"], ["skip", "Skip"], ["only", "Only"]];

function settingsOpen() {
  const d = card.draft;
  const when = [line("Event", "on", chips("on", EVENT_NAMES))];
  if (d.on === "schedule") {
    when.push(line("Cron", "cron", `<input data-input="cron" class="mono" value="${esc(d.cron)}" placeholder="0 9 * * *" spellcheck="false">`));
    when.push(line("Folder", "folder", `<input data-input="folder" class="mono" value="${esc(d.folder)}" placeholder="The home" spellcheck="false">`));
  } else {
    if (d.on === "wiki.written") {
      when.push(line("Pages", "pages", `<input data-input="pages" class="mono" value="${esc(d.pages.join(" "))}" placeholder="Any" spellcheck="false">`));
    }
    when.push(line("Changed", "changed", `<input data-input="changed" class="mono" value="${esc(d.changed.join(" "))}" placeholder="Anything" spellcheck="false">`));
    when.push(line("Sketches", "sketch", chips("sketch", SKETCHES)));
  }
  const by = [
    d.on === "schedule"
      ? line("Who", "by", `<div class="chips"><span class="chip chosen">A new chat</span></div>`)
      : line("Who", "by", chips("by", WHO)),
  ];
  if (d.on === "schedule" || d.by === "new") {
    by.push(line("Allowed", "allow", patterns("allow")));
    by.push(line("Rejected", "reject", patterns("reject")));
  }
  return `<div class="section">WHEN</div>${when.join("")}<div class="section">BY</div>${by.join("")}`;
}

/// The settings folded to a table of what each is, a click opening them.
function settingsFolded(changed) {
  const d = card.draft;
  const mark = (field) => (changed.has(field) ? " changed" : "");
  const codes = (list) => list.map((item) => `<code>${esc(item)}</code>`).join(" ");
  const cell = (label, field, value) => `<span class="label">${label}</span><span class="value${mark(field)}">${value}</span>`;
  const cells = [cell("When", "on", EVENT_NAMES.find(([on]) => on === d.on)?.[1] ?? esc(d.on))];
  if (d.on === "schedule") {
    cells.push(cell("By", "by", "A new chat"));
    cells.push(cell("Cron", "cron", `<code>${esc(d.cron)}</code>`));
    cells.push(cell("Folder", "folder", esc(d.folder || "The home")));
  } else {
    cells.push(cell("By", "by", WHO.find(([by]) => by === d.by)?.[1] ?? esc(d.by)));
    if (d.on === "wiki.written") cells.push(cell("Pages", "pages", d.pages.length ? codes(d.pages) : "Any"));
    cells.push(cell("Changed", "changed", d.changed.length ? codes(d.changed) : "Anything"));
    cells.push(cell("Sketches", "sketch", SKETCHES.find(([value]) => value === (d.sketch ?? ""))[1]));
  }
  if ((d.on === "schedule" || d.by === "new") && d.allow.length) {
    cells.push(`<span class="label">Allowed</span><span class="value wide${mark("allow")}">${codes(d.allow)}</span>`);
  }
  return `<div class="folded settings" data-open="settings">${cells.join("")}<span class="unfold">▸</span></div>`;
}

function doOpen() {
  const command = card.draft.by === "shell" && card.draft.on !== "schedule";
  return `<div class="section">DO</div>
    <textarea data-input="text" class="text${command ? " command" : ""}" spellcheck="false" placeholder="${command ? "The command to run" : card.draft.by === "this" ? "What the chat's agent is told" : "What the agent is asked"}">${esc(card.draft.text)}</textarea>`;
}

function doFolded(changed) {
  const first = card.draft.text.split("\n").find((l) => l.trim()) ?? "";
  return `<div class="section">DO</div>
    <div class="folded do" data-open="do"><span class="first${changed.has("text") ? " changed" : ""}">${esc(first) || "Nothing yet"}</span><span class="unfold">▸</span></div>`;
}

/// How it went when the card was tried.
function triedShown() {
  const tried = card.tried;
  if (!tried) return "";
  const how = { running: "Running…", passed: "Passed", failed: "Found something", broken: "Could not run" }[tried.state] ?? tried.state;
  const said = tried.said ? `<pre class="said">${esc(tried.said)}</pre>` : "";
  return `<div class="section">ON THE LAST TURN</div><div class="tried"><span class="dot ${esc(tried.state)}"></span>${how}</div>${said}`;
}

function drawMain() {
  const main = sheet.querySelector(".main");
  if (!main) return;
  const changed = marked();
  const settings = card.open === "settings" ? settingsOpen() : settingsFolded(changed);
  const what = card.open === "do" ? doOpen() : doFolded(changed);
  main.innerHTML = `${settings}${what}${triedShown()}`;
  marks();
}

/// Marks the open fields the human changed, and says how many on the
/// card's foot.
function marks() {
  const changed = marked();
  for (const field of sheet.querySelectorAll("[data-of]")) {
    field.classList.toggle("changed", changed.has(field.dataset.of));
  }
  sheet.querySelector(".top .title")?.classList.toggle("changed", changed.has("name"));
  sheet.querySelector(".top .where")?.classList.toggle("changed", changed.has("scope"));
  sheet.querySelector("textarea.text")?.classList.toggle("changed", changed.has("text"));
  drawFoot();
}

function drawFoot() {
  const foot = sheet.querySelector(".foot");
  if (!foot) return;
  const n = changes().size;
  const discuss = card.talking ? "" : `<span class="button" data-do="discuss">Discuss</span>`;
  const save = `<span class="button primary" data-do="save">Save</span>`;
  const error = card.error ? `<span class="error">${esc(card.error)}</span>` : "";
  // A schedule runs at its times, on no turn.
  const tryIt = card.draft.on === "schedule" ? "" : `<span class="button" data-do="try">Try on the last turn</span>`;
  if (!card.file) {
    foot.innerHTML = `${error || `<span class="faint">New trigger</span>`}<span class="grow"></span>${discuss}${tryIt}${save}`;
  } else if (n) {
    const unsaved = `<span class="unsaved">${n} unsaved change${n > 1 ? "s" : ""}</span>`;
    foot.innerHTML = `${error || unsaved}<span class="grow"></span><span class="button quiet" data-do="revert">Revert</span>${discuss}${tryIt}${save}`;
  } else {
    foot.innerHTML = `<a href="${esc(`cox://open${card.file}`)}">Open file</a>${error}<span class="grow"></span>${discuss}${save}`;
  }
}

/// The talk beside the card.
function drawTalk() {
  const talk = sheet.querySelector(".talk .said-all");
  if (!talk) return;
  const told = card.talk;
  const agent = (told?.agent || "Agent").toUpperCase();
  const said = (told?.said ?? [])
    .map((s) => {
      const who = s.who === "you" ? "YOU" : agent;
      const laid = s.draft ? `<span class="laid"><span class="dot passed"></span>Changed the card</span>` : "";
      const unread = s.unread ? `<span class="error">What it proposed is no trigger: ${esc(s.unread)}</span>` : "";
      const text = s.text || (s.who === "you" ? "" : "…");
      return `<div class="entry"><span class="who">${esc(who)}</span>${laid}<div class="words">${esc(text)}</div>${unread}</div>`;
    })
    .join("");
  const error = told?.error ? `<div class="error">${esc(told.error)}</div>` : "";
  const none = said ? "" : `<div class="faint">Say what it should do, or what went wrong when it ran.</div>`;
  talk.innerHTML = `${none}${said}${error}`;
  talk.scrollTop = talk.scrollHeight;
}

function drawCard() {
  if (!card) {
    sheet.hidden = true;
    sheet.innerHTML = "";
    return;
  }
  const t = card.file && find(card.folder, card.file);
  const state = t?.ran?.state ?? "";
  const talk = card.talking
    ? `<div class="talk">
        <div class="talk-top"><span>Discuss this trigger</span><span class="grow"></span><span class="close" data-do="undiscuss" title="Close">×</span></div>
        <div class="said-all"></div>
        <textarea class="reply" placeholder="Reply…" rows="2"></textarea>
      </div>`
    : "";
  sheet.innerHTML = `<div class="card${card.talking ? " talking" : ""}" role="dialog">
    <div class="side">
      <div class="top">
        <span class="dot ${esc(state)}" title="${esc(t ? status(t) : "Never ran")}"></span>
        <input class="title" data-input="name" value="${esc(card.draft.name)}" placeholder="Name" spellcheck="false">
        <span class="where" data-do="scope" title="${esc(card.file ?? "")}">${card.draft.scope === "project" ? "Project" : "Global"}<span class="caret">▾</span></span>
        <span class="status">${esc(t ? status(t) : "")}</span>
        <span class="grow"></span>
        ${card.talking ? "" : `<span class="close" data-do="close" title="Close">×</span>`}
      </div>
      <div class="main"></div>
      <div class="foot"></div>
    </div>
    ${talk}
  </div>`;
  sheet.hidden = false;
  drawMain();
  drawTalk();
}

/// Lays on the card what the agent proposed last, once it has replied,
/// its name and where it is kept staying the human's.
function layProposed() {
  if (card.talk?.working) return;
  const said = card.talk?.said ?? [];
  let proposed = null;
  for (; card.laid < said.length; card.laid++) proposed = said[card.laid].draft ?? proposed;
  if (proposed) {
    card.draft = { ...proposed, name: card.draft.name, scope: card.draft.scope };
    drawMain();
  }
}

async function ask(path, what) {
  try {
    const answer = await fetch(`${API}/${path}/${encodeURIComponent(typeof what === "string" ? what : JSON.stringify(what))}`, { cache: "no-store" });
    return answer.ok ? await answer.json() : null;
  } catch {
    return null;
  }
}

async function save() {
  const at = card;
  const answer = await ask("save", { folder: at.folder, file: at.file, draft: sent(at.draft) });
  if (at !== card) return;
  if (!answer) {
    card.error = "Cox did not save it.";
  } else if (answer.error) {
    card.error = answer.error;
  } else {
    card.file = answer.saved;
    card.saved = structuredClone(card.draft);
    card.error = null;
    told = answer;
    toldText = null;
    draw();
    drawCard();
    return;
  }
  drawFoot();
}

/// Follows what goes on beside the card: the talk while the agent
/// replies, and the try while it runs.
async function follow() {
  if (!card) return;
  const at = card;
  if (at.talking && at.talk?.working) {
    const talk = await ask("discussion", at.id);
    if (at === card && talk) {
      card.talk = talk;
      drawTalk();
      layProposed();
    }
  }
  if (at.tried?.state === "running") {
    const tried = await ask("tried", at.id);
    if (at === card && tried) {
      card.tried = tried;
      drawMain();
    }
  }
}

async function say(words) {
  const t = card.file && find(card.folder, card.file);
  const talk = await ask("discuss", {
    id: card.id,
    folder: card.folder,
    file: card.file,
    draft: sent(card.draft),
    ran: t?.ran?.said ?? null,
    tried: card.tried?.state === "running" ? null : card.tried,
    line: words,
  });
  if (talk) {
    card.talk = talk;
    drawTalk();
  }
}

async function tryOn() {
  const tried = await ask("try", { id: card.id, folder: card.folder, file: card.file, draft: sent(card.draft) });
  card.tried = tried ?? { state: "broken", said: "Cox did not try it." };
  if (card.open !== "do" && card.draft.by !== "shell") card.open = "do";
  drawMain();
}

sheet.addEventListener("click", (event) => {
  if (event.target === sheet) {
    if (!changes().size) closeCard();
    return;
  }
  const chip = event.target.closest(".chips[data-field] .chip");
  if (chip) {
    const field = chip.parentElement.dataset.field;
    const value = chip.dataset.value;
    card.draft[field] = field === "sketch" ? value || null : value;
    drawMain();
    return;
  }
  const drop = event.target.closest(".drop");
  if (drop) {
    card.draft[drop.dataset.field].splice(Number(drop.dataset.at), 1);
    drawMain();
    return;
  }
  const folded = event.target.closest("[data-open]");
  if (folded) {
    card.open = folded.dataset.open;
    drawMain();
    sheet.querySelector(card.open === "do" ? "textarea.text" : ".main input")?.focus();
    return;
  }
  const what = event.target.closest("[data-do]")?.dataset.do;
  if (what === "close") closeCard();
  if (what === "scope") {
    card.draft.scope = card.draft.scope === "project" ? "global" : "project";
    drawCard();
  }
  if (what === "revert") {
    card.draft = structuredClone(card.saved);
    card.error = null;
    drawCard();
  }
  if (what === "save") save();
  if (what === "try") tryOn();
  if (what === "discuss") {
    card.talking = true;
    drawCard();
    sheet.querySelector(".reply")?.focus();
  }
  if (what === "undiscuss") {
    card.talking = false;
    drawCard();
  }
});

sheet.addEventListener("input", (event) => {
  const field = event.target.dataset.input;
  if (!field) return;
  const value = event.target.value;
  card.draft[field] = field === "changed" || field === "pages" ? value.split(/\s+/).filter(Boolean) : value;
  card.error = null;
  marks();
});

sheet.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
  const add = event.target.dataset.add;
  if (add) {
    event.preventDefault();
    const value = event.target.value.trim();
    if (value) card.draft[add].push(value);
    drawMain();
    sheet.querySelector(`[data-add="${add}"]`)?.focus();
    return;
  }
  if (event.target.classList.contains("reply")) {
    event.preventDefault();
    const words = event.target.value.trim();
    if (!words || card.talk?.working) return;
    event.target.value = "";
    say(words);
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && card) closeCard();
});

setInterval(follow, 700);

app.addEventListener("click", (event) => {
  if (event.target.closest("[data-do='new']")) {
    openCard(told.folders[0].folder, null);
    return;
  }
  const row = event.target.closest(".row");
  if (!row || event.target.closest(".handle")) return;
  const { folder, file, at } = row.dataset;
  const t = find(folder, file);
  if (!t) return;
  if (event.target.closest(".switch")) {
    change({ folder: at, file, on: !t.enabled });
    return;
  }
  openCard(folder, t);
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
