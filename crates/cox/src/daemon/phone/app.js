// The phone's page: the projects and chats coxd runs, and a chat's
// conversation, as coxd tells them on /events, drawn as the window draws
// its list of chats and its chat's panel. Everything the agents wrote is
// put in as text, never as HTML.

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
  sketch: document.getElementById("sketch-form"),
  sketchLabel: document.getElementById("sketch-label"),
  sketchLine: document.getElementById("sketch-line"),
  sketchFrom: document.getElementById("sketch-from"),
  sketchEndings: document.getElementById("sketch-endings"),
  sketchButton: document.getElementById("sketch-button"),
  changesCount: document.getElementById("changes-count"),
  failed: document.getElementById("failed"),
  held: document.getElementById("held"),
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
  plan: document.getElementById("plan"),
  lost: document.getElementById("lost"),
};

let listing = null;
// The pages of the sketch the chat shown works in, for the human to try
// them: { key, asked, told: { port, sketch, pages } }, as coxd lists them.
let mocks = null;
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
let chat = null; // { key, entries, state, steers, title, goal, settings }
let events = null;
let drawing = false;
// What the human opened of the conversation, folded otherwise, as the
// window keeps it: the work at its first entry, a step, a line.
const opened = new Set();
// Whether the human opened the plan under the head.
let planOpen = false;
// How many turns toward its goal a chat takes on a line of the human's,
// as the window's bar counts them.
const GOAL_TURNS = 10;

// An element of `tag`, with `props` set and `children` in it: strings as
// text, nodes as they are.
function el(tag, props, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(props || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (name === "class") node.className = value;
    else if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
    else node.setAttribute(name, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === undefined || child === null || child === false) continue;
    node.append(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
}

// The window's icons, as its own SVGs draw them, in the color of the text
// around them.
const ICONS = {
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
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
// writes it. Says so under the input when coxd would not.
async function act(message) {
  compose.failed.hidden = true;
  try {
    await post("/act", message);
    return true;
  } catch (error) {
    compose.failed.textContent = `Not done: ${error.message}`;
    compose.failed.hidden = false;
    return false;
  }
}

// The chat the address names, as `#chat/12`.
function shownKey() {
  const match = /^#chat\/(\d+)$/.exec(location.hash);
  return match ? Number(match[1]) : null;
}

function connect() {
  if (events) events.close();
  const key = shownKey();
  chat = key === null ? null : { key, entries: [], state: null, steers: false, title: null, goal: null, settings: [], came: false };
  events = new EventSource(key === null ? "/events" : `/events?chat=${key}`);
  events.onopen = () => { head.lost.hidden = true; };
  events.onerror = () => {
    head.lost.hidden = false;
    // A stream the browser gave up on, as on 401, is not tried again by
    // itself.
    if (events.readyState === EventSource.CLOSED) setTimeout(connect, 3000);
  };
  events.onmessage = (message) => hear(JSON.parse(message.data));
  draw();
}

function hear(told) {
  if (told === null || typeof told !== "object") return;
  if (told.Listing) {
    listing = told.Listing;
    // A sketch opened as a project, or on its way to an end, ends no more.
    if (sketching) sketching.asked = false;
    showOpened();
  } else if (told.Patch && chat && told.Patch.key === chat.key) {
    const patch = told.Patch.patch;
    chat.entries.length = patch.len;
    for (const [at, entry] of patch.entries) chat.entries[at] = entry;
    // A turn that ended may have made the sketch's pages.
    if (working(chat.state) && !working(patch.state) && mocks) mocks.asked = false;
    // And changed the files.
    if (working(chat.state) && !working(patch.state) && changes) changes.asked = false;
    // And ended a sketch.
    if (working(chat.state) && !working(patch.state) && sketching) sketching.asked = false;
    chat.state = patch.state;
    chat.steers = patch.steers;
    chat.title = patch.title;
    chat.goal = patch.goal;
    chat.settings = patch.settings || [];
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
  drawPush();
  if (chat) drawChat();
  else drawHome();
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

function waits(row) {
  return row.state === "Asking" || row.unread;
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
    el("a", { href: `#chat/${row.key}`, title: stands.words },
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
  location.hash = `#chat/${opened.key}`;
}

function drawHead(row) {
  head.back.hidden = !chat;
  head.icon.hidden = Boolean(chat);
  head.dot.hidden = !row;
  head.switcher.setAttribute("href", "#");
  if (!chat) {
    head.title.textContent = "Cox";
    head.waiting.hidden = true;
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

function drawHome() {
  compose.form.hidden = true;
  compose.goal.hidden = true;
  compose.goalButton.hidden = true;
  compose.changesButton.hidden = true;
  compose.sketchButton.hidden = true;
  compose.sketch.hidden = true;
  drawHead(null);
  document.title = "Cox";
  const shelves = [];
  if (!listing) {
    shelves.push(el("p", { class: "empty" }, "Reaching Cox…"));
  } else {
    const asking = listing.chats.filter(waits);
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
      return el("section", { class: project.id === null ? "shelf own" : "shelf" },
        head,
        el("ul", { class: "chats" },
          rows.map((row) => chatRow(row, { child: true })),
          project.missing ? null : el("li", { class: "new" },
            el("button", { type: "button", onclick: (event) => newChat(event.currentTarget, index) },
              el("span", { class: "plus" }, "+"), "New chat"))));
    };
    const own = listing.projects.findIndex((project) => project.id === null);
    if (own >= 0) shelves.push(shelf(own, null));
    // A project put away, or deleted, is off the list, as off the rail.
    const projects = listing.projects.map((_, index) => index)
      .filter((index) => index !== own && !listing.projects[index].archived && !listing.projects[index].deleted);
    shelves.push(el("div", { class: "projects-head" }, el("span", { class: "label" }, "Projects")));
    for (const index of projects) {
      shelves.push(shelf(index, el("div", { class: "shelf-head" }, square(index), el("span", {}, projectName(index)))));
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
  // The sketch's pages, where the human reads on, to try them.
  if (row) {
    findMocks(row);
    const tried = mockBar();
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
    if (kind === "media" || kind === "shown") shown.push([kind, value]);
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
  fetch(`/changes?chat=${row.key}`)
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
  fetch(`/sketch?chat=${row.key}`)
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

// Asks coxd for the pages of the sketch `row` works in, if it works in
// one, unless it was asked since the chat was shown or its turn last
// ended.
function findMocks(row) {
  // A sketch's chats work in its folder, as its setup says.
  const folder = (row.setup && row.setup.folder) || row.folder;
  if (!folder) return;
  if (mocks && mocks.key === row.key && mocks.asked) return;
  mocks = { key: row.key, asked: true, told: mocks && mocks.key === row.key ? mocks.told : null };
  const asked = mocks;
  fetch(`/mocks?folder=${encodeURIComponent(folder)}`)
    .then((answer) => (answer.ok ? answer.json() : null))
    .catch(() => null)
    .then((told) => {
      if (mocks !== asked) return;
      const before = JSON.stringify(asked.told);
      asked.told = told;
      if (JSON.stringify(told) !== before && chat && chat.key === asked.key) draw();
    });
}

// The pages of the sketch, for the human to try, each a chip.
function mockBar() {
  const told = mocks && chat && mocks.key === chat.key && mocks.told;
  if (!told || !told.pages.length) return null;
  return el("div", { class: "mocks" },
    el("span", { class: "label" }, "Try it"),
    told.pages.map((page) => el("button", { type: "button", onclick: () => openMock(told, page) }, page)));
}

// Where coxd serves `page` of the sketch `told` names: the Mac's name the
// page came for, at the port of the mocks, another origin than the page.
function mockAddress(told, page) {
  const path = [told.sketch, ...page.split("/")].map(encodeURIComponent).join("/");
  return `${location.protocol}//${location.hostname}:${told.port}/${path}`;
}

// Shows `page` of the sketch over the chat, in a frame, as the human
// tries it: it runs as it would on its own, but cannot reach the page
// that frames it.
function openMock(told, page) {
  const address = mockAddress(told, page);
  const frame = el("iframe", {
    src: address,
    title: page,
    sandbox: "allow-scripts allow-same-origin allow-forms allow-modals allow-popups allow-downloads",
  });
  const close = () => { viewer.remove(); document.body.classList.remove("trying"); };
  const viewer = el("div", { id: "mock", role: "dialog", "aria-label": page },
    el("div", { class: "bar" },
      el("button", { type: "button", class: "icon-button", "aria-label": "Back to the chat", onclick: close }, "‹"),
      el("span", { class: "page" }, page),
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
  if (kind === "human" || kind === "attached") return "you";
  if (kind === "cox" || kind === "keep" || kind === "checks" || kind === "event") return "cox";
  if (kind === "note" || kind === "switched" || kind === "merge") return null;
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
// where they start.
function conversation(entries, current) {
  // The agent speaking, as the chat switched from one to another.
  let agent = current;
  const switched = entries.find((entry) => entry && kindOf(entry)[0] === "switched");
  if (switched) agent = kindOf(switched)[1].from;
  const places = [];
  let speaking = null;
  const asking = chat.state === "Asking";
  for (let at = 0; at < entries.length; at++) {
    const entry = entries[at];
    if (!entry) continue;
    const [kind, value] = kindOf(entry);
    if (kind === "switched") agent = value.to;
    // Two entries of work or more in a row fold into one place, and so
    // does a tool call alone, to show as a step.
    let end = at;
    while (end < entries.length && entries[end] && WORK.has(kindOf(entries[end])[0])) end++;
    const folds = end - at > 1 || kind === "tool_call";
    const speaker = speakerOf(kind);
    const starts = speaker !== null && speaker !== speaking;
    if (speaker !== null) speaking = speaker;
    const name = { you: "YOU", cox: "COX", agent: agent.toUpperCase() }[speaker];
    const asks = kind === "permission" && (value.chosen === null || value.chosen === undefined)
      && asking && at === entries.length - 1;
    if (folds) {
      places.push(place(work(at, entries.slice(at, end).map(kindOf), starts ? name : null), starts ? speaker : null, null, starts));
      at = end - 1;
      continue;
    }
    const node = entryNode(at, kind, value, asks);
    if (!node) continue;
    // What asks the human is marked by its amber dot alone.
    if (asks) places.push(place(node, "asking", null, false));
    else places.push(place(node, starts ? speaker : null, starts ? name : null, starts));
  }
  return places;
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
    case "permission":
      return permission(at, value, asks);
    case "proposal":
      return proposal(value, at);
    case "name_proposal":
      return nameProposal(value, at);
    case "about_proposal":
      return aboutProposal(value, at);
    case "project_proposal":
      return projectProposal(value, at);
    case "keep":
      return keep(value, at);
    case "checks":
      return checks("Turn end", value);
    case "event":
      return checks(value.on.replace(".", " "), value.checks);
    case "note":
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
        }, mark(value.result), el("span", {}, short(value.title)));
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
  const shows = value.content.map(content).filter(Boolean);
  if (value.result === "failed" && !opened.has(`seen-${key}`)) {
    opened.add(`seen-${key}`);
    opened.add(key);
  }
  return el("div", { class: "call" },
    line(key, sign, color, value.title, null, shows.length > 0),
    shows.length && opened.has(key) ? shows : null);
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
  const shows = [gathered(asked.gathered), ...asked.content.map(content)].filter(Boolean);
  return el("div", { class: "call" },
    line(key, "?", "var(--asking)", asked.title, answer, shows.length > 0),
    shows.length && opened.has(key) ? shows : null);
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
      ["Rename", { AnswerNameProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerNameProposal: { key: chat.key, index, answer: "dismissed" } }],
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
      ["Set", { AnswerAboutProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerAboutProposal: { key: chat.key, index, answer: "dismissed" } }],
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
      ["Make project", { AnswerProjectProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProjectProposal: { key: chat.key, index, answer: "dismissed" } }],
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
  const root = el("div", { class: "entry reply" });
  const lines = text.split("\n");
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
      root.append(language === "mermaid" ? diagram(code.join("\n")) : codeBlock(code.join("\n")));
      continue;
    }
    if (!line.trim()) { at++; continue; }
    if (startsTable(lines, at)) {
      const rows = [line];
      const aligns = cells(lines[at + 1]).map(alignOf);
      at += 2;
      while (at < lines.length && lines[at].trim() && lines[at].includes("|")) rows.push(lines[at++]);
      root.append(table(rows.map(cells), aligns));
      continue;
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    if (heading) { root.append(el("h3", {}, inline(heading[1]))); at++; continue; }
    if (/^\s*>/.test(line)) {
      const quote = [];
      while (at < lines.length && /^\s*>/.test(lines[at])) quote.push(lines[at++].replace(/^\s*>\s?/, ""));
      root.append(el("blockquote", {}, inline(quote.join(" "))));
      continue;
    }
    const item = /^\s*([-*+]|\d+[.)])\s+/;
    if (item.test(line)) {
      const ordered = /^\s*\d/.test(line);
      const list = el(ordered ? "ol" : "ul");
      while (at < lines.length && (item.test(lines[at]) || (/^\s{2,}\S/.test(lines[at]) && list.lastChild))) {
        if (item.test(lines[at])) list.append(el("li", {}, inline(lines[at].replace(item, ""))));
        else list.lastChild.append(" ", ...inline(lines[at].trim()));
        at++;
      }
      root.append(list);
      continue;
    }
    const paragraph = [];
    while (at < lines.length && lines[at].trim() && !/^\s*(```|~~~|#{1,6}\s|>|[-*+]\s|\d+[.)]\s)/.test(lines[at])
      && !startsTable(lines, at)) {
      paragraph.push(lines[at++]);
    }
    if (!paragraph.length) paragraph.push(lines[at++]);
    root.append(el("p", {}, inline(paragraph.join(" "))));
  }
  return root;
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
compose.line.addEventListener("input", grow);

head.back.addEventListener("click", (event) => {
  event.preventDefault();
  location.hash = "";
});
window.addEventListener("hashchange", () => {
  const trying = document.getElementById("mock");
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
    connect();
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
