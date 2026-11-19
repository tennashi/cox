// What goes on in the folder of the stage, drawn by a page of the stage:
// read as JSON from `cox://api/activity`, and again every two seconds, as
// Cox's own tabs follow what they show, and drawn afresh only when it
// changed.
//
// On top, how it all stands: how many chats wait for the human, how many
// are at work, and how many steps got done over the last 15 minutes,
// hour or four hours, as the human picks; then every step of every plan
// in one bar. Below, one line a chat, grouped by project in the home:
// how it stands, its plan as a bar, how far it got, and what it does
// now, those waiting for the human first, those done or stopped last,
// stepped back. A click on its name shows the chat, by going to
// `cox://chat/<key>`; a click elsewhere on the line opens it, and its
// story is read, from `cox://api/activity/timeline/<chat>`, for it alone:
// when it started, what it did when, and when and how it ended, then its
// steps, each of which opens its thread with a click. A thread tells
// what the human wrote on the step and what the agent replied, with a
// line to write another, which goes to the chat as the human's next
// line, on the step, as one typed in the chat goes: into the turn the
// agent takes, if it takes lines so, or else once the turn ends, but not
// while the agent asks something, unless it takes the line into the
// turn. A plan of a chat not at work the human may dismiss, until its
// agent sends another.

const API = "cox://api/activity";
const EVERY = 2000;

const app = document.getElementById("app");
let told = null;
let toldText = null;
/// The window the top numbers tell of, by its name, as the human last
/// picked it, kept across the page's loads.
let windowed = (() => {
  try {
    return localStorage.getItem("activity.window") ?? "1 h";
  } catch {
    return "1 h";
  }
})();

/// The chats opened, by their key, and the story read of each, by the
/// chat as coxd keeps it.
const opened = new Set();
const stories = new Map();

/// The step whose thread shows, by its chat, as coxd keeps it, and its
/// id, or null.
let threaded = null;

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

/// The address that tells what goes on, with the thread that shows.
const address = () =>
  threaded
    ? `${API}/thread/${encodeURIComponent(JSON.stringify({ chat: threaded.chat, step: threaded.step }))}`
    : API;

async function read(url) {
  try {
    const answer = await fetch(url, { cache: "no-store" });
    return answer.ok ? await answer.text() : null;
  } catch {
    return null;
  }
}

async function load(url = address()) {
  const text = await read(url);
  if (text === toldText) return;
  toldText = text;
  told = text ? JSON.parse(text) : null;
  // Something moved: the stories open are read again.
  for (const chat of stories.keys()) tell(chat);
  draw();
}

/// Reads the story of `chat`, as coxd keeps it, and draws it.
async function tell(chat) {
  const text = await read(`${API}/timeline/${encodeURIComponent(JSON.stringify({ chat }))}`);
  const story = text ? JSON.parse(text) : null;
  if (JSON.stringify(stories.get(chat)) === JSON.stringify(story)) return;
  stories.set(chat, story);
  draw();
}

/// How many of `count` there are, as "1 step" or "2 steps".
const many = (count, one, more) => `${count} ${count === 1 ? one : more}`;

/// How long from `from` to `to`, both in milliseconds, as "4 min" or
/// "1 h 37 min".
function long(from, to) {
  const minutes = Math.max(0, Math.round((to - from) / 60000));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return minutes % 60 ? `${hours} h ${minutes % 60} min` : `${hours} h`;
}

/// What tells `chat` apart from others of its name, when another has it.
const hint = (chat) => (chat.hint ? ` <span class="hint">${esc(chat.hint)}</span>` : "");

/// How a chat stands, in the words the page uses for it everywhere, and
/// the class that colors it: the human's turn, at work, waiting on CI,
/// stopped with its plan not done, or done.
function standing(chat) {
  if (chat.working) return { as: "work", says: "at work" };
  const waits = chat.waits ?? chat.why;
  if (waits === "asks you" || waits === "finished a turn") return { as: "yours", says: waits };
  if (waits === "waits on CI") return { as: "ci", says: waits };
  // A chat only waiting, whose CI ended while the human did not look.
  if (!chat.plans && chat.why) return { as: "yours", says: chat.why };
  // Its plan done, a subagent that finished, or a chat marked done.
  if (chat.finished || waits === "finished" || waits === "done") return { as: "done", says: "done" };
  return { as: "stopped", says: "stopped" };
}

/// The chats of the page, one each: those with a plan or at work, and
/// those that only wait for the human, each with how it stands, how far
/// its plan went, as Cox counted it, and the step under way.
function chats() {
  const all = told.plans.map((chat) => ({ ...chat, plans: true }));
  for (const waiting of told.waiting) {
    const planned = all.find((chat) => chat.key === waiting.key);
    if (planned) Object.assign(planned, { asks: waiting.asks, why: waiting.why });
    else all.push({ ...waiting, plan: [], pulls: [], background: [], depth: 0 });
  }
  return all.map((chat) => {
    const plan = chat.plan ?? [];
    return {
      ...chat,
      stands: standing(chat),
      progress: chat.progress ?? { done: 0, of: 0 },
      under: plan.find((step) => step.state === "in_progress"),
      next: plan.find((step) => step.state === "pending"),
    };
  });
}

/// The order chats stand in: the human's turn first, done or stopped
/// last.
const ORDER = { yours: 0, work: 1, ci: 2, stopped: 3, done: 4 };

/// What `chat` does now, in a line.
function now(chat) {
  if (chat.stands.as === "yours" && chat.asks) return chat.asks;
  if (chat.running) return `$ ${chat.running.called}`;
  if (chat.stands.as === "work" && chat.under) return chat.under.text;
  if (chat.stands.as === "stopped" && chat.next) return `next: ${chat.next.text}`;
  if (chat.said) return chat.said;
  return chat.stands.says;
}

/// How far a plan went, as Cox counted it, as "2/3".
const counted = (progress) => `${progress.done}/${progress.of}`;

/// The bar of `plan`, a block a step.
const bar = (plan) =>
  `<span class="bar">${plan.map((step) => `<i class="${esc(step.state)}" title="${esc(step.text)}"></i>`).join("")}</span>`;

/// The line of `chat`, and what opens under it.
function row(chat) {
  const open = opened.has(chat.key);
  const subagents = (chat.plan ?? []).flatMap((step) => step.chats).length;
  const named =
    `<a href="cox://chat/${chat.key}" title="shows the chat">${chat.depth || chat.sketch ? "↳ " : ""}${esc(chat.name)}</a>${hint(chat)}` +
    (subagents ? ` <span class="aside">+${many(subagents, "subagent", "subagents")}</span>` : "");
  const plan = chat.plan ?? [];
  return (
    `<div class="row ${chat.stands.as}${open ? " open" : ""}${chat.depth ? " under" : ""}" data-key="${chat.key}" title="${esc(chat.stands.says)}">` +
    `<span class="dot"></span><span class="name">${named}</span>` +
    `${plan.length ? bar(plan) : "<span></span>"}<span class="steps">${plan.length ? counted(chat.progress) : ""}</span>` +
    `<span class="now">${esc(now(chat))}</span></div>` +
    (open ? opening(chat) : "")
  );
}

/// How each kind of what happened in a chat is marked.
const MARKS = {
  start: "▶",
  step_started: "◐",
  step_done: "✓",
  asked: "?",
  answered: "→",
  declined: "×",
  said: "✎",
  subagent: "↳",
  reported: "↲",
  commit: "●",
  pull_opened: "#",
  pull_merged: "#",
  pull_closed: "#",
  ci: "◇",
  turn_end: "·",
  running: "◌",
};

/// What a turn's end is called, by how it ended.
const TURN_ENDS = {
  done: "Ended its turn",
  stopped: "Stopped",
  too_long: "Ended its turn: its reply grew too long",
  too_many_requests: "Ended its turn: too many requests",
  refused: "Refused to go on",
  failed: "The turn failed",
  closed: "The agent closed",
  other: "Ended its turn",
};

/// What `event` of a chat's story says, as HTML, with the chats its
/// subagents are, of `chat`, to link to.
function says(event, chat) {
  const sub = (chat.plan ?? []).flatMap((step) => step.chats).find((sub) => sub.name === event.text);
  const subagent = sub ? `<a href="cox://chat/${sub.key}" title="shows the chat">${esc(event.text)}</a>` : esc(event.text);
  switch (event.kind) {
    case "start":
      return esc(event.text);
    case "step_started":
      return `Started <b>${esc(event.text)}</b>`;
    case "step_done":
      return `Done <b>${esc(event.text)}</b>`;
    case "asked":
      return `Asked you: ${esc(event.text)}`;
    case "answered":
      return `You chose <b>${esc(event.text)}</b>${event.question ? ` for ${esc(event.question)}` : ""}${event.other ? `, “${esc(event.other)}”` : ""}`;
    case "declined":
      return "You declined to answer";
    case "said":
      return `${event.by === "human" ? "You" : "Agent"} on “${esc(event.step)}”: ${esc(event.text)}`;
    case "subagent":
      return `Opened ${subagent}`;
    case "reported":
      return `${subagent} reported`;
    case "commit":
      return `Committed ${esc(event.text)} <span class="lines">+${event.added} −${event.removed}</span>`;
    case "pull_opened":
      return `<span class="number">#${event.number}</span> ${esc(event.text)} opened`;
    case "pull_merged":
      return `<span class="number">#${event.number}</span> merged`;
    case "pull_closed":
      return `<span class="number">#${event.number}</span> closed`;
    case "ci":
      return `CI ${esc(event.text)} on <span class="number">#${event.number}</span>`;
    case "turn_end":
      return esc(TURN_ENDS[event.text] ?? "Ended its turn");
    default:
      return esc(event.text);
  }
}

/// Whether `event` tells of something that failed: CI, or a turn.
const failed = (event) => (event.kind === "ci" || event.kind === "turn_end") && event.text === "failed";

/// How `chat` ended, as its story tells, or that it goes on, as a line.
function ending(chat, story) {
  const ended = story.ended;
  const since = story.started.at;
  const at = (when) => `<span class="at">${esc(when)}</span>`;
  const line = (as, mark, how, more, when) =>
    `<div class="end ${as}">${at(when)}<span class="mark">${mark}</span><span class="how">${how} <span>· ${esc(more)}</span></span></div>`;
  const clock = new Date().toTimeString().slice(0, 5);
  if (!ended) {
    const what = chat.stands.as === "ci" ? "Waits on CI" : "At work";
    return line("work", "◌", what, `for ${long(since, Date.now())}, since ${story.started.when}`, clock);
  }
  const ago = `${long(ended.at, Date.now())} ago`;
  const took = `took ${long(since, ended.at)}`;
  switch (ended.how) {
    case "resolved":
      return line("done", "✓", "Done", `marked done by ${ended.by === "human" ? "you" : "Cox"} · ${took}`, ended.when);
    case "asks":
      return line("yours", "?", "Waits for your answer", `since ${long(ended.at, Date.now())}`, ended.when);
    case "done":
      if (chat.finished) return line("done", "✓", "Done", took, ended.when);
      if (chat.stands.as === "yours") return line("yours", "●", "Finished a turn", ago, ended.when);
      return line("stopped", "■", chat.progress.of ? `Stopped at ${counted(chat.progress)}` : "Stopped", `after ${long(since, ended.at)} · ${ago}`, ended.when);
    case "stopped":
      return line("stopped", "■", "Stopped", `after ${long(since, ended.at)} · ${ago}`, ended.when);
    default:
      return line("failed", "×", esc(TURN_ENDS[ended.how] ?? "Ended"), ago, ended.when);
  }
}

/// What opens under `chat`'s line: what it is, then side by side its
/// story, how it ended, what it runs and its open pull requests, and its
/// steps.
function opening(chat) {
  const spent = chat.spent && [chat.spent.cost, chat.spent.context == null ? null : `${chat.spent.context}% context`].filter((part) => part != null);
  const meta = [
    chat.goal ? `◎ ${esc(chat.goal)}` : null,
    spent?.length ? `<span title="${esc(chat.spent.title)}">${esc(spent.join(" · "))}</span>` : null,
  ].filter(Boolean);
  const dismissing = chat.dismiss
    ? `<button class="dismiss" data-dismiss="${esc(JSON.stringify([chat.dismiss]))}" title="hides this plan until the agent sends another">dismiss</button>`
    : "";
  const story = chat.chat == null ? undefined : stories.get(chat.chat);
  const toldStory =
    story === undefined
      ? `<div class="none">Reading what it did…</div>`
      : story === null
        ? ""
        : story.events
            .map(
              (event) =>
                `<div class="event ${esc(event.kind)}${failed(event) ? " failed" : ""}"><span class="at">${esc(event.when)}</span>` +
                `<span class="mark">${MARKS[event.kind] ?? "·"}</span><span class="text">${says(event, chat)}</span></div>`,
            )
            .join("") +
          (chat.running
            ? `<div class="event running"><span class="at"></span><span class="mark">◌</span><span class="text" title="${esc(chat.running.line)}">Running: ${esc(chat.running.called)}</span></div>`
            : "") +
          ending(chat, story);
  const background = (chat.background ?? [])
    .map((what) => `<div class="background">${what.command ? "&gt;_ " : ""}${esc(what.what)} <span class="on">in the background</span></div>`)
    .join("");
  const pulls = (chat.pulls ?? [])
    .map(
      (pull) =>
        `<div class="pull"><span class="number">#${pull.number}</span>${pull.draft ? " draft" : ""} ${esc(pull.title)}` +
        (pull.ci ? ` <span class="ci ${esc(pull.ci)}">CI ${esc(pull.ci)}</span>` : "") +
        `</div>`,
    )
    .join("");
  return (
    `<div class="opened">` +
    (meta.length || dismissing ? `<div class="meta">${meta.join(" · ")}${dismissing}</div>` : "") +
    // Its story on the left, its steps beside it, once there is room.
    `<div class="columns${(chat.plan ?? []).length ? "" : " alone"}">` +
    `<div class="story">${toldStory}${background}${pulls}</div>` +
    steps(chat) +
    `</div></div>`
  );
}

/// How each state of a step is marked, and named to the human.
const STATES = {
  pending: ["○", "not started"],
  in_progress: ["◐", "in progress"],
  completed: ["✓", "done"],
};

/// The steps of `chat`'s plan, each with the subagents' chats that work
/// on it, and its thread, which a click on it opens.
function steps(chat) {
  const plan = chat.plan ?? [];
  if (!plan.length) return "";
  const lines = plan
    .map((step) => {
      const [mark, named] = STATES[step.state] ?? ["?", step.state];
      const open = step.id != null && chat.chat != null;
      const shows = open && threaded?.chat === chat.chat && threaded?.step === step.id;
      const text = open
        ? `<span class="text opens${shows ? " shown" : ""}" data-step="${esc(JSON.stringify({ chat: chat.chat, step: step.id }))}" title="${shows ? "closes" : "opens"} the thread of this step">${esc(step.text)}</span>` +
          (step.on ? ` <span class="on" title="${many(step.on, "line", "lines")} you wrote on this step">✎ ${step.on}</span>` : "")
        : `<span class="text">${esc(step.text)}</span>`;
      const subs = step.chats
        .map(
          (sub) =>
            `<a class="sub" href="cox://chat/${sub.key}" title="shows the chat">↳ ${esc(sub.name)}${hint(sub)}${sub.working ? " · at work" : ""}${sub.said ? ` · ${esc(sub.said)}` : ""}</a>`,
        )
        .join("");
      return (
        `<div class="step ${esc(step.state)}"><span class="mark" title="${named}">${mark}</span>` +
        `<span>${text}${shows ? thread(chat) : ""}${subs}</span></div>`
      );
    })
    .join("");
  return `<div class="plan"><div class="count" title="a click on a step opens its thread">Steps · ${counted(chat.progress)} done</div>${lines}</div>`;
}

/// The thread of the step that shows, of `chat`'s plan: each line the
/// human wrote on it and what the agent replied, and a line to write
/// another, which goes while the chat takes one.
function thread(chat) {
  const shown = told.thread && told.thread.chat === threaded.chat && told.thread.step === threaded.step ? told.thread.said : null;
  const said = (shown ?? [])
    .map(
      (said) =>
        `<div class="said ${said.by}"><span class="by">${said.by === "human" ? "you" : "agent"} ${esc(said.at)}</span>` +
        `<span class="says">${esc(said.text)}</span></div>`,
    )
    .join("");
  const waiting = (shown === null ? [] : told.thread.waiting ?? [])
    .map((text) => `<div class="said human waiting"><span class="by">you · goes once the turn ends</span><span class="says">${esc(text)}</span></div>`)
    .join("");
  const why = !chat.takes
    ? "answer what it asks first"
    : chat.working
      ? chat.steers
        ? "it is at work: a line goes into its turn"
        : "it is at work: a line goes once its turn ends"
      : "";
  return (
    `<div class="thread">${shown === null ? "" : said + waiting || `<div class="none">Nothing written on this step yet.</div>`}` +
    `<form class="write" data-key="${chat.key}"><textarea name="text" rows="2" placeholder="Write on this step, for ${esc(chat.name)}"${chat.takes ? "" : " disabled"}></textarea>` +
    `<button type="submit"${chat.takes ? "" : " disabled"}>Send</button></form>` +
    (why ? `<div class="why">${esc(why)}</div>` : "") +
    `</div>`
  );
}

/// How it all stands: the three numbers, and every step in one bar.
function numbers(all) {
  const summaries = told.summaries ?? [];
  const shown = summaries.find((summary) => summary.window === windowed) ?? summaries[0];
  const picks = summaries
    .map((summary) => `<button class="pick${summary === shown ? " picked" : ""}" data-window="${esc(summary.window)}">${esc(summary.window)}</button>`)
    .join("");
  const yours = all.filter((chat) => chat.stands.as === "yours").length;
  const tops = all.filter((chat) => !chat.depth);
  const steps = tops.flatMap((chat) => chat.plan ?? []);
  const rank = { completed: 0, in_progress: 1, pending: 2 };
  steps.sort((one, other) => (rank[one.state] ?? 3) - (rank[other.state] ?? 3));
  // How far all the plans went, as Cox counted each.
  const done = tops.reduce((sum, chat) => sum + chat.progress.done, 0);
  return (
    `<div class="numbers">` +
    `<div class="number yours"><div class="count">${yours}</div><div class="what">${yours === 1 ? "waits" : "wait"} for you</div></div>` +
    `<div class="number work"><div class="count">${told.busy}</div><div class="what">at work</div></div>` +
    `<div class="number"><div class="count">${shown?.steps ?? 0}</div><div class="what">${shown?.steps === 1 ? "step" : "steps"} done in ${picks}</div></div>` +
    `</div>` +
    (steps.length
      ? `<div class="overall"><span>${done} of ${many(tops.reduce((sum, chat) => sum + chat.progress.of, 0), "step", "steps")}</span>${bar(steps)}<span>${many(tops.length, "chat", "chats")}</span></div>`
      : "")
  );
}

/// Draws the page, keeping what the human was writing on a step, and
/// where, as it was.
function draw() {
  const writing = app.querySelector(".write textarea");
  const kept = writing && { value: writing.value, focused: document.activeElement === writing, start: writing.selectionStart, end: writing.selectionEnd };
  drawn();
  const now = app.querySelector(".write textarea");
  if (kept && now) {
    now.value = kept.value;
    if (kept.focused && !now.disabled) {
      now.focus();
      now.setSelectionRange(kept.start, kept.end);
    }
  }
}

function drawn() {
  if (!told) {
    app.innerHTML = `<div class="none">Cox did not answer with what goes on here.</div>`;
    return;
  }
  const all = chats();
  // A subagent's line stays under the chat that opened it.
  const tops = all.filter((chat) => !chat.depth);
  const under = (top) => {
    const at = told.plans.findIndex((chat) => chat.key === top.key);
    const rest = [];
    for (const chat of told.plans.slice(at + 1)) {
      if (!chat.depth) break;
      rest.push(all.find((one) => one.key === chat.key));
    }
    return rest.filter(Boolean);
  };
  tops.sort((one, other) => ORDER[one.stands.as] - ORDER[other.stands.as]);
  const lines = (chats) => chats.map((chat) => row(chat) + under(chat).map(row).join("")).join("");
  const places = told.home ? [...new Set(tops.map((chat) => chat.where ?? ""))] : [""];
  const groups = tops.length
    ? places
        .map((place) => {
          const of = tops.filter((chat) => !told.home || (chat.where ?? "") === place);
          return `<div class="group">${told.home ? `<h2>${esc(place || "~")}</h2>` : ""}${lines(of)}</div>`;
        })
        .join("")
    : `<div class="none">No chat ${told.home ? "" : "here "}is at work, has a plan to finish, or waits for you.</div>`;
  app.innerHTML =
    `<div class="head"><h1>Activity</h1><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    (told.github ? `<div class="notice">${esc(told.github)}</div>` : "") +
    numbers(all) +
    groups;
}

app.addEventListener("click", (event) => {
  const pick = event.target.closest("[data-window]");
  if (pick) {
    windowed = pick.dataset.window;
    try {
      localStorage.setItem("activity.window", windowed);
    } catch {
      // The pick holds while the page is open.
    }
    draw();
    return;
  }
  const step = event.target.closest("[data-step]");
  if (step) {
    const asked = JSON.parse(step.dataset.step);
    threaded = threaded && threaded.chat === asked.chat && threaded.step === asked.step ? null : asked;
    draw();
    load();
    return;
  }
  const button = event.target.closest("[data-dismiss]");
  if (button) {
    button.disabled = true;
    load(`${API}/dismiss/${encodeURIComponent(button.dataset.dismiss)}`);
    return;
  }
  // A link shows its chat; the line around it opens or closes the chat.
  if (event.target.closest("a")) return;
  const line = event.target.closest(".row");
  if (!line) return;
  const key = Number(line.dataset.key);
  const chat = chats().find((chat) => chat.key === key);
  if (opened.has(key)) {
    opened.delete(key);
    if (chat?.chat != null) stories.delete(chat.chat);
  } else {
    opened.add(key);
    if (chat?.chat != null) tell(chat.chat);
  }
  draw();
});

/// Sends what the human wrote on the step whose thread shows.
function send(form) {
  const text = form.elements.text.value;
  if (!threaded || !text.trim()) return;
  const asked = { key: Number(form.dataset.key), chat: threaded.chat, step: threaded.step, text };
  form.elements.text.value = "";
  load(`${API}/say/${encodeURIComponent(JSON.stringify(asked))}`);
}

app.addEventListener("submit", (event) => {
  event.preventDefault();
  send(event.target);
});

app.addEventListener("keydown", (event) => {
  // Enter sends; Shift-Enter starts a new line.
  if (event.key !== "Enter" || event.shiftKey || event.isComposing || !event.target.matches(".write textarea")) return;
  event.preventDefault();
  send(event.target.form);
});

load();
setInterval(() => load(), EVERY);
