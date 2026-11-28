// The phone's page: what goes on in the chats coxd runs, the projects
// and chats, and a chat's conversation, as coxd tells them on /events
// and /activity, drawn as the window draws its Activity page, its list
// of chats and its chat's panel. Everything the agents wrote is put in
// as text, never as HTML.
//
// The top of the page has two sides, which the head's two segments
// switch between: "Activity", what goes on, which shows first when the
// page opens, and "Chats", the list of chats, at `#chats`. A chat is at
// `#kept/<number>`; the head's arrow goes back from it to the side of the
// top the human came from, Activity when they came by a notice or the
// chat's own address.

"use strict";

const main = document.getElementById("main");
const compose = {
  form: document.getElementById("compose"),
  line: document.getElementById("line"),
  send: document.getElementById("send"),
  stop: document.getElementById("stop"),
  goal: document.getElementById("goal-form"),
  goalLine: document.getElementById("goal-line"),
  goalButton: document.getElementById("goal-button"),
  changesButton: document.getElementById("changes-button"),
  mapButton: document.getElementById("map-button"),
  sketch: document.getElementById("sketch-form"),
  sketchLabel: document.getElementById("sketch-label"),
  sketchLine: document.getElementById("sketch-line"),
  sketchFrom: document.getElementById("sketch-from"),
  sketchEndings: document.getElementById("sketch-endings"),
  sketchButton: document.getElementById("sketch-button"),
  changesCount: document.getElementById("changes-count"),
  failed: document.getElementById("failed"),
  held: document.getElementById("held"),
  commands: document.getElementById("commands"),
};
// The line the human sent while the agent was at work, which goes once
// the turn ends, as the window's does.
let held = null;
const head = {
  back: document.getElementById("back"),
  switcher: document.getElementById("switcher"),
  icon: document.getElementById("app-icon"),
  title: document.getElementById("title"),
  dot: document.getElementById("dot"),
  waiting: document.getElementById("waiting"),
  seg: document.getElementById("seg"),
  plan: document.getElementById("plan"),
  lost: document.getElementById("lost"),
};

let listing = null;
// The side of the top that shows, or showed last, as the address has it:
// "activity", or "chats" at `#chats`. A chat goes back to it.
let side = location.hash === "#chats" ? "chats" : "activity";
// The pages the chat shown has for the human to try, the mocks of the
// sketch it works in and the pages of its stage served on the Mac's own
// address: { key, asked, told: { pages: [{ name, port, path }] } }, as
// coxd lists them.
let pages = null;
// What changed where the chat shown works, as the window's "Changes" tab
// shows it: { key, asked, told: { since, files } or { error } }, as coxd
// tells it.
let changes = null;
// What the human may do of sketches in the chat shown, as the window's
// menu offers it: { key, asked, told: { start } or { endings } or {} },
// as coxd tells it.
let sketching = null;
// What a sketch the human starts beside the chat starts from, "project"
// or "empty", as the input's chip has it.
let sketchFrom = "project";
let chat = null; // { key, entries, from, state, steers, title, goal, settings, commands }
let events = null;
let drawing = false;
// What the human opened of the conversation, folded otherwise, as the
// window keeps it: the work at its first entry, a step, a line.
const opened = new Set();
// What the tool calls and the answered permission requests show, by
// their places in the conversation shown, once coxd told it: the
// conversation comes without it, and the page asks for what one shows as
// the human opens it.
const contents = new Map();
// How many places of the conversation are drawn, the last: a long chat's
// earlier ones are drawn as the human asks for them, as many again each
// time.
const PLACES = 40;
let drawnPlaces = PLACES;
// Whether the human opened the plan under the head.
let planOpen = false;
// How many turns toward its goal a chat takes on a line of the human's,
// as the window's bar counts them.
const GOAL_TURNS = 10;

// An element of `tag`, with `props` set and `children` in it: strings as
// text, nodes as they are, lists of them, however deep, one after
// another.
function el(tag, props, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(props || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (name === "class") node.className = value;
    else if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
    else node.setAttribute(name, value === true ? "" : value);
  }
  for (const child of children.flat(Infinity)) {
    if (child === undefined || child === null || child === false) continue;
    node.append(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
}

// The window's icons, as its own SVGs draw them, in the color of the text
// around them.
const ICONS = {
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  map: "M3 6l6-3 6 3 6-3v15l-6 3-6-3-6 3zM9 3v15M15 6v15",
};
function icon(name, size) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [key, value] of Object.entries({
    width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
    "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true",
  })) svg.setAttribute(key, value);
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", ICONS[name]);
  svg.append(path);
  return svg;
}

// Posts `body` to coxd at `path`, as JSON, from the page itself.
async function post(path, body) {
  const answer = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Cox": "phone" },
    body: JSON.stringify(body),
  });
  if (!answer.ok) throw new Error(await answer.text());
}

// Does `message` in coxd, as the window would: a ToDaemon, as serde
// writes it, in the chat shown. Says so under the input when coxd would
// not.
async function act(message) {
  compose.failed.hidden = true;
  try {
    await post(chat ? `/act?kept=${chat.kept}` : "/act", message);
    return true;
  } catch (error) {
    compose.failed.textContent = `Not done: ${error.message}`;
    compose.failed.hidden = false;
    return false;
  }
}

// What the page measures of itself, for coxd to send with what it
// measures of its own: how long a cold start and the opening of a chat
// take, how long a draw takes and when the browser has painted it, how
// long an event takes to parse, and how late the page's timers fire.
// Only while coxd tells the page to, as `data-telemetry` on its <html>:
// a page not told measures nothing and posts nothing.
//
// The page posts what it measured to /telemetry every few seconds, and
// as it is put away, as `{"m": [[name, value, label], ...]}`: the name
// coxd's table lists the instrument by for the page, the seconds, and
// the value of its label, or "" for none. It uses only what Safari on an
// iPhone has. It posts with `fetch`, kept alive past the page, since
// coxd takes a POST only with the page's own header, which a beacon
// cannot carry.
const telemetry = (() => {
  const on = document.documentElement.hasAttribute("data-telemetry");
  // The most measurements kept until they are posted: fewer than coxd
  // reads of one report.
  const MOST = 400;
  const EVERY = 5000;
  let queue = [];
  const record = (name, seconds, label = "") => {
    if (queue.length < MOST) queue.push([name, seconds, label]);
  };
  // How late a timer of the page's thread fires: the most since the last
  // report, while the page shows.
  const TICK = 200;
  let due = performance.now() + TICK;
  let late = 0;
  const send = () => {
    if (document.visibilityState === "visible") record("lag", late / 1000);
    late = 0;
    if (!queue.length) return;
    const body = JSON.stringify({ m: queue });
    queue = [];
    fetch("/telemetry", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Cox": "phone" },
      body,
      keepalive: true,
    }).catch(() => {});
  };
  if (on) {
    setInterval(() => {
      const now = performance.now();
      if (document.visibilityState === "visible") late = Math.max(late, now - due);
      due = now + TICK;
    }, TICK);
    setInterval(send, EVERY);
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") send();
    });
    window.addEventListener("pagehide", send);
    // The browser tells its first contentful paint after the frame, which
    // may be after the first list is drawn, so the page waits to be told.
    // A browser that does not time paints tells none.
    try {
      new PerformanceObserver((list) => {
        for (const paint of list.getEntriesByName("first-contentful-paint")) record("cold", paint.startTime / 1000, "fcp");
      }).observe({ type: "paint", buffered: true });
    } catch (_) {}
  }
  // Calls `then`, with the time, once what was drawn now is on the
  // screen: after the frame that paints it.
  const painted = (then) => requestAnimationFrame(() => setTimeout(() => then(performance.now()), 0));
  // When the first list of chats came, and whether the cold start it
  // ends was told.
  let listedAt = null;
  let coldTold = false;
  // The chat the human went to, until its conversation is drawn: when
  // they did, and when the list and the conversation's first patch came.
  let opening = null;
  const tellColdStart = (drawnAt) => {
    const [navigation] = performance.getEntriesByType("navigation");
    if (navigation) {
      record("cold", navigation.responseStart / 1000, "response");
      if (navigation.domContentLoadedEventEnd > 0) record("cold", navigation.domContentLoadedEventEnd / 1000, "dom");
      record("resource", (navigation.responseEnd - navigation.startTime) / 1000, "document");
    }
    record("cold", listedAt / 1000, "listing");
    record("cold", drawnAt / 1000, "drawn");
    for (const entry of performance.getEntriesByType("resource")) {
      const file = new URL(entry.name).pathname.slice(1);
      if (["app.js", "app.css", "theme.css"].includes(file)) record("resource", entry.duration / 1000, file);
    }
  };
  return {
    // What an event of the stream, `data`, says.
    parsed(data) {
      if (!on) return JSON.parse(data);
      const from = performance.now();
      const told = JSON.parse(data);
      const now = performance.now();
      const kind = told !== null && typeof told === "object" ? Object.keys(told)[0] : undefined;
      record("sse_parse", (now - from) / 1000, String(kind).toLowerCase());
      if (kind === "Listing" && listedAt === null) listedAt = now;
      if (opening && kind === "Listing" && opening.listed === null) opening.listed = now;
      if (opening && kind === "Patch" && opening.patched === null) opening.patched = now;
      return told;
    },
    // The human went to a chat, or away from one, from within the page.
    went(toChat) {
      opening = on && toChat ? { at: performance.now(), listed: null, patched: null } : null;
    },
    // A draw of `view`, "home" or "chat", begun at `from`, is done;
    // `came` says whether it drew the conversation of the chat shown.
    drew(view, from, came) {
      if (!on) return;
      record("draw", (performance.now() - from) / 1000, view);
      painted((now) => record("paint", (now - from) / 1000, view));
      if (!coldTold && listedAt !== null) {
        coldTold = true;
        painted(tellColdStart);
      }
      if (opening && came && opening.patched !== null) {
        const { at, listed, patched } = opening;
        opening = null;
        if (listed !== null) record("open", (listed - at) / 1000, "listing");
        record("open", (patched - at) / 1000, "patch");
        painted((now) => record("open", (now - at) / 1000, "drawn"));
      }
    },
  };
})();

// The chat the address names by the number it is kept by, as `#kept/12`.
function shownKept() {
  const match = /^#kept\/(\d+)$/.exec(location.hash);
  return match ? Number(match[1]) : null;
}

// The key of the chat kept as `kept`, as coxd last listed it.
function keyOf(kept) {
  const row = listing && listing.chats.concat(listing.archived).find((one) => one.kept === kept);
  return row ? row.key : null;
}

// Follows the chats, and the chat the address names, anew, or, `again`,
// as the page followed that chat before, keeping what it holds of it and
// what it drew, as after the stream was lost or the page put away.
function connect(again = false) {
  if (events) events.close();
  const kept = shownKept();
  const keeps = again && chat !== null && chat.kept === kept;
  if (!keeps) {
    chat = kept === null ? null : { kept, key: keyOf(kept), entries: [], from: 0, state: null, steers: false, title: null, goal: null, settings: [], commands: [], came: false };
    contents.clear();
    drawnPlaces = PLACES;
  }
  // Of the entries before those coxd tells first, the page asks for no
  // more of those it read back to already: coxd tells it all from there.
  const holds = keeps && chat.from > 0 ? `&from=${chat.from}` : "";
  const stream = new EventSource(kept === null ? "/events" : `/events?kept=${kept}${holds}`);
  events = stream;
  stream.onopen = () => { head.lost.hidden = true; };
  stream.onerror = () => {
    head.lost.hidden = false;
    // The page follows the chat again itself, rather than the browser,
    // which would ask as it first did, to be told from where the human
    // read back to.
    stream.close();
    setTimeout(() => { if (events === stream) connect(true); }, 3000);
  };
  stream.onmessage = (message) => hear(telemetry.parsed(message.data));
  draw();
}

function hear(told) {
  if (told === null || typeof told !== "object") return;
  if (told.Listing) {
    listing = told.Listing;
    // What goes on changed with the list, and who waits for the human.
    readActivity();
    if (chat) chat.key = keyOf(chat.kept);
    // A sketch opened as a project, or on its way to an end, ends no more.
    if (sketching) sketching.asked = false;
    showOpened();
  } else if (told.Patch && chat && told.Patch.key === chat.key) {
    const patch = told.Patch.patch;
    // The first patch of a long conversation holds all the page is to
    // hold of it: what it held before, as it follows the chat again, may
    // have changed meanwhile.
    if (patch.from !== undefined) {
      chat.entries = [];
      contents.clear();
    }
    chat.entries.length = patch.len;
    for (const [at, entry] of patch.entries) {
      chat.entries[at] = entry;
      contents.delete(at);
    }
    for (const at of contents.keys()) if (at >= patch.len) contents.delete(at);
    // Of a long conversation, coxd tells the last entries first, from
    // `from`, and those before them as the human reads back to them.
    if (patch.from !== undefined) chat.from = patch.from;
    // A turn that ended may have made the sketch's pages, or opened one
    // on the stage.
    if (working(chat.state) && !working(patch.state) && pages) pages.asked = false;
    // And changed the files.
    if (working(chat.state) && !working(patch.state) && changes) changes.asked = false;
    // And ended a sketch.
    if (working(chat.state) && !working(patch.state) && sketching) sketching.asked = false;
    chat.state = patch.state;
    chat.steers = patch.steers;
    chat.title = patch.title;
    chat.goal = patch.goal;
    chat.settings = patch.settings || [];
    chat.commands = patch.commands || [];
    chat.came = true;
    if (held !== null && takesLines(chat)) sendHeld();
  } else {
    return;
  }
  redraw();
}

function redraw() {
  if (!drawing) {
    drawing = true;
    requestAnimationFrame(() => { drawing = false; draw(); });
  }
}

function draw() {
  const from = performance.now();
  drawPush();
  if (chat) drawChat();
  else drawHome();
  drawCommands();
  telemetry.drew(chat ? "chat" : "home", from, chat !== null && chat.came);
}

// How a chat stands, as its dot and in words.
function standing(row) {
  if (!row.open && row.state === "Closed") return { dot: "closed", words: "closed" };
  if (row.state === "Asking") return { dot: "asks", words: "asks you" };
  if (row.unread) return { dot: "unread", words: "turn ended" };
  if (row.state === "Busy" || row.state === "Checking" || row.state === "Stopping") {
    return { dot: "busy", words: row.state === "Checking" ? "checking" : "at work" };
  }
  if (row.state === "Closed") return { dot: "closed", words: "closed" };
  return { dot: "", words: "idle" };
}

// Whether the chat waits for the human, as the Activity counts it under
// "Needs you", so that the head's count, the shelf of those that wait
// and the Activity agree: as coxd told (see `activity.waiting`), or,
// until it did, by the same rule, from the list: it asks; or, unless
// the human put it away, its turn ended unseen, or was cut off and it is
// not at work again, or CI ended unseen on a pull request its agent
// opened. One marked done never does.
function waits(row) {
  if (activity) return activity.waiting.some((one) => one.key === row.key);
  if (row.resolved) return false;
  if (row.state === "Asking") return true;
  if (listing.archived.includes(row)) return false;
  const atWork = working(row.state) || (row.state === "Idle" && (row.background || []).length > 0);
  return row.unread || Boolean(row.cut_off && !atWork) || (row.pulls || []).some((pull) => pull.unseen);
}

// The chats that wait for the human, those put away too, which wait
// while they ask.
function waiting() {
  return listing ? listing.chats.concat(listing.archived).filter(waits) : [];
}

// A goal as the input that sets it reads it: the chat's, as the list
// has it, or as its conversation has it, with the turns it took.
function goalText(goal) {
  if (goal && goal.what) goal = goal.what;
  if (!goal) return null;
  if (goal.Command !== undefined) return `$ ${goal.Command}`;
  return goal.Words;
}

function projectName(index) {
  const project = listing && listing.projects[index];
  if (!project) return "";
  return project.id === null ? "Cox" : project.name;
}

// The square of the project at `index`, as the rail draws it: Cox's own
// by its icon, another by the icon the human gave it, or a folder. Each
// is made once, so that its picture is asked for once.
const squares = new Map();
function square(index) {
  const project = listing.projects[index];
  const id = project.id === null ? "home" : String(project.id);
  let made = squares.get(id);
  if (!made) {
    if (project.id === null) {
      made = el("span", { class: "square home" }, el("img", { src: "/icon.svg", alt: "" }));
    } else {
      const picture = el("img", { src: `/project-icon/${encodeURIComponent(id)}`, alt: "" });
      made = el("span", { class: "square" }, picture);
      picture.addEventListener("error", () => picture.replaceWith(icon("folder", 16)));
    }
    squares.set(id, made);
  }
  made.classList.toggle("missing", Boolean(project.missing));
  return made;
}

// The row of a chat, as the window's list draws it: a dot of how it
// stands, ↳ for one an agent opened, its name, and whether it is a sketch
// and has a goal; in the shelf of those that wait, its project too.
function chatRow(row, opts) {
  const stands = standing(row);
  const sketch = row.setup && row.setup.sketch;
  return el("li", { class: [row.open ? null : "closed", waits(row) ? "waits" : null].filter(Boolean).join(" ") || null },
    el("a", { href: row.kept === null ? null : `#kept/${row.kept}`, title: stands.words },
      opts.child && row.parent !== null ? el("span", { class: "small" }, "↳") : null,
      el("span", { class: `dot ${stands.dot}` }),
      el("span", { class: "name" }, row.name),
      sketch ? el("span", { class: "small" }, "sketch") : null,
      row.goal ? el("span", { class: "small" }, "◎") : null,
      opts.project ? el("span", { class: "project" }, projectName(row.project)) : null));
}

// The chat the human opened from the phone, in the project at `project`,
// or, with `folders`, the folders of the projects coxd listed then, in a
// project it makes, to show once coxd lists it: the one of the project
// it did not list before.
let opening = null;

// Opens a chat in the project at `index`, as the window's list does,
// and shows it once coxd lists it.
async function newChat(button, index) {
  const project = listing.projects[index];
  button.disabled = true;
  opening = { project: index, known: new Set(listing.chats.map((row) => row.key)) };
  if (!(await act({ NewChat: { folder: project.id === null ? null : project.folder } }))) {
    opening = null;
    button.disabled = false;
    say(compose.failed.textContent);
    return;
  }
  setTimeout(() => {
    if (opening && opening.project === index) {
      opening = null;
      button.disabled = false;
    }
  }, 10000);
}

// Makes a project, as the window's + on its rail does, with a chat in
// it, and shows the chat once coxd lists it.
async function newProject(button) {
  button.disabled = true;
  const made = {
    project: null,
    folders: new Set(listing.projects.filter((project) => !project.deleted).map((project) => project.folder)),
    known: new Set(listing.chats.map((row) => row.key)),
  };
  opening = made;
  if (!(await act("NewProjectAside"))) {
    opening = null;
    button.disabled = false;
    say(compose.failed.textContent);
    return;
  }
  setTimeout(() => {
    if (opening === made) {
      opening = null;
      button.disabled = false;
    }
  }, 10000);
}

function showOpened() {
  // A chat opened from the list shows from the list; a sketch, from the
  // chat it was opened beside.
  if (!opening || (chat && chat.key !== opening.from)) return;
  const inProject = (row) => opening.folders
    ? !opening.folders.has(listing.projects[row.project].folder)
    : row.project === opening.project;
  const opened = listing.chats.find((row) => inProject(row) && !opening.known.has(row.key));
  if (!opened) return;
  opening = null;
  location.hash = `#kept/${opened.kept}`;
}

function drawHead(row) {
  head.back.hidden = !chat;
  head.icon.hidden = Boolean(chat);
  head.dot.hidden = !row;
  head.switcher.setAttribute("href", "#");
  head.switcher.classList.toggle("home", !chat);
  head.seg.hidden = Boolean(chat);
  for (const tab of head.seg.children) tab.setAttribute("aria-selected", String(tab.dataset.side === side));
  if (!chat) {
    // On the top, how many chats wait for the human, in every project.
    const count = waiting().length;
    head.title.textContent = "Cox";
    head.waiting.hidden = !count;
    head.waiting.textContent = String(count);
    head.waiting.classList.add("asks");
    head.plan.hidden = true;
    return;
  }
  if (row) {
    const stands = standing({ ...row, state: chat.state || row.state });
    head.title.textContent = row.name;
    head.dot.className = `dot ${stands.dot}`;
    head.dot.title = stands.words;
    // How many of the project's other chats wait, as the window's head
    // counts them.
    const others = listing.chats.filter((one) => one.project === row.project && one.key !== row.key && waits(one));
    head.waiting.hidden = !others.length;
    head.waiting.textContent = `${others.length} waiting`;
    head.waiting.classList.toggle("asks", others.some((one) => one.state === "Asking"));
  } else {
    head.title.textContent = listing ? "No such chat" : "Cox";
    head.waiting.hidden = true;
  }
  drawPlan();
}

// The agent the chat talks to and what its session is set to, as the
// window's pill names them.
function pill(row) {
  const agent = row && listing.agents && listing.agents[row.agent];
  const set = chat.settings.map((setting) => {
    const choice = setting.choices.find((one) => one.id === setting.value);
    return choice ? choice.name : setting.value;
  });
  if (!agent && !set.length) return null;
  const said = [agent, ...set].filter(Boolean).join(" · ");
  if (!chat.settings.length) return el("div", { class: "pills" }, el("span", { class: "pill" }, said));
  return el("div", { class: "pills" },
    el("button", { type: "button", class: "pill", "aria-label": `Session: ${said}`, onclick: () => openSettings(agent) }, said));
}

// What the agent lets the human choose of the chat's session, as the
// window's pill lists it, over the chat: each setting on a row, with what
// it is set to, which a tap opens to what it can be set to, under the
// groups the agent puts them in; a tap on one sets it, while a turn runs
// too, and closes the list.
function openSettings(agent) {
  const close = () => { viewer.remove(); document.body.classList.remove("trying"); };
  const body = el("div", { class: "files" });
  const viewer = el("div", { id: "settings", role: "dialog", "aria-label": "Session" },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": "Back to the chat", onclick: close }, "‹"),
      el("span", { class: "page" }, agent ? `Session · ${agent}` : "Session")),
    body);
  document.getElementById("settings")?.remove();
  document.body.classList.add("trying");
  document.body.append(viewer);
  let open = chat.settings.length === 1 ? chat.settings[0].id : null;
  const draw = () => {
    body.replaceChildren(el("ul", { class: "settings" }, chat.settings.map((setting) => {
      const now = setting.choices.find((one) => one.id === setting.value);
      const opened = open === setting.id;
      const row = el("button", {
        type: "button", class: "setting", "aria-expanded": opened ? "true" : "false",
        onclick: () => { open = opened ? null : setting.id; draw(); },
      },
      el("span", { class: "name" }, setting.name),
      el("span", { class: "value" }, now ? now.name : setting.value),
      el("span", { class: "go" }, opened ? "▾" : "›"));
      if (!opened) return el("li", {}, row);
      const groups = [];
      for (const choice of setting.choices) {
        const group = choice.group || "";
        let last = groups[groups.length - 1];
        if (!last || last.name !== group) groups.push(last = { name: group, choices: [] });
        last.choices.push(choice);
      }
      return el("li", {}, row, groups.map((group) => el("div", { class: "group" },
        group.name ? el("div", { class: "label" }, group.name) : null,
        group.choices.map((choice) => el("button", {
          type: "button",
          class: `choice ${choice.id === setting.value ? "chosen" : ""}`,
          onclick: async () => {
            if (choice.id !== setting.value && !(await act({ Set: { key: chat.key, id: setting.id, value: choice.id } }))) return;
            setting.value = choice.id;
            close();
            redraw();
          },
        },
        el("span", { class: "tick" }, choice.id === setting.value ? "✓" : ""),
        el("span", { class: "words" }, el("span", {}, choice.name),
          choice.description ? el("span", { class: "why" }, choice.description) : null))))));
    })));
  };
  draw();
}

// The projects whose chats marked done the human unfolded, by their id,
// as the window's "Resolved" toggles: folded until they tap it.
const resolvedOpen = new Set();

// "Resolved", after the chats of a project, if any of them were marked
// done, as the window's list folds them away at its end: a tap unfolds
// them, and another folds them again.
function resolvedGroup(project, done) {
  if (!done.length) return null;
  const id = project.id === null ? "home" : String(project.id);
  const open = resolvedOpen.has(id);
  const toggle = () => {
    if (open) resolvedOpen.delete(id);
    else resolvedOpen.add(id);
    draw();
  };
  return el("ul", { class: "chats resolved" },
    el("li", { class: "new" },
      el("button", { type: "button", "aria-expanded": String(open), onclick: toggle },
        `Resolved (${done.length})`, el("span", { class: "fold" }, open ? "▴" : "▾"))),
    open ? done.map((row) => chatRow(row, { child: true })) : null);
}

// The map of the project at `index`, on its shelf: a button that opens it
// over the list. It is asked by the project, not by one of its chats,
// as a chat may work in a folder of its own, a sketch's or a worktree's,
// whose map is of another commit than the project's.
function projectMap(index) {
  const project = listing.projects[index];
  if (project.id === null || project.missing) return null;
  return el("button", {
    type: "button", class: "map", "aria-label": `Map of ${projectName(index)}`,
    onclick: () => openMap({}, `project=${project.id}`),
  }, icon("map", 16), "Map");
}

function drawHome() {
  compose.form.hidden = true;
  compose.goal.hidden = true;
  compose.goalButton.hidden = true;
  compose.changesButton.hidden = true;
  compose.mapButton.hidden = true;
  compose.sketchButton.hidden = true;
  compose.sketch.hidden = true;
  drawHead(null);
  document.title = "Cox";
  if (side === "activity") {
    main.replaceChildren(...activityPage());
    return;
  }
  const shelves = [];
  if (!listing) {
    shelves.push(el("p", { class: "empty" }, "Reaching Cox…"));
  } else {
    const asking = waiting();
    if (asking.length) {
      shelves.push(el("section", { class: "shelf asking" },
        el("div", { class: "shelf-head" }, el("span", { class: "label" }, "Waits for you")),
        el("ul", { class: "chats" }, asking.map((row) => chatRow(row, { project: true })))));
    }
    // The chats of no project first, with no heading, as they work in the
    // home the settings set, apart from the projects, as the window's rail
    // has them first; then the projects, each under its square.
    const shelf = (index, head) => {
      const project = listing.projects[index];
      const rows = listing.chats.filter((row) => row.project === index);
      const done = rows.filter((row) => row.resolved);
      return el("section", { class: project.id === null ? "shelf own" : "shelf" },
        head,
        el("ul", { class: "chats" },
          rows.filter((row) => !row.resolved).map((row) => chatRow(row, { child: true })),
          project.missing ? null : el("li", { class: "new" },
            el("button", { type: "button", onclick: (event) => newChat(event.currentTarget, index) },
              el("span", { class: "plus" }, "+"), "New chat"))),
        resolvedGroup(project, done));
    };
    const own = listing.projects.findIndex((project) => project.id === null);
    if (own >= 0) shelves.push(shelf(own, null));
    // A project put away, or deleted, is off the list, as off the rail.
    const projects = listing.projects.map((_, index) => index)
      .filter((index) => index !== own && !listing.projects[index].archived && !listing.projects[index].deleted);
    shelves.push(el("div", { class: "projects-head" }, el("span", { class: "label" }, "Projects")));
    for (const index of projects) {
      shelves.push(shelf(index, el("div", { class: "shelf-head" },
        square(index), el("span", { class: "name" }, projectName(index)), projectMap(index))));
    }
    // A project of its own, as the + at the foot of the window's rail
    // makes, for the human and the agent to work out what it is to be.
    shelves.push(el("ul", { class: "chats new-project" }, el("li", { class: "new" },
      el("button", { type: "button", onclick: (event) => newProject(event.currentTarget) },
        el("span", { class: "plus" }, "+"), "New project"))));
  }
  main.replaceChildren(...shelves);
}

function drawChat() {
  const row = listing && listing.chats.concat(listing.archived).find((one) => one.key === chat.key);
  drawHead(row);
  document.title = row ? row.name : "Cox";
  compose.form.hidden = !row;
  compose.goalButton.hidden = !row;
  // The home's chats work in no project, which has no map.
  compose.mapButton.hidden = !row || listing.projects[row.project].id === null;
  if (row) findChanges(row);
  drawChangesButton();
  if (row) findSketching(row);
  drawSketchButton();
  if (row) drawCompose(row);
  if (listing && !row) {
    main.replaceChildren(el("p", { class: "empty" }, "This chat is not there any more. ",
      el("a", { href: "#" }, "Back to the chats")));
    return;
  }
  const atEnd = window.innerHeight + window.scrollY >= document.body.scrollHeight - 40;
  const agent = (row && listing.agents && listing.agents[row.agent]) || "Agent";
  const nodes = [pill(row)];
  const goal = goalBar(chat.goal);
  if (goal) nodes.push(goal);
  if (!chat.came) nodes.push(el("p", { class: "empty" }, "Reading the conversation…"));
  const places = conversation(chat.entries, agent);
  const closes = closing(chat.entries, row);
  if (closes) places.push(place(closes, null, null, false));
  // The pages to try, where the human reads on.
  if (row) {
    findPages(row);
    const tried = pageBar();
    if (tried) places.push(place(tried, null, null, false));
  }
  if (working(chat.state)) {
    places.push(place(el("p", { class: "waits" }, el("span", { class: "dot busy" }),
      chat.state === "Checking" ? "Checking the turn…" : `${agent} is at work…`), null, null, false));
  }
  nodes.push(el("div", { class: "conversation" }, places));
  main.replaceChildren(...nodes);
  if (atEnd) window.scrollTo(0, document.body.scrollHeight);
}

// The goal over the conversation, as the window's bar: what it is, and,
// while the agent works toward it, a mark for each turn a line gets.
function goalBar(goal) {
  const text = goalText(goal);
  if (!text) return null;
  const turns = (goal && goal.turns) || 0;
  const target = el("span", { class: "sign" });
  target.style.color = "var(--accent)";
  target.textContent = "◎";
  return el("div", { class: "goal" }, target,
    el("span", { class: "what" }, text.startsWith("$ ") ? el("code", {}, text.slice(2)) : text),
    turns > 0 ? el("span", { class: "turns" }, Array.from({ length: GOAL_TURNS }, (_, at) =>
      el("span", { class: at + 1 < turns ? "failed" : at + 1 === turns ? "now" : null }))) : null,
    turns > 0 ? el("span", { class: "count" }, `${turns} / ${GOAL_TURNS}`) : null);
}

// The plan the agent works to since the human's last line, under the
// head, as the window keeps it in sight over the conversation: its line,
// with the step under way, which a tap opens to its steps.
function drawPlan() {
  const entries = chat.entries;
  const from = lastIndexOf(entries, "human");
  let steps = null;
  for (let at = entries.length - 1; at > from; at--) {
    if (entries[at] && kindOf(entries[at])[0] === "plan") { steps = kindOf(entries[at])[1]; break; }
  }
  head.plan.hidden = !steps;
  if (!steps) return;
  const now = steps.find((step) => step.state === "in_progress") || steps.find((step) => step.state === "pending");
  const summary = el("summary", {},
    el("span", { class: "fold" }, planOpen ? "▾" : "▸"),
    el("span", {}, `≡ Plan ${progress(steps)}`),
    now ? el("span", { class: "now" }, now.text) : null);
  head.plan.open = planOpen;
  head.plan.replaceChildren(summary, stepsList(steps));
}
head.plan.addEventListener("toggle", () => {
  if (planOpen !== head.plan.open) {
    planOpen = head.plan.open;
    redraw();
  }
});

// How far the plan of `steps` has gone, as `2/3`.
function progress(steps) {
  return `${steps.filter((step) => step.state === "completed").length}/${steps.length}`;
}

// Whether the agent of the chat `row` merges what it made when asked:
// one that works in the project, not a sketch's, which ends its own ways.
function merges(row) {
  return !(row.setup && (row.setup.folder || row.setup.sketch));
}

// When the chat waits for the human after the work their last line left
// it, what they close the work on, in one place: what the agent showed,
// what it decided on its own, against the plan it started with, and how
// the checks went as its turn ended, with Merge, which asks the agent,
// in the human's place, to merge it.
function closing(entries, row) {
  if (!row || !chat.came || working(chat.state) || chat.state === "Asking" || (chat.state === "Closed" && !row.again)) return null;
  if (!merges(row)) return null;
  const from = lastIndexOf(entries, "human");
  // Once merged, until the human's next line.
  if (from < 0 || lastIndexOf(entries, "merge") > from) return null;
  const shown = [];
  let drift = null;
  let ended = null;
  const met = [];
  for (const entry of entries.slice(from + 1)) {
    if (!entry) continue;
    const [kind, value] = kindOf(entry);
    if (kind === "media" || kind === "shown" || kind === "map_shown") shown.push([kind, value]);
    else if (kind === "plan_changed") drift = value;
    else if (kind === "checks") ended = value;
    else if (kind === "event" && value.on === "goal.met") met.push(value);
  }
  const decided = drift ? drift.added.length + drift.removed.length : 0;
  const changed = changedFiles();
  if (!shown.length && !decided && !ended && !changed) return null;
  const parts = [];
  if (changed) {
    parts.push(el("div", { class: "part" }, el("h3", {}, "Changed"),
      el("button", { type: "button", class: "changed", onclick: openChanges }, changedSummary(changes.told), el("span", { class: "go" }, "›"))));
  }
  if (shown.length) {
    parts.push(el("div", { class: "part" }, el("h3", {}, "To see"),
      el("div", { class: "shown" }, shown.map(([kind, one]) => {
        if (kind === "shown") {
          return el("button", { type: "button", class: "file", title: one.path, onclick: () => openShown(one) },
            el("span", { class: "icon" }, one.markdown ? "≡" : "‹›"),
            el("span", { class: "name" }, shownName(one)));
        }
        if (kind === "map_shown") {
          return el("button", { type: "button", class: "file", title: mapShownName(one), onclick: () => openMap(one) },
            el("span", { class: "icon" }, icon("map", 18)),
            el("span", { class: "name" }, (one.at || []).at(-1) || mapShownName(one)));
        }
        const source = `/media/${encodeURIComponent(one.file)}`;
        return el("a", { href: source, target: "_blank", rel: "noopener noreferrer", title: one.caption || one.from },
          one.kind === "video" ? el("span", { class: "video" }, "▶") : el("img", { src: source, alt: one.caption || one.from, loading: "lazy" }));
      }))));
  }
  if (decided) {
    parts.push(el("div", { class: "part" }, el("h3", {}, "Decided on its own"), driftList(drift)));
  }
  if (ended) {
    const all = ended.concat(...met.map((event) => event.checks));
    const passed = all.every((check) => check.state === "passed");
    parts.push(el("div", { class: "part" }, el("h3", {}, passed ? "Checks passed" : "Checks"),
      el("div", { class: "chips" }, all.map(checkChip))));
  }
  const answers = el("div", { class: "answers" });
  const button = el("button", { type: "button", class: "first merge" }, "Merge");
  answers.append(button);
  let armed = null;
  button.addEventListener("click", async () => {
    // A second tap merges, so that a tap by mistake does not.
    if (!armed) {
      button.textContent = "Tap again to merge";
      button.classList.add("armed");
      armed = setTimeout(() => {
        armed = null;
        button.textContent = "Merge";
        button.classList.remove("armed");
      }, 4000);
      return;
    }
    clearTimeout(armed);
    button.disabled = true;
    button.textContent = "Asking to merge…";
    if (!(await act({ Merge: { key: chat.key } }))) {
      armed = null;
      button.disabled = false;
      button.textContent = "Merge";
      button.classList.remove("armed");
    }
  });
  return el("div", { class: "card-offer closing" },
    cardHead("✓", "Close the work", "for you to merge"),
    parts,
    answers,
    el("p", { class: "hint" }, "Asks the agent to merge it, as the project merges its work."));
}

// Asks coxd what changed where the chat `row` works, unless it was asked
// since the chat was shown or its turn last ended.
function findChanges(row) {
  if (changes && changes.key === row.key && changes.asked) return;
  changes = { key: row.key, asked: true, told: changes && changes.key === row.key ? changes.told : null };
  const asked = changes;
  fetch(`/changes?kept=${row.kept}`)
    .then((answer) => (answer.ok ? answer.json() : null))
    .catch(() => null)
    .then((told) => {
      if (changes !== asked) return;
      const before = JSON.stringify(asked.told);
      asked.told = told;
      if (JSON.stringify(told) === before) return;
      if (chat && chat.key === asked.key) draw();
      drawChanges();
    });
}

// Asks coxd what the human may do of sketches in the chat of `row`, as
// the window's menu offers it, unless it asked already.
function findSketching(row) {
  if (sketching && sketching.key === row.key && sketching.asked) return;
  sketching = { key: row.key, asked: true, told: sketching && sketching.key === row.key ? sketching.told : null };
  const asked = sketching;
  fetch(`/sketch?kept=${row.kept}`)
    .then((answer) => (answer.ok ? answer.json() : null))
    .catch(() => null)
    .then((told) => {
      if (sketching !== asked) return;
      const before = JSON.stringify(asked.told);
      asked.told = told;
      if (JSON.stringify(told) === before) return;
      if (told && told.start && compose.sketch.hidden) sketchFrom = told.start;
      drawSketchButton();
      if (!compose.sketch.hidden) drawSketch();
    });
}

// What coxd last told of sketches in the chat shown.
function sketchingTold() {
  return sketching && chat && sketching.key === chat.key ? sketching.told : null;
}

// The head's button for sketches, as the window's menu has "Start
// sketch…" and the ways a sketch ends: there while the chat can start one,
// or end.
function drawSketchButton() {
  const told = sketchingTold();
  const starts = Boolean(told && told.start);
  const ends = Boolean(told && told.endings && told.endings.length);
  compose.sketchButton.hidden = !starts && !ends;
  compose.sketchButton.setAttribute("aria-label", ends ? "End the sketch" : "Start a sketch");
  if (!starts && !ends) compose.sketch.hidden = true;
}

// Above the input, as the window's: for a chat, what a sketch beside it is
// to try, and what it starts from, which Enter opens; for a sketch, the
// ways it ends, each tapped twice, as the window's menu offers them.
function drawSketch() {
  const told = sketchingTold() || {};
  const ends = told.endings && told.endings.length ? told.endings : null;
  compose.sketchLine.hidden = Boolean(ends);
  compose.sketchFrom.hidden = Boolean(ends);
  compose.sketchLabel.textContent = ends ? "END THE SKETCH" : "SKETCH · WHAT IT TRIES · ENTER STARTS IT";
  compose.sketchFrom.textContent = sketchFrom === "project" ? "from the project" : "from nothing";
  compose.sketchEndings.replaceChildren(...(ends || []).map(endingButton));
  compose.sketchEndings.hidden = !ends;
}

// A way the sketch ends, as the window's menu offers it: a second tap
// ends it so, since what the sketch made may go with it.
function endingButton(ending) {
  const button = el("button", { type: "button" }, ending.name);
  let armed = null;
  button.addEventListener("click", async () => {
    if (!armed) {
      button.textContent = `Tap again: ${ending.name}`;
      button.classList.add("armed");
      armed = setTimeout(() => {
        armed = null;
        button.textContent = ending.name;
        button.classList.remove("armed");
      }, 4000);
      return;
    }
    clearTimeout(armed);
    button.disabled = true;
    if (await act({ EndSketch: { key: chat.key, ending } })) {
      compose.sketch.hidden = true;
      if (sketching) sketching.asked = false;
    } else {
      armed = null;
      button.disabled = false;
      button.textContent = ending.name;
      button.classList.remove("armed");
    }
  });
  return button;
}

// How many files changed where the chat shown works, as coxd last told.
function changedFiles() {
  const told = changes && chat && changes.key === chat.key && changes.told;
  return told && told.files ? told.files.length : 0;
}

// What changed in all, as `3 files · +40 −12`.
function changedSummary(told) {
  const added = told.files.reduce((sum, file) => sum + file.added, 0);
  const removed = told.files.reduce((sum, file) => sum + file.removed, 0);
  return el("span", { class: "counts" },
    `${told.files.length} ${told.files.length === 1 ? "file" : "files"} · `,
    el("span", { class: "gained" }, `+${added}`), " ",
    el("span", { class: "lost" }, `−${removed}`));
}

// The button at the head that opens what changed, counting the files,
// as the window's bar counts them on "Changes".
function drawChangesButton() {
  const count = changedFiles();
  compose.changesButton.hidden = !count;
  compose.changesCount.textContent = String(count);
}
compose.changesButton.addEventListener("click", () => openChanges());
compose.mapButton.addEventListener("click", () => openMap({}));

// Shows what changed over the chat, file by file and line by line, as
// the window's "Changes" tab does, asking coxd again for what it is now.
function openChanges() {
  if (!chat) return;
  const close = () => {
    viewer.remove();
    document.body.classList.remove("trying");
  };
  const viewer = el("div", { id: "changes", role: "dialog", "aria-label": "Changes" },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": "Back to the chat", onclick: close }, "‹"),
      el("span", { class: "page" }, "Changes")),
    el("div", { class: "files" }));
  document.getElementById("changes")?.remove();
  document.body.classList.add("trying");
  document.body.append(viewer);
  drawChanges();
  if (changes && changes.key === chat.key) changes.asked = false;
  const row = listing && listing.chats.concat(listing.archived).find((one) => one.key === chat.key);
  if (row) findChanges(row);
}

// What changed, in the viewer, if it is open.
function drawChanges() {
  const files = document.querySelector("#changes .files");
  if (!files) return;
  const told = changes && chat && changes.key === chat.key ? changes.told : null;
  if (!told) {
    files.replaceChildren(el("p", { class: "empty" }, "Reading what changed…"));
    return;
  }
  if (told.error) {
    files.replaceChildren(el("p", { class: "empty" }, told.error));
    return;
  }
  if (!told.files.length) {
    files.replaceChildren(el("p", { class: "empty" }, `Nothing changed ${told.since}.`));
    return;
  }
  // A few files open at once; more fold to their heads, for the human
  // to open the one they look for.
  const open = told.files.length <= 3;
  files.replaceChildren(
    el("p", { class: "since" }, changedSummary(told), ` ${told.since}`),
    ...told.files.map((file) => changedFile(file, open)));
}

// A file that changed, as a card: how it changed, its path and the lines
// it gained and lost, over its lines.
function changedFile(file, open) {
  const lines = file.binary ? el("p", { class: "note" }, "A binary file.")
    : file.cut ? el("p", { class: "note" }, "Too long to show here.")
      : el("div", { class: "lines" }, file.lines.map((line) => {
        if (line.kind === "stretch") return el("div", { class: "stretch" }, line.text);
        const sign = line.kind === "added" ? "+" : line.kind === "removed" ? "−" : " ";
        return el("div", { class: line.kind },
          el("span", { class: "number" }, String(line.new ?? line.old ?? "")),
          el("span", { class: "sign" }, sign),
          el("span", { class: "text" }, line.text));
      }));
  return el("details", { class: "file", open },
    el("summary", {},
      el("span", { class: `how how-${file.how}` }, file.how),
      el("span", { class: "path" }, file.was ? `${file.was} → ${file.path}` : file.path),
      el("span", { class: "gained" }, `+${file.added}`),
      el("span", { class: "lost" }, `−${file.removed}`)),
    lines);
}

// Asks coxd for the pages the chat `row` has to try, unless it was asked
// since the chat was shown or its turn last ended.
function findPages(row) {
  if (pages && pages.key === row.key && pages.asked) return;
  pages = { key: row.key, asked: true, told: pages && pages.key === row.key ? pages.told : null };
  const asked = pages;
  fetch(`/pages?kept=${row.kept}`)
    .then((answer) => (answer.ok ? answer.json() : null))
    .catch(() => null)
    .then((told) => {
      if (pages !== asked) return;
      const before = JSON.stringify(asked.told);
      asked.told = told;
      if (JSON.stringify(told) !== before && chat && chat.key === asked.key) draw();
    });
}

// The pages to try, each a chip.
function pageBar() {
  const told = pages && chat && pages.key === chat.key && pages.told;
  if (!told || !told.pages.length) return null;
  return el("div", { class: "tries" },
    el("span", { class: "label" }, "Try it"),
    told.pages.map((page) => el("button", { type: "button", onclick: () => openPage(page) }, page.name)));
}

// Where the phone reaches `page`: at the Mac's name the page came for, at
// the port it is served on of the Mac's own address, which the relay
// passes on, another origin than the page.
function pageAddress(page) {
  return `${location.protocol}//${location.hostname}:${page.port}${page.path}`;
}

// Shows `page` over the chat, in a frame, as the human tries it: it runs
// as it would on its own, but cannot reach the page that frames it.
function openPage(page) {
  const address = pageAddress(page);
  const frame = el("iframe", {
    src: address,
    title: page.name,
    sandbox: "allow-scripts allow-same-origin allow-forms allow-modals allow-popups allow-downloads",
  });
  const close = () => { viewer.remove(); document.body.classList.remove("trying"); };
  const viewer = el("div", { id: "trial", role: "dialog", "aria-label": page.name },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": "Back to the chat", onclick: close }, "‹"),
      el("span", { class: "page" }, page.name),
      el("button", { type: "button", class: "icon-button", "aria-label": "Load again", onclick: () => { frame.src = address; } }, "↻"),
      el("a", { href: address, class: "icon-button", target: "_blank", rel: "noopener noreferrer", "aria-label": "Open on its own" }, "↗")),
    frame);
  document.body.classList.add("trying");
  document.body.append(viewer);
}

function working(state) {
  return state === "Busy" || state === "Checking" || state === "Stopping";
}

// The input under the conversation: the line to send, and Stop while
// the agent is at work. While it is empty, it tells what the chat waits
// for, as the window's does.
function drawCompose(row) {
  const state = chat.state || row.state;
  const busy = working(state);
  compose.stop.hidden = !busy;
  compose.stop.disabled = state === "Stopping";
  compose.line.placeholder = state === "Asking" ? "Answer above first"
    : busy && chat.steers && state === "Busy" ? "Goes into this turn"
    : busy ? "Waits for this turn to end"
      : row.open || row.again ? "Reply" : "This chat cannot start again";
}

// The kind of an entry and what it holds, as serde writes it: a unit
// variant is its name, any other an object of one key.
function kindOf(entry) {
  if (typeof entry === "string") return [entry, null];
  const kind = Object.keys(entry)[0];
  return [kind, entry[kind]];
}

// Work the agent does on its way to a reply, which folds, as the window
// folds it.
const WORK = new Set(["tool_call", "thought", "plan"]);

// Who says an entry, as the window has it: none for Cox's notes, which go
// with whoever spoke last.
function speakerOf(kind) {
  if (kind === "human" || kind === "attached" || kind === "on_step") return "you";
  if (kind === "cox" || kind === "keep" || kind === "checks" || kind === "event") return "cox";
  if (kind === "note" || kind === "compacted" || kind === "aside" || kind === "switched" || kind === "merge") return null;
  return "agent";
}

// One place of the conversation, on the line down its left: `node`, and
// a dot of `who` where they start, named `name` over it, or the amber dot
// of what asks the human.
function place(node, who, name, starts) {
  return el("div", { class: starts ? "place starts" : "place" },
    who ? el("span", { class: `tdot ${who}` }) : null,
    name ? el("div", { class: "label" }, name) : null,
    node);
}

// The conversation, as the window lays it out: each entry, or the work
// the agent did one after another, a place on the line, who speaks named
// where they start. Only the last of its places are drawn, under a button
// that draws more of those before them.
function conversation(entries, current) {
  // The agent speaking, as the chat switched from one to another.
  let agent = current;
  const switched = entries.find((entry) => entry && kindOf(entry)[0] === "switched");
  if (switched) agent = kindOf(switched)[1].from;
  // Each place, to draw: what it holds is made only for those drawn.
  const found = [];
  let speaking = null;
  const asked = askedAt(chat);
  for (let at = 0; at < entries.length; at++) {
    const entry = entries[at];
    if (!entry) continue;
    const [kind, value] = kindOf(entry);
    if (kind === "switched") agent = value.to;
    // Before the entries told in a row, those coxd told of them, for what
    // the page reads, are not drawn alone.
    if (at < chat.from) continue;
    // Two entries of work or more in a row fold into one place, and so
    // does a tool call alone, to show as a step.
    let end = at;
    while (end < entries.length && entries[end] && WORK.has(kindOf(entries[end])[0])) end++;
    const folds = end - at > 1 || kind === "tool_call";
    const speaker = speakerOf(kind);
    const starts = speaker !== null && speaker !== speaking;
    if (speaker !== null) speaking = speaker;
    const name = { you: "YOU", cox: "COX", agent: agent.toUpperCase() }[speaker];
    if (folds) {
      const first = at;
      found.push(() => place(work(first, entries.slice(first, end).map(kindOf), starts ? name : null),
        starts ? speaker : null, null, starts));
      at = end - 1;
      continue;
    }
    const asks = (kind === "permission" || kind === "question") && at === asked;
    const first = at;
    found.push(() => {
      const node = entryNode(first, kind, value, asks);
      if (!node) return null;
      // What asks the human is marked by its amber dot alone.
      if (asks) return place(node, "asking", null, false);
      return place(node, starts ? speaker : null, starts ? name : null, starts);
    });
  }
  const from = Math.max(0, found.length - drawnPlaces);
  const places = found.slice(from).map((made) => made()).filter(Boolean);
  if (from > 0) places.unshift(place(earlier(from), null, null, false));
  else if (chat.from > 0) places.unshift(place(earlier(null), null, null, false));
  return places;
}

// The button over the places drawn, which draws as many again of the
// `left` before them, where the human reads, or, when all of those coxd
// told are drawn, asks it for the entries before them first.
function earlier(left) {
  return el("button", {
    type: "button",
    class: "earlier",
    onclick: async (event) => {
      if (left === null) {
        event.currentTarget.disabled = true;
        event.currentTarget.textContent = "Reading…";
        if (!(await readEarlier())) {
          draw();
          return;
        }
      }
      const below = document.body.scrollHeight - window.scrollY;
      drawnPlaces += PLACES;
      draw();
      window.scrollTo(0, document.body.scrollHeight - below);
    },
  }, left === null ? "Earlier" : `Earlier · ${left} more`);
}

// Asks coxd for the entries of the chat before those it told, and holds
// them, saying whether it could.
async function readEarlier() {
  const reading = chat;
  const told = await fetch(`/entries?kept=${reading.kept}&before=${reading.from}`)
    .then((answer) => (answer.ok ? answer.json() : null))
    .catch(() => null);
  if (!told || chat !== reading) return false;
  for (const [at, entry] of told.entries) reading.entries[at] = entry;
  reading.from = told.from;
  return true;
}

// What the entry at `at`, of `kind`, holding `value`, shows.
function entryNode(at, kind, value, asks) {
  switch (kind) {
    case "human":
      return el("div", { class: "human" }, value);
    case "merge":
      return el("p", { class: "note" }, "You asked the agent to merge what it made.");
    case "cox":
      return el("div", { class: "cox-line" }, value);
    case "attached":
      return el("div", { class: "chips" }, value.map((one) => el("span", { class: "chip" }, el("span", {}, one.label))));
    case "on_step":
      return el("p", { class: "note" }, `on the plan's step: ${value.text}`);
    case "reply":
      return markdown(value);
    case "thought":
      return thought(at, value);
    case "plan":
      return planLine(at, value);
    case "plan_changed":
      return el("div", { class: "plan-changed" }, el("div", { class: "label" }, "Plan changed"), driftList(value));
    case "media":
      return media(value);
    case "shown":
      return shownRow(value);
    case "map_shown":
      return mapShownRow(value);
    case "permission":
      return permission(at, value, asks);
    case "question":
      return question(at, value, asks);
    case "proposal":
      return proposal(value, at);
    case "name_proposal":
      return nameProposal(value, at);
    case "about_proposal":
      return aboutProposal(value, at);
    case "icon_proposal":
      return iconProposal(value, at);
    case "project_proposal":
      return projectProposal(value, at);
    case "keep":
      return keep(value, at);
    case "commits":
      return el("div", { class: "chips" }, value.map((one) =>
        el("span", { class: "chip" }, el("code", {}, one.id.slice(0, 7)),
          el("span", {}, one.amended ? `amended ${one.subject}` : one.subject))));
    case "checks":
      return checks("Turn end", value);
    case "event":
      return checks(value.on.replace(".", " "), value.checks);
    case "note":
    case "compacted":
    case "aside":
      return el("p", { class: "note" }, value);
    case "switched":
      return el("p", { class: "note" }, `Switched from ${value.from} to ${value.to}.`);
    case "login":
      return el("div", { class: "card-ask" },
        cardHead("?", "Log in", value.chosen === null ? "waiting for you" : null),
        value.failed ? el("p", { class: "why" }, value.failed) : null,
        el("ul", { class: "steps-list" }, value.options.map((option, index) =>
          el("li", {}, el("span", { class: "state" }, value.chosen === index ? "✓" : `${index + 1}`), option.name))),
        value.chosen === null ? el("p", { class: "hint" }, "Log in on the Mac.") : null);
    default:
      return null;
  }
}

// The place of the permission request or the question the agent waits
// on the human to answer, while it does, or -1: the agent asks one thing
// at a time, so it is the last asked, wherever notes Cox added since put
// it.
function askedAt(chat) {
  if (chat.state !== "Asking") return -1;
  const at = Math.max(lastIndexOf(chat.entries, "permission"), lastIndexOf(chat.entries, "question"));
  if (at < 0) return -1;
  const [kind, value] = kindOf(chat.entries[at]);
  const open = kind === "question" ? value.answer : value.chosen;
  return open === null || open === undefined ? at : -1;
}

function lastIndexOf(entries, kind) {
  for (let at = entries.length - 1; at >= 0; at--) {
    if (entries[at] && kindOf(entries[at])[0] === kind) return at;
  }
  return -1;
}

// The mark of how a call goes, as the window's: a ring while it runs,
// then a dot, in the accent once done, or in red when it failed.
function mark(result) {
  if (result === null || result === undefined) return el("span", { class: "mark" });
  if (result === "completed") return el("span", { class: "mark done" });
  if (result === "failed") return el("span", { class: "mark failed" });
  return el("span", { class: "mark stopped" });
}

// A call's title, short, as its chip names it in the window: each path
// in it by its last part.
function short(title) {
  return title.split(/\s+/).filter(Boolean).map((word) => {
    const bare = word.replace(/^`+|`+$/g, "");
    const cut = bare.lastIndexOf("/");
    return /^[\p{L}\p{N}/._\-~@+]+$/u.test(bare) && cut >= 0 && cut < bare.length - 1 ? bare.slice(cut + 1) : bare;
  }).join(" ");
}

// Opens or closes what the human opened at `key`, and draws it so.
function flip(key) {
  if (opened.has(key)) opened.delete(key);
  else opened.add(key);
  redraw();
}

// A line that opens, as the window's: ▸ or ▾, `sign` in its color, and
// `what`, with `answer` at its end; or, with nothing to open, the line
// alone.
function line(key, sign, signColor, what, answer, opens) {
  const signNode = el("span", {}, sign);
  if (signColor) signNode.style.color = signColor;
  const parts = [
    el("span", { class: "fold" }, opens ? (opened.has(key) ? "▾" : "▸") : ""),
    signNode,
    el("span", { class: "what" }, what),
    answer ? el("span", { class: "answer" }, answer) : null,
  ];
  if (!opens) return el("div", { class: "line" }, parts);
  return el("button", { type: "button", class: "line", onclick: () => flip(key) }, parts);
}

// The agent's work, one entry after another, under a line that names it
// where it starts, `name`, counts its steps and tells how far its plan
// has gone: closed, each call as a chip, which opens the call's line
// under the chips, and the last thought; open, each entry on its line.
function work(first, group, name) {
  const key = `work-${first}`;
  const open = opened.has(key);
  const calls = [];
  group.forEach(([kind, value], offset) => { if (kind === "tool_call") calls.push([first + offset, value]); });
  const thoughts = group.filter(([kind]) => kind === "thought");
  const plans = group.filter(([kind]) => kind === "plan");
  const told = name ? [name] : [];
  if (calls.length) told.push(`${calls.length} ${calls.length === 1 ? "STEP" : "STEPS"}`);
  else if (thoughts.length) told.push(`${thoughts.length} ${thoughts.length === 1 ? "THOUGHT" : "THOUGHTS"}`);
  if (plans.length) told.push(`PLAN ${progress(plans[plans.length - 1][1])}`);
  const header = el("button", { type: "button", class: "work-head label", onclick: () => flip(key) },
    told.join(" · "), el("span", {}, open ? "▾" : "▸"));
  const body = [];
  if (open) {
    group.forEach(([kind, value], offset) => {
      const at = first + offset;
      if (kind === "tool_call") body.push(call(at, value));
      else if (kind === "thought") body.push(thought(at, value));
      else body.push(planLine(at, value));
    });
  } else {
    if (calls.length) {
      body.push(el("div", { class: "chips" }, calls.map(([at, value]) => {
        const how = value.result === "failed" ? "failed" : value.result === "cancelled" ? "stopped" : "";
        return el("button", {
          type: "button",
          class: `chip ${how} ${opened.has(`call-${at}`) ? "open" : ""}`,
          onclick: () => flip(`call-${at}`),
        }, mark(value.result), el("span", {}, value.about ?? short(value.title)));
      })));
      for (const [at, value] of calls) if (opened.has(`call-${at}`)) body.push(call(at, value));
    }
    const last = group.map(([kind, value], offset) => [kind, first + offset, value]).filter(([kind]) => kind === "thought").pop();
    if (last) body.push(thought(last[1], last[2]));
  }
  return el("div", { class: "work" }, header, body);
}

// A tool call on its line, as the window's: how it went, its title, and
// what it showed, which opens under it, from the start for one that failed.
function call(at, value) {
  const key = `content-${at}`;
  const marks = { completed: ["✓", "var(--accent)"], failed: ["✗", "var(--failed)"], cancelled: ["–", "var(--dim)"] };
  const [sign, color] = marks[value.result] || ["●", "var(--link)"];
  const opens = value.shows > 0;
  if (value.result === "failed" && !opened.has(`seen-${key}`)) {
    opened.add(`seen-${key}`);
    opened.add(key);
  }
  return el("div", { class: "call" },
    line(key, sign, color, value.title, null, opens),
    opens && opened.has(key) ? shownAt(at) : null);
}

// What the entry at `at` shows, as coxd tells it, asked for the first
// time it is drawn open, and a hint while it comes.
function shownAt(at) {
  const known = contents.get(at);
  if (known && known.content) return known.content.map(content).filter(Boolean);
  if (!known) {
    const asked = {};
    contents.set(at, asked);
    const key = chat.key;
    fetch(`/content?kept=${chat.kept}&at=${at}`)
      .then((answer) => (answer.ok ? answer.json() : { content: [] }))
      .catch(() => null)
      .then((told) => {
        if (contents.get(at) !== asked || !chat || chat.key !== key) return;
        // Asked again as it is drawn next, when coxd could not be reached.
        if (!told) contents.delete(at);
        else asked.content = told.content || [];
        redraw();
      });
  }
  return el("p", { class: "hint" }, "Reading…");
}

// What the agent thought, as an aside: its start, in italics, which a tap
// opens to all of it, open while it arrives.
function thought(at, text) {
  const key = `thought-${at}`;
  const arriving = working(chat.state) && at === chat.entries.length - 1;
  const open = opened.has(key) || arriving;
  return el("div", { class: "thought", onclick: () => flip(key) },
    open ? "Thinking" : shorten(firstLine(text), 120),
    open ? el("div", { class: "full" }, text) : null);
}

// The plan the agent worked to, on a line that tells how far it went,
// opening to its steps.
function planLine(at, steps) {
  const key = `plan-${at}`;
  return el("div", { class: "call" },
    line(key, "≡", null, `Plan ${progress(steps)}`, null, true),
    opened.has(key) ? stepsList(steps) : null);
}

function stepsList(steps) {
  const marks = { completed: "✓", in_progress: "●", pending: "○" };
  return el("ul", { class: "steps-list" }, steps.map((step) =>
    el("li", { class: step.state }, el("span", { class: "state" }, marks[step.state] || ""), step.text)));
}

function content(one) {
  const [kind, value] = kindOf(one);
  if (kind === "text") return value.trim() ? el("pre", {}, value) : null;
  if (kind === "diff") {
    return el("div", { class: "diff" },
      el("div", { class: "path" }, value.path),
      value.lines.map((line) => {
        const [how, text] = kindOf(line);
        if (how === "skipped") return el("div", { class: "skipped" }, `⋯ ${text} lines`);
        const sign = how === "added" ? "+ " : how === "removed" ? "- " : "  ";
        return el("div", { class: how }, sign + text);
      }));
  }
  return null;
}

// How the plan differs from the one the human left the agent to: the
// steps it added, in amber, as what it decided on its own, and those it
// removed, dimmer.
function driftList(changed) {
  const steps = [
    ...changed.added.map((step) => el("li", { class: "added" }, el("span", { class: "state" }, "+"), step)),
    ...changed.removed.map((step) => el("li", { class: "removed" }, el("span", { class: "state" }, "−"), step)),
  ];
  if (!steps.length) steps.push(el("li", { class: "removed" }, el("span", { class: "state" }, "="), "The plan is back to the one the line started with."));
  return el("ul", { class: "steps-list" }, steps);
}

// The head of a card that waits for the human: `sign` and `what`, with
// `tag` at its right, in the card's tone.
function cardHead(sign, what, tag) {
  return el("div", { class: "card-head" },
    el("span", { class: "sign" }, sign),
    el("span", { class: "what" }, what),
    tag ? el("span", { class: "tag" }, tag) : null);
}

// A permission request: while the agent waits on it, the amber card with
// what triggers gathered, what it shows and its options; answered, its
// line, which opens to what was asked.
function permission(at, asked, asks) {
  if (asks) {
    const gathering = (asked.gathered || []).some((check) => check.state === "running");
    return el("div", { class: "card-ask" },
      cardHead("?", asked.title, gathering ? "gathering" : "waiting for you"),
      gathered(asked.gathered),
      asked.content.map(content),
      el("div", { class: "answers options" }, asked.options.map((option, index) =>
        el("button", { type: "button", class: index === 0 ? "first" : null, onclick: (event) => choose(event, index) },
          `${index + 1} · ${option}`))));
  }
  const chosen = asked.chosen === null || asked.chosen === undefined ? "no answer" : asked.options[asked.chosen];
  const answer = asked.by_setup ? `${chosen}, as the chat is set up` : chosen;
  const key = `content-${at}`;
  const found = gathered(asked.gathered);
  // What it shows comes with it while it is not answered, and is asked
  // for once it is.
  const shows = asked.content ? asked.content.map(content).filter(Boolean) : null;
  const more = shows ? shows.length > 0 : asked.shows > 0;
  const opens = Boolean(found) || more;
  return el("div", { class: "call" },
    line(key, "?", "var(--asking)", asked.title, answer, opens),
    opens && opened.has(key) ? [found].concat(more ? shows || shownAt(at) : []) : null);
}

// What the human answered of `questions`, as the window's line says it.
function questionSaid(questions, answer) {
  if (answer === null || answer === undefined) return "no answer";
  if (answer === "declined") return "declined";
  const said = questions.map((asked, at) => {
    const picked = answer.given[at] || { chosen: [] };
    const words = picked.chosen.map((index) => asked.options[index] && asked.options[index].label).filter(Boolean);
    if (picked.other) words.push(picked.other);
    return words.length ? words.join(", ") : "skipped";
  });
  return said.length ? said.join("; ") : "answered";
}

// The questions the agent asks: while it waits on them, the amber card of
// each question's options, which a tap answers when it asks one question
// of which the human picks one, and else picks until Send, with a box for
// the human's own words where it takes them, and Decline; answered, its
// line.
function question(at, asked, asks) {
  if (!asks) return line(`question-${at}`, "?", "var(--asking)", asked.message, questionSaid(asked.questions, asked.answer), false);
  const byATap = asked.questions.length === 1 && !asked.questions[0].multiple && asked.questions[0].options.length > 0;
  const picks = asked.questions.map(() => new Set());
  const boxes = [];
  const parts = asked.questions.map((one, at) => {
    const options = one.options.map((option, index) => {
      const button = el("button", { type: "button", class: "choice" },
        el("span", {}, byATap ? `${index + 1} · ${option.label}` : option.label),
        option.description ? el("span", { class: "why" }, option.description) : null);
      button.onclick = (event) => {
        if (byATap) return choose(event, index);
        const picked = picks[at];
        if (picked.has(index)) picked.delete(index);
        else {
          if (!one.multiple) picked.clear();
          picked.add(index);
        }
        for (const [other, node] of [...button.parentNode.querySelectorAll(".choice")].entries()) {
          node.classList.toggle("first", picked.has(other));
        }
      };
      return button;
    });
    const box = one.other ? el("input", { type: "text", placeholder: one.options.length ? "Other…" : "Your answer" }) : null;
    boxes.push(box);
    return el("div", { class: "question" },
      one.header ? el("div", { class: "tag" }, one.header) : null,
      one.text ? el("div", {}, one.text) : null,
      el("div", { class: "answers options column" }, options),
      box);
  });
  const given = () => ({ given: asked.questions.map((_, at) => {
    const words = boxes[at] ? boxes[at].value.trim() : "";
    return words ? { chosen: [...picks[at]].sort(), other: words } : { chosen: [...picks[at]].sort() };
  }) });
  const answers = [["Decline", { AnswerQuestion: { key: chat.key, answer: "declined" } }]];
  const card = el("div", { class: "card-ask" }, cardHead("?", asked.message, "waiting for you"), parts);
  if (!byATap) {
    const send = el("button", { type: "button", class: "first" }, "Send");
    send.onclick = () => act({ AnswerQuestion: { key: chat.key, answer: given() } });
    card.append(el("div", { class: "answers" }, send));
  }
  card.append(buttons(answers));
  return card;
}

// A picture or a video shown in the conversation, with its caption, or
// where it was shown from.
function media(shown) {
  const source = `/media/${encodeURIComponent(shown.file)}`;
  const seen = shown.kind === "video"
    ? el("video", { src: source, controls: true, playsinline: true, preload: "metadata" })
    : el("img", {
      src: source, alt: shown.caption || shown.from, loading: "lazy",
      // Its size, when Cox knows it, keeps its room while it loads.
      ...(shown.size ? { width: shown.size[0], height: shown.size[1] } : {}),
    });
  return el("figure", { class: "media" }, seen, el("figcaption", {}, shown.caption || shown.from));
}

// What a file the agent showed is called, with its lines, if it named
// them, as `main.rs:12-20`.
function shownName(shown) {
  const name = shown.path.split("/").pop() || shown.path;
  if (!shown.lines) return name;
  const [first, last] = shown.lines;
  return first === last ? `${name}:${first}` : `${name}:${first}-${last}`;
}

// A file the agent showed the human on the stage, which the phone has
// none of: a row that opens it over the chat, as it was then.
function shownRow(shown) {
  return el("button", { type: "button", class: "shown-file", onclick: () => openShown(shown) },
    el("span", { class: "icon" }, shown.markdown ? "≡" : "‹›"),
    el("span", { class: "path" }, shown.path),
    shown.lines ? el("span", { class: "lines-at" }, shownName(shown).split(":").pop()) : null,
    el("span", { class: "go" }, "›"));
}

// Shows the file the agent showed over the chat, as it was then: drawn
// as Markdown, or as code with the numbers of its lines, the lines it
// named marked, and scrolled to.
function openShown(shown) {
  const close = () => { viewer.remove(); document.body.classList.remove("trying"); };
  const body = el("div", { class: "files" }, el("p", { class: "empty" }, "Reading the file…"));
  const viewer = el("div", { id: "reading", role: "dialog", "aria-label": shown.path },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": "Back to the chat", onclick: close }, "‹"),
      el("span", { class: "page" }, shown.path)),
    body);
  document.getElementById("reading")?.remove();
  document.body.classList.add("trying");
  document.body.append(viewer);
  fetch(`/media/${encodeURIComponent(shown.file)}`)
    .then((answer) => (answer.ok ? answer.text() : Promise.reject(new Error(answer.statusText))))
    .then((text) => {
      if (shown.markdown) {
        body.replaceChildren(el("div", { class: "document" }, markdown(text)));
        return;
      }
      const [first, last] = shown.lines || [0, -1];
      const rows = text.replace(/\n$/, "").split("\n").map((line, index) => {
        const number = index + 1;
        const marked = number >= first && number <= last;
        return el("div", { class: marked ? "marked" : null },
          el("span", { class: "number" }, String(number)),
          el("span", { class: "text" }, line));
      });
      body.replaceChildren(el("div", { class: "code" }, rows));
      const at = body.querySelector(".marked");
      if (at) at.scrollIntoView({ block: "center" });
    })
    .catch((error) => body.replaceChildren(el("p", { class: "empty" }, `Cannot read it: ${error.message}`)));
}

// What a map the agent showed is called: the map, by its name, unless
// the project's first.
function mapShownName(shown) {
  return shown.map ? `Map ${shown.map}` : "Map";
}

// Where the phone shows the Map page, beside its own, open where the
// agent showed the map, as the window's stage opens it: the map's name and
// `:`, unless the project's first, then the ids of the nodes from the top
// down, each encoded; of what `whose` names, in the page's query: where
// a chat works, as `kept=<n>`, or a project, as `project=<n>`.
function mapAddress(shown, whose) {
  const at = (shown.at || []).map(encodeURIComponent).join("/");
  const hash = shown.map ? `${encodeURIComponent(shown.map)}:${at}` : at;
  return `/map/?${whose}#${hash}`;
}

// A map of the project the agent showed on the stage: a row that opens it
// over the chat, where the agent showed it, with the node it opened at.
function mapShownRow(shown) {
  const last = (shown.at || []).at(-1);
  return el("button", { type: "button", class: "shown-file map-shown", onclick: () => openMap(shown) },
    el("span", { class: "icon" }, icon("map", 16)),
    el("span", { class: "name" }, mapShownName(shown)),
    el("span", { class: "path" }, last || ""),
    el("span", { class: "go" }, "›"));
}

// Shows the Map page over the chat, in a frame, where the agent showed the
// map, as it is drawn now: it is the window's own page, read beside the
// phone's, whose taps, drags and pinches move through the map. With
// nothing shown, as from the head's button or a project's on the list, it
// opens at the first map, of what `whose` names (see `mapAddress`): where
// the chat shown works, unless given.
function openMap(shown, whose = `kept=${chat.kept}`) {
  const close = () => { viewer.remove(); document.body.classList.remove("trying"); };
  const name = mapShownName(shown);
  const viewer = el("div", { id: "map", role: "dialog", "aria-label": name },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": chat ? "Back to the chat" : "Back to the chats", onclick: close }, "‹"),
      el("span", { class: "page" }, name)),
    el("iframe", { src: mapAddress(shown, whose), title: name }));
  document.getElementById("map")?.remove();
  document.body.classList.add("trying");
  document.body.append(viewer);
}

// What the triggers gathered for the human to answer with, each by its
// name, with its headline, its first line, first.
function gathered(list) {
  if (!list || !list.length) return null;
  return el("div", { class: "gathered" }, list.map((check) => {
    let said;
    if (check.state === "running") said = el("p", { class: "hint" }, "Gathering what you need to answer…");
    else if (!check.said) said = el("p", { class: "hint" }, "It said nothing.");
    else {
      const text = check.said.trim();
      const cut = text.indexOf("\n");
      const headline = (cut < 0 ? text : text.slice(0, cut)).trim().replace(/^#+\s*/, "").replace(/^\*\*(.*)\*\*$/, "$1");
      const rest = cut < 0 ? "" : text.slice(cut + 1).trim();
      said = [el("div", { class: "headline" }, headline), rest ? markdown(rest) : null];
    }
    return el("div", { class: "gather" },
      el("span", { class: "check" }, el("span", { class: `mark ${checkMark(check.state)}` }), check.name),
      said);
  }));
}

// Answers what the agent asks with the option at `index`, once.
function choose(event, index) {
  const row = event.target.closest(".options");
  for (const button of row.querySelectorAll("button")) button.disabled = true;
  act({ Choose: { key: chat.key, index } }).then((done) => {
    if (!done) for (const button of row.querySelectorAll("button")) button.disabled = false;
  });
}

// Buttons that each do a message once, all of them off while it goes,
// the first standing out, as the one most likely tapped.
function buttons(list) {
  const row = el("div", { class: "answers" });
  for (const [label, message, first] of list) {
    row.append(el("button", {
      type: "button",
      class: first ? "first" : null,
      onclick: () => {
        for (const button of row.querySelectorAll("button")) button.disabled = true;
        act(message).then((done) => {
          if (!done) for (const button of row.querySelectorAll("button")) button.disabled = false;
        });
      },
    }, label));
  }
  return row;
}

// A goal the agent proposed: while the human has not answered it, a card
// in the accent, with why, to set it or let it be; then its line, which
// says what they did.
function proposal(proposed, index) {
  const answers = { set: "set", changed: "changed, and set", dismissed: "let be" };
  if (proposed.answer) {
    return line(`proposal-${index}`, "◎", "var(--link)", `Proposed goal: ${proposed.goal}`, answers[proposed.answer], false);
  }
  return el("div", { class: "card-offer" },
    cardHead("◎", "Proposed goal", "for you to set"),
    el("div", {}, proposed.goal),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    buttons([
      ["Set goal", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

// A name the agent proposed for the project: while the human has not
// answered it, a card in the accent, with why, to rename the project so or
// let it be; then its line, which says what they did.
function nameProposal(proposed, index) {
  const answers = { set: "renamed", changed: "renamed", dismissed: "let be" };
  if (proposed.answer) {
    return line(`name-proposal-${index}`, "✎", "var(--link)", `Proposed name: ${proposed.name}`, answers[proposed.answer], false);
  }
  return el("div", { class: "card-offer" },
    cardHead("✎", "Proposed name", "for the project, and its folder"),
    el("div", {}, proposed.name),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    buttons([
      ["Rename", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

// What the agent proposed the project is: while the human has not
// answered it, a card in the accent, with why, to set it or let it be;
// then its line, which says what they did.
function aboutProposal(proposed, index) {
  const answers = { set: "set", changed: "set", dismissed: "let be" };
  if (proposed.answer) {
    return line(`about-proposal-${index}`, "✎", "var(--link)", `What the project is: ${proposed.about}`, answers[proposed.answer], false);
  }
  return el("div", { class: "card-offer" },
    cardHead("✎", "About the project", "each chat is told it first"),
    el("div", {}, proposed.about),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    buttons([
      ["Set", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

// An icon the agent proposed for the project: while the human has not
// answered it, a card in the accent that shows it as the rail would, with
// why, to set it or let it be; then its line, which says what they did.
function iconProposal(proposed, index) {
  const answers = { set: "set", changed: "set", dismissed: "let be" };
  if (proposed.answer) {
    return line(`icon-proposal-${index}`, "✎", "var(--link)", `Proposed icon, from ${proposed.from}`, answers[proposed.answer], false);
  }
  const icon = el("img", { src: `/media/${encodeURIComponent(proposed.file)}`, alt: "" });
  return el("div", { class: "card-offer" },
    cardHead("✎", "Proposed icon", "for the project, on the rail"),
    el("div", { class: "proposed-icon" }, el("span", { class: "square" }, icon), el("span", { class: "why" }, proposed.from)),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    buttons([
      ["Set", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

// A project the agent proposed to make of the chat: while the human has
// not answered it, a card in the accent, with why, to make it, the chat
// moving into it, or let it be; then its line, which says what they did.
function projectProposal(proposed, index) {
  const answers = { set: "made", changed: "made", dismissed: "let be" };
  if (proposed.answer) {
    return line(`project-proposal-${index}`, "✦", "var(--link)", `Proposed project: ${proposed.name}`, answers[proposed.answer], false);
  }
  return el("div", { class: "card-offer" },
    cardHead("✦", "Proposed project", "this chat moves into it"),
    el("div", {}, proposed.name),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    proposed.sketch_name ? el("div", { class: "why" }, `The files of the sketch \u201c${proposed.sketch_name}\u201d go into it, as its directory.`) : null,
    buttons([
      ["Make project", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

// Cox's card asking whether to keep what the chat learned in the wiki, or,
// once answered, its line.
function keep(asked, index) {
  if (asked.kept !== null && asked.kept !== undefined) {
    return line(`keep-${index}`, "✎", "var(--link)", `${asked.why} Keep it in the wiki?`, asked.kept ? "asked to save" : "not now", false);
  }
  return el("div", { class: "card-offer" },
    cardHead("✎", asked.why, "keep it?"),
    el("div", { class: "why" }, "Keep what was learned in the wiki, for the agents of every project to find again?"),
    buttons([
      ["Save to wiki", { AnswerKeep: { key: chat.key, index, keep: true } }, true],
      ["Not now", { AnswerKeep: { key: chat.key, index, keep: false } }],
    ]));
}

function checkMark(state) {
  return { waiting: "waiting", running: "", passed: "done", failed: "found", broken: "failed" }[state];
}

// A check of a turn's end, or of what happened, as the window's chip: a
// ring while it waits or runs, then a dot of how it went.
function checkChip(check) {
  return el("span", { class: `chip ${check.state === "broken" ? "failed" : ""}`, title: check.said || null },
    el("span", { class: `mark ${checkMark(check.state)}` }), el("span", {}, check.name));
}

// What ran as a turn ended, or as something else happened, on a line:
// `title`, then a chip for each.
function checks(title, list) {
  return el("div", { class: "checks" }, el("span", { class: "label" }, title), list.map(checkChip));
}

function firstLine(text) {
  return text.trim().split("\n")[0];
}

function shorten(text, most) {
  return text.length > most ? `${text.slice(0, most - 1)}…` : text;
}

// Markdown, as far as a phone needs it: paragraphs, headings, lists,
// quotes, code, tables, Mermaid diagrams, and inline code, bold, italics
// and links.
function markdown(text) {
  const lines = text.split("\n").map((line) => line.replace(/^\t+/, (tabs) => "    ".repeat(tabs.length)));
  return el("div", { class: "entry reply" }, blocks(lines));
}

// A line that starts an item of a list: its indent, its marker, a bullet
// or a number, and its text.
const ITEM = /^( *)([-*+]|\d+[.)])\s+(.*)$/;

// A line that starts a block other than a paragraph, and so ends one.
const STARTS_BLOCK = /^\s*(```|~~~|#{1,6}\s|>|[-*+]\s|\d+[.)]\s)/;

// The blocks `lines` hold, as nodes.
function blocks(lines) {
  const found = [];
  let at = 0;
  while (at < lines.length) {
    const line = lines[at];
    const fence = /^\s*(```|~~~)/.exec(line);
    if (fence) {
      const code = [];
      at++;
      while (at < lines.length && !lines[at].trim().startsWith(fence[1])) code.push(lines[at++]);
      at++;
      const language = line.trim().slice(fence[1].length).trim().split(/\s+/)[0];
      found.push(language === "mermaid" ? diagram(code.join("\n")) : codeBlock(code.join("\n")));
      continue;
    }
    if (!line.trim()) { at++; continue; }
    if (startsTable(lines, at)) {
      const rows = [line];
      const aligns = cells(lines[at + 1]).map(alignOf);
      at += 2;
      while (at < lines.length && lines[at].trim() && lines[at].includes("|")) rows.push(lines[at++]);
      found.push(table(rows.map(cells), aligns));
      continue;
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    if (heading) { found.push(el("h3", {}, inline(heading[1]))); at++; continue; }
    if (/^\s*>/.test(line)) {
      const quote = [];
      while (at < lines.length && /^\s*>/.test(lines[at])) quote.push(lines[at++].replace(/^\s*>\s?/, ""));
      found.push(el("blockquote", {}, inline(quote.join(" "))));
      continue;
    }
    if (ITEM.test(line)) {
      const [node, after] = list(lines, at);
      found.push(node);
      at = after;
      continue;
    }
    const paragraph = [];
    while (at < lines.length && lines[at].trim() && !STARTS_BLOCK.test(lines[at]) && !startsTable(lines, at)) {
      paragraph.push(lines[at++]);
    }
    if (!paragraph.length) paragraph.push(lines[at++]);
    found.push(el("p", {}, inline(paragraph.join(" "))));
  }
  return found;
}

// The list whose first item is at line `at` of `lines`, and the line
// after it. Its items are those of its kind, numbered or not, up to one
// indented less than an item of it. Each holds its line and those under
// it indented more than its marker, as blocks of their own, so a list
// in an item nests in it; a line not indented, right under its text,
// goes on with the text. A blank line between items makes the list
// loose, its items' paragraphs apart, as the window shows them; a tight
// list's items hold their text alone. An item that starts with a box,
// `[ ]` or `[x]`, shows the box for its marker.
function list(lines, at) {
  const first = ITEM.exec(lines[at]);
  const ordered = /\d/.test(first[2]);
  const items = [];
  let loose = false;
  while (at < lines.length) {
    const item = ITEM.exec(lines[at]);
    if (!item || /\d/.test(item[2]) !== ordered) break;
    const indent = item[1].length;
    const column = item[0].length - item[3].length;
    const body = [item[3]];
    at++;
    while (at < lines.length) {
      const line = lines[at];
      if (!line.trim()) {
        let next = at;
        while (next < lines.length && !lines[next].trim()) next++;
        if (next === lines.length || indentOf(lines[next]) <= indent) break;
        while (at < next) { body.push(""); at++; }
        loose = true;
        continue;
      }
      if (indentOf(line) > indent) body.push(line.slice(Math.min(indentOf(line), column)));
      else if (body[body.length - 1].trim() && !STARTS_BLOCK.test(line) && !startsTable(lines, at)) body.push(line);
      else break;
      at++;
    }
    items.push(body);
    let next = at;
    while (next < lines.length && !lines[next].trim()) next++;
    const following = next > at && next < lines.length && ITEM.exec(lines[next]);
    if (following && /\d/.test(following[2]) === ordered) { loose = true; at = next; }
  }
  const start = parseInt(first[2], 10);
  const node = el(ordered ? "ol" : "ul", { start: ordered && start !== 1 ? String(start) : null });
  for (const body of items) {
    const box = /^\[([ xX])\]\s+/.exec(body[0]);
    if (box) body[0] = body[0].slice(box[0].length);
    const held = blocks(body);
    node.append(el("li", { class: box ? "task" : null },
      box ? el("span", { class: "box" }, box[1] === " " ? "☐" : "☑") : null,
      loose ? held : held.flatMap((one) => (one.tagName === "P" ? [...one.childNodes] : [one]))));
  }
  return [node, at];
}

// How many spaces `line` starts with.
function indentOf(line) {
  return line.length - line.trimStart().length;
}

function codeBlock(code) {
  return el("pre", {}, el("code", {}, code));
}

// A Mermaid diagram, as coxd draws it, in the window's colors, or its
// source as code if coxd cannot draw it.
function diagram(source) {
  const picture = el("img", {
    class: "diagram",
    src: `/diagram?${new URLSearchParams({ source })}`,
    alt: source,
    onerror: () => picture.replaceWith(codeBlock(source)),
  });
  return el("div", { class: "diagram" }, picture);
}

// Whether a table starts at line `at` of `lines`: a row of cells, then
// the line under its head, of dashes between pipes.
function startsTable(lines, at) {
  const under = lines[at + 1];
  return lines[at].includes("|") && under !== undefined
    && /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(under) && under.includes("-")
    && (under.includes("|") || lines[at].trim().replace(/^\||\|$/g, "").includes("|"));
}

// The cells of a table's row, split at its pipes but those escaped or in
// code.
function cells(row) {
  const found = [];
  let cell = "";
  let code = false;
  const text = row.trim().replace(/^\|/, "");
  for (let at = 0; at < text.length; at++) {
    const char = text[at];
    if (char === "\\" && text[at + 1] === "|") { cell += "|"; at++; continue; }
    if (char === "`") code = !code;
    if (char === "|" && !code) { found.push(cell.trim()); cell = ""; continue; }
    cell += char;
  }
  if (cell.trim()) found.push(cell.trim());
  return found;
}

// How a column's cells line up, as the dashes under its head say.
function alignOf(under) {
  const left = under.startsWith(":");
  const right = under.endsWith(":");
  if (left && right) return "center";
  return right ? "right" : left ? "left" : null;
}

// A table, its head first, with as many columns as its head, which
// scrolls sideways when wider than the phone.
function table(rows, aligns) {
  const width = rows[0].length;
  const row = (cellsOf, tag) => el("tr", {}, Array.from({ length: width }, (_, column) =>
    el(tag, { class: aligns[column] ? `align-${aligns[column]}` : null }, inline(cellsOf[column] || ""))));
  return el("div", { class: "table" },
    el("table", {},
      el("thead", {}, row(rows[0], "th")),
      el("tbody", {}, rows.slice(1).map((one) => row(one, "td")))));
}

function inline(text) {
  const nodes = [];
  const pattern = /`([^`]+)`|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > last) nodes.push(text.slice(last, match.index));
    if (match[1] !== undefined) nodes.push(el("code", {}, match[1]));
    else if (match[2] !== undefined) nodes.push(el("strong", {}, match[2]));
    else if (match[3] !== undefined) nodes.push(el("em", {}, match[3]));
    else if (/^https?:\/\//.test(match[5])) {
      nodes.push(el("a", { href: match[5], target: "_blank", rel: "noopener noreferrer" }, match[4]));
    } else nodes.push(match[4]);
    last = match.index + match[0].length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

// The goal as the input that sets it reads it: `$ ` and a command, or
// words, or nothing to clear it.
function goalOf(text) {
  const line = text.trim();
  if (!line) return null;
  if (line.startsWith("$ ")) return { Command: line.slice(2).trim() };
  return { Words: line };
}

// Whether the chat takes a line now, rather than once its turn ends: an
// agent that steers takes it into the turn it takes.
function takesLines(chat) {
  if (chat.state === "Idle" || chat.state === "Closed") return true;
  return Boolean(chat.steers) && (chat.state === "Busy" || chat.state === "Asking");
}

function hold(text) {
  held = text;
  compose.held.hidden = held === null;
  compose.held.textContent = held === null ? "" : `Goes once this turn ends: ${shorten(held, 80)}`;
}

async function sendHeld() {
  const text = held;
  hold(null);
  if (!(await act({ Submit: { key: chat.key, text } }))) compose.line.value = text;
}

compose.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const text = compose.line.value.trim();
  if (!text || !chat) return;
  compose.line.value = "";
  grow();
  hold(text);
  if (takesLines(chat)) await sendHeld();
});
compose.stop.addEventListener("click", () => {
  if (chat) act({ Stop: { key: chat.key } });
});
compose.goalButton.addEventListener("click", () => {
  if (!chat) return;
  compose.sketch.hidden = true;
  compose.goal.hidden = !compose.goal.hidden;
  if (!compose.goal.hidden) {
    compose.goalLine.value = goalText(chat.goal) || "";
    compose.goalLine.focus();
  }
});
compose.goal.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (chat && await act({ SetGoal: { key: chat.key, goal: goalOf(compose.goalLine.value) } })) {
    compose.goal.hidden = true;
  }
});
compose.sketchButton.addEventListener("click", () => {
  if (!chat) return;
  compose.goal.hidden = true;
  compose.sketch.hidden = !compose.sketch.hidden;
  if (compose.sketch.hidden) return;
  const told = sketchingTold();
  if (told && told.start) sketchFrom = told.start;
  compose.sketchLine.value = "";
  drawSketch();
  if (!compose.sketchLine.hidden) compose.sketchLine.focus();
});
compose.sketchFrom.addEventListener("click", () => {
  sketchFrom = sketchFrom === "project" ? "empty" : "project";
  drawSketch();
  compose.sketchLine.focus();
});
compose.sketch.addEventListener("submit", async (event) => {
  event.preventDefault();
  const what = compose.sketchLine.value.trim();
  if (!chat || !what || compose.sketchLine.hidden) return;
  const row = listing && listing.chats.find((one) => one.key === chat.key);
  const known = new Set(listing ? listing.chats.map((one) => one.key) : []);
  if (await act({ StartSketch: { key: chat.key, sketch: sketchFrom, what } })) {
    compose.sketch.hidden = true;
    // The sketch opens in the chat's project, and shows, as the window
    // shows it.
    if (row) {
      opening = { project: row.project, known, from: chat.key };
      showOpened();
    }
  }
});
// The line grows with what is written in it, up to a few lines.
function grow() {
  compose.line.style.height = "auto";
  compose.line.style.height = `${Math.min(compose.line.scrollHeight, 160)}px`;
}
compose.line.addEventListener("input", () => {
  grow();
  drawCommands();
});

// The name of a command being written in the word that ends at `cursor`,
// if the word is `/` and the start of a name, anywhere in the line, as
// the window's `naming` finds it: where the word is, and what of the name
// is written before the cursor. Null for a word that is not.
function naming(line, cursor) {
  const before = line.slice(0, cursor);
  const start = before.search(/\S*$/);
  const word = before.slice(start);
  if (!word.startsWith("/")) return null;
  const query = word.slice(1);
  // A path, as `/usr/bin`, is no command's name.
  if (query.includes("/")) return null;
  // The rest of the word goes too, when the cursor is inside it.
  const gap = line.slice(cursor).search(/\s/);
  return { start, end: gap < 0 ? line.length : cursor + gap, query };
}

// The commands of `commands` whose name can start with `query`, what of
// a name is written after its `/`, the best first, as the window's list
// over its input finds them: those whose name starts with it, then those
// whose name has it, then those whose description has it, whatever its
// case.
function matching(commands, query) {
  const lower = query.toLowerCase();
  const rank = (command) => {
    const name = command.name.toLowerCase();
    if (name.startsWith(lower)) return 0;
    if (name.includes(lower)) return 1;
    if ((command.description || "").toLowerCase().includes(lower)) return 2;
    return null;
  };
  // The sort is stable: the agent's order stays within a rank.
  return commands
    .map((command) => [rank(command), command])
    .filter(([fit]) => fit !== null)
    .sort((one, other) => one[0] - other[0])
    .map(([, command]) => command);
}

// `line` with the command `name` at its start, ready for its input, and
// where its cursor goes, as the window's `taken` puts it: the name being
// written in `named` leaves the line, with the gap before it, and the
// command of `commands` the line started with, if it did, gives way, as
// the agent runs the command a line starts with and no other.
function taken(line, named, name, commands) {
  const before = line.slice(0, named.start).replace(/\s$/, "");
  const rest = before + line.slice(named.end);
  const first = rest.split(/\s/, 1)[0];
  const started = commands.some((command) => `/${command.name}` === first);
  const lead = started ? first.length : 0;
  const gone = rest.length - rest.slice(lead).trimStart().length;
  const command = `/${name} `;
  return {
    line: command + rest.slice(gone),
    cursor: command.length + Math.max(before.length - gone, 0),
  };
}

// The list over the input of the agent's commands the word at the cursor
// can go on to name, as the window's: each by its name, with what to
// write after it and what it does. A tap puts the command at the start of
// the line, ready for its input, which closes the list.
function drawCommands() {
  const line = compose.line.value;
  const named = chat && !compose.form.hidden ? naming(line, compose.line.selectionStart) : null;
  const fits = named ? matching(chat.commands, named.query) : [];
  compose.commands.hidden = !fits.length;
  compose.commands.replaceChildren(...fits.map((command) => el("button", {
    type: "button",
    class: "command",
    role: "option",
    // The line keeps the keyboard, as the tap goes to the list.
    onmousedown: (event) => event.preventDefault(),
    onclick: () => {
      const took = taken(line, named, command.name, chat.commands);
      compose.line.value = took.line;
      compose.line.setSelectionRange(took.cursor, took.cursor);
      grow();
      compose.line.focus();
      drawCommands();
    },
  },
  el("span", { class: "name" }, el("span", { class: "slash" }, `/${command.name}`),
    command.hint ? el("span", { class: "hint" }, command.hint) : null),
  command.description ? el("span", { class: "what" }, command.description) : null)));
}
// The list follows the cursor, as it moves without the line changing.
document.addEventListener("selectionchange", () => {
  if (document.activeElement === compose.line) drawCommands();
});

// ---- The Activity, the top of the page ----
//
// What goes on in every project, as the window's Activity page of the
// home tells it, read as JSON from /activity, which coxd puts together
// as the window does, from the chats as it lists them itself, so that it
// is there with no window open. It is asked again whenever coxd lists
// the chats anew, and every few seconds while the top shows, since a
// plan goes on with no change to the list; and drawn as that page draws
// it, laid for a phone: how it all stands, then a card for each chat at
// the top of its tree, in groups by what it needs, then the pull
// requests in theirs. A tap on a card shows its chat; on its plan, the
// steps; on "Timeline", what the chat did when, read from
// /activity/timeline for it alone.

// What coxd last told goes on, and the text it came as, to draw afresh
// only when it changed.
let activity = null;
let activityText = null;
let activityAsked = false;
let activityAgain = false;
// How often, in milliseconds, the top asks what goes on, and how often
// it is drawn again though nothing changed, for "4 min ago" to go on.
const ACTIVITY_EVERY = 5000;
const AGES_EVERY = 30000;
// The window the third tile tells of, by its name, as the human last
// picked it, kept across the page's loads.
let windowed = (() => {
  try {
    return localStorage.getItem("activity.window") || "1 h";
  } catch (_) {
    return "1 h";
  }
})();
// What the human unfolded: the plans, by their chat's key; the
// subagents folded away, by the key of the chat over them; the folds of
// those done; the timelines, by the number their chat is kept by, and
// what was read of each; what a line of one printed, by its key; and
// the buttons pressed, until coxd tells what came of them.
const plansOpen = new Set();
const subsFolded = new Set();
const foldsOpen = new Set();
const storiesOpen = new Set();
const stories = new Map();
const printedOpen = new Set();
const pressed = new Set();

// Asks coxd what goes on, and draws it if it changed: one question at a
// time, and once more after it when it was asked meanwhile.
async function readActivity() {
  if (activityAsked) {
    activityAgain = true;
    return;
  }
  activityAsked = true;
  try {
    const answer = await fetch("/activity", { cache: "no-store" });
    if (answer.ok) {
      const text = await answer.text();
      if (text !== activityText) {
        activityText = text;
        activity = JSON.parse(text);
        pressed.clear();
        // Something moved: the timelines open are read again.
        for (const kept of storiesOpen) readStory(kept);
        redraw();
      }
    }
  } catch (_) {
    // The stream says when Cox cannot be reached; what was told stays.
  } finally {
    activityAsked = false;
    if (activityAgain) {
      activityAgain = false;
      readActivity();
    }
  }
}

// Reads the timeline of the chat kept as `kept`, and draws it.
async function readStory(kept) {
  let story = null;
  try {
    const answer = await fetch(`/activity/timeline?kept=${kept}`, { cache: "no-store" });
    if (answer.ok) story = await answer.json();
  } catch (_) {
    // Nothing read: the card says so.
  }
  if (JSON.stringify(stories.get(kept)) === JSON.stringify(story) && stories.has(kept)) return;
  stories.set(kept, story);
  redraw();
}

// How many of `count` there are, as "1 step" or "2 steps".
const many = (count, one, more) => `${count} ${count === 1 ? one : more}`;

// How long ago `at`, in milliseconds since the epoch, was, roughly.
function ago(at) {
  const minutes = Math.max(0, Math.round((Date.now() - at) / 60000));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} h ago`;
  return `${Math.floor(minutes / (24 * 60))} d ago`;
}

// How long from `from` to `to`, both in milliseconds, as "4 min" or
// "1 h 37 min".
function long(from, to) {
  const minutes = Math.max(0, Math.round((to - from) / 60000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
}

// How a chat of the Activity stands, in the words the page uses for it,
// and the class that colors it, as the window's page has it: at work,
// cut off, the human's turn, waiting on CI or on its subagents, stopped
// with its plan not done, or done.
function standsAs(chat) {
  if (chat.working) return { as: "work", says: "at work" };
  if (chat.cut_off) return { as: "cut", says: "cut off" };
  const waits = chat.waits ?? chat.why;
  if (waits === "asks you" || waits === "finished a turn") return { as: "yours", says: waits };
  if (waits === "waits on CI") return { as: "ci", says: waits };
  if (waits === "waits on subagents") return { as: "subs", says: waits };
  // A chat only waiting, whose CI ended while the human did not look.
  if (!chat.plans && chat.why) return { as: "yours", says: chat.why };
  if (chat.finished || waits === "finished" || waits === "done") return { as: "done", says: "done" };
  return { as: "stopped", says: "stopped" };
}

// The chats of the Activity, one each: those with a plan or at work, the
// subagents under them and the chats above them, and those that only
// wait for the human, each with how it stands, how far its plan went, as
// Cox counted it, the step under way, the subagents its agent opened,
// and all those under it, at any depth.
function activityChats() {
  const all = activity.plans.map((chat) => ({ ...chat, plans: true }));
  for (const waiting of activity.waiting) {
    const planned = all.find((chat) => chat.key === waiting.key);
    if (planned) Object.assign(planned, { asks: waiting.asks, why: waiting.why });
    else all.push({ ...waiting, plan: [], pulls: [], background: [], depth: 0 });
  }
  const chats = all.map((chat) => {
    const plan = chat.plan || [];
    return {
      ...chat,
      plan,
      stands: standsAs(chat),
      progress: chat.progress || { done: 0, of: 0 },
      under: plan.find((step) => step.state === "in_progress"),
      next: plan.find((step) => step.state === "pending"),
      subs: [],
    };
  });
  for (const chat of chats) {
    const above = chats.find((one) => one.key === chat.parent);
    if (above) above.subs.push(chat);
  }
  const below = (chat) => chat.subs.flatMap((sub) => [sub, ...below(sub)]);
  for (const chat of chats) chat.below = below(chat);
  return chats;
}

// The order chats stand in: the human's turn first, done last.
const ORDER = { cut: 0, yours: 0, work: 1, subs: 2, ci: 3, stopped: 4, done: 5 };

// How many of the subagents under `chat`, at any depth, are at work.
const busyUnder = (chat) => chat.below.filter((sub) => sub.working).length;

// What `chat` does now, in a line.
function doesNow(chat) {
  if (chat.stands.as === "cut") return `Cut off: ${chat.cut_off.says}`;
  if (chat.stands.as === "yours" && chat.asks) return chat.asks;
  if (chat.running) return `$ ${chat.running.called}`;
  if (chat.stands.as === "work" && chat.under) return chat.under.text;
  if (chat.stands.as === "subs") return `waits on ${many(busyUnder(chat), "subagent", "subagents")} at work`;
  if (chat.reported) return "↲ reported";
  if (chat.waits === "finished") return "finished, not reported yet";
  if (chat.stands.as === "stopped" && chat.next) return `next: ${chat.next.text}`;
  if (chat.said) return chat.said;
  return chat.stands.says;
}

// How `chat` stands, as its badge says it, the class that colors it and
// what it does now, and the group it leads its card to: "busy" for one
// at work, since `work` is what the conversation folds.
function badgeOf(chat) {
  switch (chat.stands.as) {
    case "cut":
      return { as: "bad", tone: "bad", says: "Cut off", group: "yours" };
    case "yours":
      if (chat.asks || chat.waits === "asks you" || chat.why === "asks you") return { as: "ask", tone: "ask", says: "Asking", group: "yours" };
      if (chat.waits === "finished a turn" || chat.why === "finished a turn") return { as: "ask", tone: "ask", says: "Your turn", group: "yours" };
      return { as: "ask", tone: "ask", says: "CI ended", group: "yours" };
    case "work":
      return { as: "busy", tone: "run", says: "Working", group: "busy" };
    case "subs":
      return { as: "busy", tone: "run", says: "Waiting", group: "busy" };
    case "ci":
      return { as: "busy", tone: "run", says: "On CI", group: "busy" };
    case "done":
      return { as: "quiet", tone: "quiet", says: chat.reported ? "Reported" : "Done", group: "done" };
    default:
      return { as: "quiet", tone: "quiet", says: "Stopped", group: "stopped" };
  }
}

// The group the card of the chat at the top, `chat`, falls in: the
// human's turn, if any chat of it waits for them, then at work, if any
// is, then stopped, if any is, else done.
function treeGroup(chat) {
  const groups = [chat, ...chat.below].map((one) => badgeOf(one).group);
  return ["yours", "busy", "stopped"].find((group) => groups.includes(group)) || "done";
}

// Where a tap on `chat` goes: its conversation, by the number it is kept
// by, if it is kept.
const chatHref = (chat) => (chat.chat == null ? null : `#kept/${chat.chat}`);

// The key's chat, by the number it is kept by, as coxd last listed it,
// if it is kept.
function keptOf(key) {
  const row = listing && listing.chats.concat(listing.archived).find((one) => one.key === key);
  return row ? row.kept : null;
}

// `node`, stepped in `indent` times: set as the page's script sets a
// style, since the page's policy lets no element say its own.
function indented(node, indent) {
  node.style.setProperty("--indent", String(indent));
  return node;
}

// A bar of blocks, one each of `all`, by its class.
const barOf = (all) => el("span", { class: "blocks" }, all.map((as) => el("s", { class: as || null })));

// The plan of `chat` as blocks, the step under way colored as the chat
// stands.
function planBlocks(chat) {
  const under = { yours: "a", cut: "f" }[chat.stands.as] || "r";
  return barOf(chat.plan.map((step) => ({ completed: "p", in_progress: under })[step.state] || ""));
}

const STEP_MARKS = { pending: "○", in_progress: "◐", completed: "✓" };

// What `chat` asks of the human: to answer or reply, in the chat, or,
// once its turn was cut off, to have it go on, as the line the human
// would send.
function chatAct(chat) {
  const href = chatHref(chat);
  if (chat.stands.as === "cut") {
    if (!chat.resume || chat.chat == null) return null;
    const asked = pressed.has(`resume-${chat.key}`);
    return el("button", {
      type: "button", class: "btn bad", disabled: asked,
      onclick: () => ask(`resume-${chat.key}`, `/activity/resume?kept=${chat.chat}`, {}),
    }, asked ? "Asked" : "Resume");
  }
  if (chat.stands.as !== "yours" || !href) return null;
  return el("a", { class: "btn", href }, badgeOf(chat).says === "Asking" ? "Answer" : "Reply");
}

// Asks coxd, at `path`, for what the button pressed as `name` asks, and
// reads what goes on then; says so when coxd would not.
async function ask(name, path, body) {
  pressed.add(name);
  redraw();
  try {
    await post(path, body);
    say("");
  } catch (error) {
    pressed.delete(name);
    say(`Not done: ${error.message}`);
  }
  readActivity();
}

// The row of a subagent, `chat`, stepped in `indent` times, and those
// under it.
function subRows(chat, indent) {
  const stands = badgeOf(chat);
  const far = chat.plan.length
    ? [planBlocks(chat), el("span", { class: "far" }, `${chat.progress.done}/${chat.progress.of}`)]
    : null;
  return [
    indented(el(chatHref(chat) ? "a" : "div", { class: `sub ${stands.as}`, href: chatHref(chat) },
      el("span", { class: "l1" },
        el("span", { class: "arrow" }, "↳"), el("span", { class: "cname" }, chat.name),
        el("span", { class: `stand ${stands.as}` }, stands.says)),
      el("span", { class: "l2" }, far,
        el("span", { class: `now ${stands.tone}` }, doesNow(chat)),
        chat.active ? el("span", { class: "age" }, ago(chat.active)) : null)), indent),
    ...subsUnder(chat, indent + 1),
  ];
}

// The subagents under `chat`: those that came from no step of its plan
// first, then those of each step, under a line of the step.
function subsUnder(chat, indent) {
  const from = (at) => chat.subs.filter((sub) => (sub.step ?? null) === at);
  return [
    ...from(null).flatMap((sub) => subRows(sub, indent)),
    ...chat.plan.flatMap((step, at) => {
      const subs = from(at);
      if (!subs.length) return [];
      return [
        indented(el("div", { class: `from ${step.state}` },
          el("span", { class: "emark" }, STEP_MARKS[step.state] || "?"), el("span", { class: "text" }, step.text)), indent),
        ...subs.flatMap((sub) => subRows(sub, indent)),
      ];
    }),
  ];
}

// The card of the chat at the top of its tree, `chat`: how it stands,
// where, when it last moved, its name and what it does now, which a tap
// opens the chat by; at its foot, its plan, which a tap unfolds to its
// steps, "Timeline", which a tap unfolds to what it did when, and what
// it asks; then the subagents under it, which a tap folds away.
function chatCard(chat) {
  const stands = badgeOf(chat);
  const under = chat.below;
  const cut = [chat, ...under].some((one) => one.stands.as === "cut");
  const planOpen = plansOpen.has(chat.key);
  const storyOpen = chat.chat != null && storiesOpen.has(chat.chat);
  const flip = (set, key) => () => {
    if (!set.delete(key)) set.add(key);
    draw();
  };
  const planned = chat.plan.length
    ? el("button", { type: "button", class: "planbar", "aria-expanded": String(planOpen), "aria-label": "The steps of its plan", onclick: flip(plansOpen, chat.key) },
        el("span", { class: "fold" }, planOpen ? "▾" : "▸"), planBlocks(chat),
        el("span", { class: "far" }, `${chat.progress.done}/${chat.progress.of}`))
    : el("span", { class: "spacer" });
  const timeline = chat.chat == null ? null : el("button", {
    type: "button", class: "story-button", "aria-expanded": String(storyOpen),
    onclick: () => {
      if (storiesOpen.delete(chat.chat)) stories.delete(chat.chat);
      else {
        storiesOpen.add(chat.chat);
        readStory(chat.chat);
      }
      draw();
    },
  }, el("span", { class: "fold" }, storyOpen ? "▾" : "▸"), "Timeline");
  const folded = subsFolded.has(chat.key);
  const busy = busyUnder(chat);
  const yours = under.filter((sub) => badgeOf(sub).group === "yours").length;
  const where = [chat.where, chat.sketch ? "sketch" : null].filter(Boolean).join(" · ");
  return el("div", { class: `acard tree ${treeGroup(chat)}${cut ? " failed" : ""}` },
    el(chatHref(chat) ? "a" : "div", { class: "abody", href: chatHref(chat) },
      el("span", { class: "l1" },
        el("span", { class: `stand ${stands.as}` }, stands.says),
        el("span", { class: "where" }, where),
        chat.active ? el("span", { class: "age" }, ago(chat.active)) : null),
      el("span", { class: "aname" }, chat.name, chat.hint ? el("span", { class: "hint" }, ` ${chat.hint}`) : null),
      el("span", { class: `now ${stands.tone}` }, doesNow(chat))),
    el("div", { class: "afoot" }, planned, timeline, chatAct(chat)),
    planOpen && chat.plan.length
      ? el("ul", { class: "steps-list" }, chat.plan.map((step) =>
          el("li", { class: step.state }, el("span", { class: "state" }, STEP_MARKS[step.state] || "?"), el("span", {}, step.text))))
      : null,
    storyOpen ? storyOf(chat) : null,
    under.length
      ? el("button", { type: "button", class: "subfold", "aria-expanded": String(!folded), onclick: flip(subsFolded, chat.key) },
          el("span", { class: "fold" }, folded ? "▸" : "▾"),
          el("span", {}, many(under.length, "subagent", "subagents"),
            yours ? [" · ", el("span", { class: "ask" }, `${yours} for you`)] : null,
            busy ? ` · ${busy} at work` : null))
      : null,
    under.length && !folded ? subsUnder(chat, 1) : null);
}

// How each kind of what happened in a chat is marked.
const EVENT_MARKS = {
  start: "▶", step_started: "◐", step_done: "✓", plan_revised: "≡", asked: "?", answered: "→",
  declined: "×", said: "✎", subagent: "↳", reported: "↲", commit: "●", pull_opened: "#",
  pull_merged: "#", pull_closed: "#", ci: "◇", check: "◇", ran: "$", turn_end: "·",
};

// What a turn's end is called, by how it ended.
const TURN_ENDS = {
  done: "Ended its turn", stopped: "Stopped", too_long: "Ended its turn: its reply grew too long",
  too_many_requests: "Ended its turn: too many requests", refused: "Refused to go on",
  failed: "The turn failed", limited: "Stopped at a usage limit", closed: "The agent closed",
  other: "Ended its turn",
};

// How a check that ran as a turn ended went, as a line says it.
const CHECKED = { passed: "passed", failed: "failed", broken: "could not run", running: "runs" };

// How long a command ran, from its seconds, as "42s" or "1m 3s".
function took(secs) {
  if (!secs) return "";
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

// What `event` of the timeline of `chat` says, as what goes in its line.
function eventSays(event, chat) {
  const b = (text) => el("b", {}, text);
  const number = `#${event.number}`;
  switch (event.kind) {
    case "step_started":
      return ["Started ", b(event.text)];
    case "step_done":
      return ["Done ", b(event.text)];
    case "plan_revised": {
      const line = (as, mark, text) => el("span", { class: as }, `${mark} ${text}`, el("br"));
      const far = event.before && event.after
        ? `${event.before.done}/${event.before.of} → ${event.after.done}/${event.after.of} steps done` : "";
      return ["Plan ", b(event.text), el("span", { class: "revised" },
        (event.added || []).map((text) => line("added", "+", text)),
        (event.removed || []).map((text) => line("removed", "−", text)),
        (event.reworded || []).map((step) => line("reworded", "~", `${step.from} → ${step.to}`)),
        (event.undone || []).map((text) => line("added", "↺", `${text} (no longer done)`)),
        el("span", { class: "lines" }, far))];
    }
    case "asked":
      return [`Asked you: ${event.text}`];
    case "answered":
      return ["You chose ", b(event.text), event.question ? ` for ${event.question}` : null, event.other ? `, “${event.other}”` : null];
    case "declined":
      return ["You declined to answer"];
    case "said":
      return [`${event.by === "human" ? "You" : "Agent"} on “${event.step}”: ${event.text}`];
    case "subagent":
      return [`Opened ${event.text}`];
    case "reported":
      return [`${event.text} reported`];
    case "commit":
      return [`Committed ${event.text} `, el("span", { class: "lines" }, `+${event.added} −${event.removed}`)];
    case "pull_opened":
      return [`${number} ${event.text} opened`];
    case "pull_merged":
      return [`${number} merged`];
    case "pull_closed":
      return [`${number} closed`];
    case "ci":
      return [`CI ${event.text} on ${number}`];
    case "ran": {
      const shown = event.commands.length;
      const of = event.of > shown ? `${shown} of ${many(event.of, "command", "commands")}` : many(shown, "command", "commands");
      return [event.text || "Ran", " ", el("span", { class: "lines" }, of),
        event.commands.map((command) => el("div", { class: `command ${command.state}` },
          el("span", { class: "ended" }, { done: "✓", failed: "✗", cancelled: "–" }[command.state] || "?"),
          el("span", { class: "cline" }, command.about ? `${command.about} · ${command.gist}` : command.gist),
          el("span", { class: "secs" }, took(command.secs))))];
    }
    case "check": {
      const key = `${chat.chat}/${event.at}/${event.text}`;
      const printed = event.said == null ? null : el("details", {
        open: printedOpen.has(key),
        ontoggle: (toggled) => {
          if (toggled.currentTarget.open) printedOpen.add(key);
          else printedOpen.delete(key);
        },
      }, el("summary", {}, event.state === "broken" ? "why" : "what it printed"), el("pre", { class: "printed" }, event.said));
      return ["Check ", b(event.text), ` ${CHECKED[event.state] || event.state}${event.on ? ` on ${event.on}` : ""}`, printed];
    }
    case "turn_end":
      return [event.cut != null ? `Cut off: ${event.cut}` : TURN_ENDS[event.text] || "Ended its turn"];
    default:
      return [event.text];
  }
}

// Whether `event` tells of something that failed: CI, a turn, or a check
// that failed or could not run.
const eventFailed = (event) =>
  ((event.kind === "ci" || event.kind === "turn_end") && event.text === "failed") ||
  (event.kind === "turn_end" && event.cut != null) ||
  (event.kind === "check" && (event.state === "failed" || event.state === "broken"));

// How `chat` ended, as its timeline, `story`, tells, or that it goes on,
// as its last line.
function storyEnd(chat, story) {
  const ended = story.ended;
  const since = story.started.at;
  const line = (as, mark, how, more, when) =>
    el("div", { class: `ending ${as}` }, el("span", { class: "at" }, when), el("span", { class: "emark" }, mark),
      el("span", { class: "how" }, `${how} `, el("span", {}, `· ${more}`)));
  if (!ended) {
    const clock = new Date().toTimeString().slice(0, 5);
    return line("busy", "◌", chat.stands.as === "ci" ? "Waits on CI" : "At work", `for ${long(since, Date.now())}, since ${story.started.when}`, clock);
  }
  const when = ended.clock || ended.when;
  const before = `${long(ended.at, Date.now())} ago`;
  const lasted = `took ${long(since, ended.at)}`;
  switch (ended.how) {
    case "resolved":
      return line("done", "✓", "Done", `marked done by ${ended.by === "human" ? "you" : "Cox"} · ${lasted}`, when);
    case "asks":
      return line("yours", "?", "Waits for your answer", `since ${long(ended.at, Date.now())}`, when);
    case "done":
      if (chat.stands.as === "subs") return line("busy", "◌", "Waits on its subagents", `${busyUnder(chat)} at work · ended its turn ${before}`, when);
      if (chat.finished) return line("done", "✓", "Done", lasted, when);
      if (chat.stands.as === "yours") return line("yours", "●", "Finished a turn", before, when);
      return line("stopped", "■", chat.progress.of ? `Stopped at ${chat.progress.done}/${chat.progress.of}` : "Stopped", `after ${long(since, ended.at)} · ${before}`, when);
    case "stopped":
      return line("stopped", "■", "Stopped", `after ${long(since, ended.at)} · ${before}`, when);
    default:
      return line("failed", "×", ended.cut != null ? `Cut off: ${ended.cut}` : TURN_ENDS[ended.how] || "Ended", before, when);
  }
}

// The timeline of `chat`, under its card's foot: its goal and what it
// spent, then what it did when, the first first, under the line of each
// day that is not today, what it runs now, how it ended or that it goes
// on, what it runs in the background and its open pull requests.
function storyOf(chat) {
  const story = stories.get(chat.chat);
  if (story === undefined) return el("div", { class: "story" }, el("p", { class: "none" }, "Reading what it did…"));
  if (story === null) return el("div", { class: "story" }, el("p", { class: "none" }, "Cox keeps nothing of what it did."));
  const spent = chat.spent ? [chat.spent.cost, chat.spent.context == null ? null : `${chat.spent.context}% context`].filter((part) => part != null) : [];
  const meta = [chat.goal ? `◎ ${chat.goal}` : null, spent.length ? spent.join(" · ") : null].filter(Boolean);
  let last = null;
  const day = (time) => {
    const of = (time && time.day) || null;
    if (of === last) return null;
    last = of;
    return el("div", { class: "day" }, of || "Today");
  };
  return el("div", { class: "story" },
    meta.length ? el("p", { class: "meta" }, meta.join(" · ")) : null,
    story.events.map((event) => [
      day(event),
      el("div", { class: `event ${event.kind}${eventFailed(event) ? " failed" : ""}${event.dropped ? " dropped" : ""}` },
        el("span", { class: "at" }, event.clock || event.when),
        el("span", { class: "emark" }, EVENT_MARKS[event.kind] || "·"),
        el("span", { class: "text" }, eventSays(event, chat))),
    ]),
    chat.running
      ? el("div", { class: "event running" }, el("span", { class: "at" }), el("span", { class: "emark" }, "◌"),
          el("span", { class: "text" }, `Running: ${chat.running.called}`))
      : null,
    day(story.ended),
    storyEnd(chat, story),
    (chat.background || []).map((what) => el("div", { class: "background" }, `${what.command ? ">_ " : ""}${what.what} · in the background`)),
    (chat.pulls || []).map((pull) => el("div", { class: "pull" }, `#${pull.number}${pull.draft ? " draft" : ""} ${pull.title || ""}${pull.ci ? ` · CI ${pull.ci}` : ""}`)));
}

// A group of `cards`, under its name and how many it holds, of the class
// `as`, or nothing when it holds none.
const cardGroup = (as, name, cards) =>
  cards.length
    ? el("div", { class: `group ${as}` },
        el("div", { class: "group-head" }, el("span", { class: "label" }, name), el("span", { class: "n" }, String(cards.length))),
        el("div", { class: "cards" }, cards))
    : null;

// A fold, by its name, `name`, which says `says`, of `cards`, open as
// the human last left it.
const cardFold = (name, says, cards) =>
  el("details", {
    class: "folded", open: foldsOpen.has(name),
    ontoggle: (event) => {
      if (event.currentTarget.open) foldsOpen.add(name);
      else foldsOpen.delete(name);
    },
  }, el("summary", {}, says), el("div", { class: "cards" }, cards));

// How it all stands: the three tiles, the third of which a tap has tell
// of another window, and every step of every plan in one bar.
function numbers(all) {
  const summaries = activity.summaries || [];
  const at = Math.max(0, summaries.findIndex((summary) => summary.window === windowed));
  const shown = summaries[at];
  const yours = all.filter((chat) => badgeOf(chat).group === "yours").length;
  const planned = all.filter((chat) => chat.plan.length);
  const steps = planned.flatMap((chat) => chat.plan);
  const rank = { completed: 0, in_progress: 1, pending: 2 };
  steps.sort((one, other) => (rank[one.state] ?? 3) - (rank[other.state] ?? 3));
  const done = planned.reduce((sum, chat) => sum + chat.progress.done, 0);
  const of = planned.reduce((sum, chat) => sum + chat.progress.of, 0);
  const cycle = () => {
    if (!summaries.length) return;
    windowed = summaries[(at + 1) % summaries.length].window;
    try {
      localStorage.setItem("activity.window", windowed);
    } catch (_) {
      // The pick holds while the page is open.
    }
    draw();
  };
  const count = shown ? shown.steps : 0;
  return [
    el("div", { class: "numbers" },
      el("div", { class: "number yours" }, el("span", { class: "count" }, String(yours)), el("span", { class: "what" }, `${yours === 1 ? "waits" : "wait"} for you`)),
      el("div", { class: "number busy" }, el("span", { class: "count" }, String(activity.busy)), el("span", { class: "what" }, "at work")),
      el("button", { type: "button", class: "number", title: "a tap shows another span", onclick: cycle },
        el("span", { class: "count" }, String(count)),
        el("span", { class: "what" }, `${count === 1 ? "step" : "steps"} done in `, el("b", {}, shown ? shown.window : "")))),
    steps.length
      ? el("div", { class: "overall" }, el("span", {}, `${done} of ${many(of, "step", "steps")}`),
          el("span", { class: "steps" }, steps.map((step) => el("i", { class: step.state }))),
          el("span", {}, many(planned.length, "chat", "chats")))
      : null,
  ];
}

// Whether `pull` is merged or closed.
const pullEnded = (pull) => pull.state === "merged" || pull.state === "closed";

// The group `pull` falls in, by what it needs, as the window's page
// groups them: merged or closed, Cox to read it, a draft, the human, as
// its CI failed, a review asked for changes or it conflicts, its CI to
// end, a review, or nothing, as it is ready to merge.
function pullNeed(pull) {
  if (pullEnded(pull)) return "done";
  if (!pull.state) return "unread";
  if (pull.state === "draft") return "draft";
  if (pull.ci === "failed" || pull.review === "changes" || pull.conflicts) return "yours";
  if (pull.ci === "running") return "running";
  if (pull.review === "required") return "review";
  return "ready";
}

// The groups of pull requests, in the order they show.
const PULL_NEEDS = [
  ["yours", "Needs you"], ["running", "CI running"], ["ready", "Ready to merge"],
  ["review", "Waiting on review"], ["draft", "Drafts"], ["unread", "Not read yet"],
];

// How the checks of `pull` stand: a block each, then how many passed,
// failed, run and were skipped, of how many.
function pullChecks(pull) {
  const checks = pull.checks;
  if (!checks || !checks.total) return pull.state ? el("span", { class: "fact quiet" }, "No CI") : null;
  const drawn = [...Array(checks.passed).fill("p"), ...Array(checks.failed).fill("f"), ...Array(checks.running).fill("r"), ...Array(checks.skipped).fill("")];
  const parts = [el("span", { class: checks.passed + checks.skipped === checks.total ? "p" : null }, `${checks.passed}/${checks.total} passed`)];
  if (checks.failed) parts.push(el("span", { class: "f" }, `${checks.failed} failed`));
  if (checks.running) parts.push(pullEnded(pull) ? el("span", {}, `${checks.running} cancelled`) : el("span", { class: "r" }, `${checks.running} running`));
  if (checks.skipped) parts.push(el("span", {}, `${checks.skipped} skipped`));
  return el("span", { class: "ci" }, barOf(drawn),
    el("span", {}, parts.flatMap((part, at) => (at ? [el("span", { class: "sep" }, " · "), part] : [part]))));
}

// What the reviews of `pull` decided, while it is open.
function pullReview(pull) {
  if (pullEnded(pull) || !pull.state) return null;
  const fact = (as, mark, says) => el("span", { class: `fact ${as}` }, mark ? el("b", {}, mark) : null, says);
  switch (pull.review) {
    case "approved":
      return fact("good", "✓", "Approved");
    case "changes":
      return fact("ask", "±", "Changes requested");
    case "required":
      return fact("quiet", "○", "Review required");
    default:
      return fact("quiet", null, "No review needed");
  }
}

// The card of `pull`: how it stands, its number, where it is, when it
// was opened, or merged or closed, its title, how its checks stand, the
// names of those that failed, what its reviews decided, whether it
// conflicts and how much it changes, which a tap opens on GitHub; at its
// foot, the chat that opened it, which a tap shows, and what asks that
// chat to fix its CI.
function pullCard(pull) {
  const ended = pullEnded(pull);
  const says = { open: "Open", draft: "Draft", merged: "Merged", closed: "Closed" }[pull.state];
  const when = ended && pull.ended ? `${pull.state} ${ago(pull.ended)}` : pull.opened ? `opened ${ago(pull.opened)}` : null;
  const author = pull.author;
  const authorKept = author ? keptOf(author.key) : null;
  const by = author
    ? el(authorKept == null ? "span" : "a", { class: `by${author.resolved ? " done" : ""}`, href: authorKept == null ? null : `#kept/${authorKept}` },
        el("span", {}, "↳"), el("span", { class: "who" }, `${author.name}${author.hint ? ` ${author.hint}` : ""}${author.resolved ? " (done)" : ""}`))
    : el("span", { class: "by none" }, "opened by hand");
  const name = `fix-${pull.repo}#${pull.number}`;
  const asked = pressed.has(name);
  const fix = pull.fix && authorKept != null
    ? el("button", {
        type: "button", class: "btn", disabled: asked,
        onclick: () => ask(name, `/activity/fix?kept=${authorKept}`, { repo: pull.repo, number: pull.number }),
      }, asked ? "Asked" : "Ask to fix")
    : null;
  return el("div", { class: `acard pr ${pullNeed(pull)}${pull.ci === "failed" && !ended ? " failed" : ""}` },
    el("a", { class: "abody", href: pull.url, target: "_blank", rel: "noopener" },
      el("span", { class: "l1" },
        says ? el("span", { class: `stand ${pull.state}` }, says) : null,
        el("span", { class: "num" }, `#${pull.number}`),
        el("span", { class: "where" }, pull.where || (pull.repo || "").split("/").pop()),
        when ? el("span", { class: "age" }, when) : null),
      el("span", { class: "aname" }, pull.title || pull.url),
      pullChecks(pull),
      !ended && pull.failed.length ? el("span", { class: "failnames" }, pull.failed.map((failed) => el("code", {}, failed))) : null,
      el("span", { class: "facts" }, pullReview(pull),
        !ended && pull.conflicts ? el("span", { class: "fact ask" }, el("b", {}, "⚠"), "Conflicts") : null,
        pull.size
          ? el("span", { class: "size" }, el("span", { class: "a" }, `+${pull.size.added}`), " ", el("span", { class: "d" }, `−${pull.size.removed}`), ` · ${many(pull.size.files, "file", "files")}`)
          : null)),
    el("div", { class: "afoot" }, by, fix));
}

// The pull requests the projects hold, under their own head, in groups
// by what they need, those merged or closed folded away; or nothing when
// they hold none.
function pullRequests() {
  const pulls = activity.pulls || [];
  if (!pulls.length) return [];
  const count = (as) => pulls.filter((pull) => pullNeed(pull) === as).length;
  const going = count("running") + count("review") + count("draft") + count("unread");
  const sum = [
    count("yours") ? `${count("yours")} ${count("yours") === 1 ? "needs" : "need"} you` : null,
    count("ready") ? `${count("ready")} ready to merge` : null,
    going ? `${going} in progress` : null,
  ].filter(Boolean);
  const done = pulls.filter(pullEnded);
  return [
    el("div", { class: "section-head" }, el("h2", {}, "Pull requests"), el("span", { class: "sum" }, sum.join(" · "))),
    ...PULL_NEEDS.map(([as, name]) => cardGroup(as, name, pulls.filter((pull) => pullNeed(pull) === as).map(pullCard))),
    done.length ? cardFold("pulls", `${done.length} merged or closed lately`, done.map(pullCard)) : null,
  ];
}

// The Activity side of the top: how it all stands, the chats in their
// groups, those done folded away, and the pull requests in theirs.
function activityPage() {
  if (!activity) return [el("p", { class: "empty" }, listing ? "Reading what goes on…" : "Reaching Cox…")];
  const all = activityChats();
  // A subagent's row stays under the chat that opened it, in the card of
  // the chat at the top.
  const tops = all.filter((chat) => chat.parent == null);
  tops.sort((one, other) => ORDER[one.stands.as] - ORDER[other.stands.as]);
  const of = (as) => tops.filter((chat) => treeGroup(chat) === as).map(chatCard);
  const done = of("done");
  return [el("div", { class: "activity" },
    activity.github ? el("p", { class: "notice" }, activity.github) : null,
    numbers(all),
    tops.length
      ? [cardGroup("yours", "Needs you", of("yours")), cardGroup("busy", "At work", of("busy")), cardGroup("stopped", "Stopped", of("stopped")),
          done.length ? cardFold("chats", `${done.length} done`, done) : null]
      : el("p", { class: "empty" }, "No chat is at work, has a plan to finish, or waits for you."),
    pullRequests())];
}

// The two segments of the head switch the side of the top that shows, in
// place: the address says which, for a chat to go back to it, but no
// page is left behind for the browser to go back through.
head.seg.addEventListener("click", (event) => {
  const tab = event.target.closest("[data-side]");
  if (!tab || tab.dataset.side === side) return;
  side = tab.dataset.side;
  history.replaceState(null, "", side === "chats" ? "#chats" : location.pathname + location.search);
  say("");
  if (side === "activity") readActivity();
  draw();
  window.scrollTo(0, 0);
});
// What goes on is asked again while the top shows, as a plan goes on
// with no change to the list of chats, and drawn again as time passes.
setInterval(() => {
  if (!chat && document.visibilityState === "visible") readActivity();
}, ACTIVITY_EVERY);
setInterval(() => {
  if (!chat && side === "activity" && document.visibilityState === "visible") redraw();
}, AGES_EVERY);

// The arrow goes back from a chat to the side of the top the human came
// from.
head.back.addEventListener("click", (event) => {
  event.preventDefault();
  location.hash = side === "chats" ? "#chats" : "";
});
window.addEventListener("hashchange", () => {
  telemetry.went(shownKept() !== null);
  if (shownKept() === null) side = location.hash === "#chats" ? "chats" : "activity";
  const trying = document.getElementById("trial");
  if (trying) trying.remove();
  document.body.classList.remove("trying");
  opened.clear();
  compose.line.value = "";
  hold(null);
  compose.goal.hidden = true;
  compose.sketch.hidden = true;
  compose.failed.hidden = true;
  say("");
  connect();
  window.scrollTo(0, 0);
});
// A phone puts a page away when it goes behind another app: the stream
// goes with it, so that coxd pushes to the phone what the page would
// have shown, and comes again with the page.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") {
    if (events) events.close();
    events = null;
  } else if (!events || events.readyState === EventSource.CLOSED) {
    connect(true);
  }
});

// Being told of the chats that need the human while they are away from
// the Mac, with Web Push: the bell on the list of chats asks for it, or
// asks no more, and the service worker shows what coxd pushes.
const push = {
  button: document.getElementById("push-button"),
  note: document.getElementById("note"),
  on: false,
  can: "serviceWorker" in navigator && "PushManager" in window && "Notification" in window,
};
let worker = null; // the service worker's registration

// Says `text` under the head, or nothing.
function say(text) {
  push.note.textContent = text;
  push.note.hidden = !text;
}

function drawPush() {
  push.button.hidden = chat !== null;
  push.button.setAttribute("aria-pressed", push.on ? "true" : "false");
  push.button.classList.toggle("on", push.on);
}

// The bytes of `text`, in URL-safe base64.
function bytesOf(text) {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4);
  return Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
}

// coxd's key, which what it pushes is signed with.
async function pushKey() {
  const answer = await fetch("/push");
  if (!answer.ok) throw new Error(await answer.text());
  return (await answer.json()).key;
}

// Whether `subscription` was made with coxd's `key`, as when coxd's data
// folder was made again it was not.
function madeWith(subscription, key) {
  const had = subscription.options && subscription.options.applicationServerKey;
  if (!had) return true;
  const ours = bytesOf(key);
  const theirs = new Uint8Array(had);
  return theirs.length === ours.length && theirs.every((byte, at) => byte === ours[at]);
}

// Tells coxd again where to push to this phone, as the browser has it,
// so that coxd keeps it whatever it forgot.
async function followPushes() {
  if (!push.can) return;
  try {
    worker = await navigator.serviceWorker.register("/sw.js");
    const subscription = await worker.pushManager.getSubscription();
    if (subscription && Notification.permission === "granted") {
      if (madeWith(subscription, await pushKey())) {
        await post("/push", subscription.toJSON());
        push.on = true;
      } else {
        await subscription.unsubscribe();
      }
    }
  } catch (error) {
    say(`Notifications: ${error.message}`);
  }
  drawPush();
}

push.button.addEventListener("click", async () => {
  say("");
  if (!push.can) {
    say(navigator.standalone === false
      ? "To be told of the chats, add Cox to the Home Screen from Safari's Share menu, and open it there."
      : "This browser cannot be told of the chats.");
    return;
  }
  try {
    if (push.on) {
      const subscription = await (worker || await navigator.serviceWorker.ready).pushManager.getSubscription();
      if (subscription) {
        await post("/push/end", { endpoint: subscription.endpoint });
        await subscription.unsubscribe();
      }
      push.on = false;
      say("This phone is told of the chats no more.");
    } else {
      // Asked first, while the tap lasts, as a phone has it.
      if (await Notification.requestPermission() !== "granted") {
        say("Cox may not notify this phone: allow it in the phone's settings.");
        return;
      }
      worker = worker || await navigator.serviceWorker.ready;
      const subscription = await worker.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: bytesOf(await pushKey()),
      });
      await post("/push", subscription.toJSON());
      push.on = true;
      say("This phone is told of the chats that need you while you are away from the Mac.");
    }
  } catch (error) {
    say(`Not done: ${error.message}`);
  }
  drawPush();
});

// A tap on a notice shows its chat: the service worker posts it to the
// page, and keeps it too, for an iPhone, which hands a page put away
// nothing posted to it, so the page reads it as it comes to the front.
// What is kept goes once read, and goes stale after a minute, so that a
// tap shows its chat once.
const TAPPED_FOR = 60 * 1000;

function showTapped(hash) {
  if (hash && location.hash !== hash) location.hash = hash;
}

async function readTapped() {
  if (!("caches" in window)) return;
  try {
    const kept = await caches.open("cox");
    const tapped = await kept.match("/tapped");
    if (!tapped) return;
    await kept.delete("/tapped");
    const { show, at } = await tapped.json();
    if (typeof show === "string" && Date.now() - at < TAPPED_FOR) showTapped(show);
  } catch (_) {
    // Nothing kept to read: the page stays where it is.
  }
}

if ("serviceWorker" in navigator) {
  navigator.serviceWorker.addEventListener("message", (event) => {
    if (event.data && typeof event.data.show === "string") {
      showTapped(event.data.show);
      readTapped();
    }
  });
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") readTapped();
});
window.addEventListener("focus", readTapped);
readTapped();

connect();
followPushes();
