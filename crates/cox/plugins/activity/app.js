// What goes on in the project of the stage, drawn by a page of the stage,
// alike in every stage of the project, whichever of its chats the window
// shows: read as JSON from `cox://api/activity`, and again every two
// seconds, as Cox's own tabs follow what they show, and drawn afresh only
// when it changed.
//
// On top, how it all stands: how many chats wait for the human, how many
// are at work, and how many steps got done over the last 15 minutes,
// hour or four hours, as the human picks; then every step of every plan
// in one bar. Below, a card for each chat at the top of its tree, in
// groups by the chat of it that most needs the human: one that waits for
// them, one at work, the rest folded away. Each chat tells how it stands,
// as a badge, its plan as a bar, how far it got, what it does now, in the
// color of how it stands, when it last moved, where it is in the home, and,
// when it waits for the human, a button that shows it, to answer or reply
// there. A chat whose turn was cut off, as by a usage limit, a failure, or
// its agent or Cox going as it ran, waits for the human too, with why, in
// red, and a button that has it go on, as the line the human would send,
// from `cox://api/activity/resume/<chat>`, and its card is marked red. Under
// it, stepped in, the subagents its agent opened, a line
// each, those that came from a step of its plan under the step, and
// theirs under them, as deep as they go, which a click on how many there
// are folds away, as those three deep are at first, but for those over
// the chat the window shows, whose line is marked. A click on its name
// shows the chat, by going to
// `cox://chat/<key>`; a click elsewhere on the line opens it, and its
// story is read, from `cox://api/activity/timeline/<chat>`, for it alone:
// when it started, what it did when, and when and how it ended, then its
// steps, each of which opens its thread with a click. A check that failed
// as a turn ended, the chat's goal or a trigger, opens what it printed,
// at the end, with a click. The commands the agent ran that ran long or
// failed show under what it said before them, those it ran one after
// another together; a click on one opens it laid out to read, read from
// `cox://api/activity/command/<command>` (see
// `stage::commands::layout`), with a button that copies it as the agent
// gave it, and the last lines of what it printed. A thread tells
// what the human wrote on the step and what the agent replied, with a
// line to write another, which goes to the chat as the human's next
// line, on the step, as one typed in the chat goes: into the turn the
// agent takes, if it takes lines so, or else once the turn ends, but not
// while the agent asks something, unless it takes the line into the
// turn. A plan of a chat not at work the human may dismiss, until its
// agent sends another.
//
// Last, the pull requests the project holds, whichever chat opened them,
// a card each, in groups by what they need: the human, as CI failed, a
// review asked for changes or it conflicts; CI to end; nothing, ready to
// merge; a review; drafts. Each tells how it stands, its number, which
// opens it on GitHub, its title, how much it changes, its checks as a bar
// and how many passed, failed and run, the names of those that failed,
// what its reviews decided, when it was opened, and the chat that opened
// it, which a click shows; one whose CI failed has a button that asks
// that chat to fix it, as a line the human sends it, from
// `cox://api/activity/fix/<pull>`, which reopens it if it was done. Those
// merged or closed show a while, folded away, as a chat done does.

const API = "cox://api/activity";
/// What the button that has a chat cut off go on sends it, as Cox's
/// `activity::RESUME` says.
const RESUME = "Go on from where you were cut off.";
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

/// The lines of the stories that show what they printed, as a check that
/// failed, by their key (see `printedKey`).
const unfolded = new Set();

/// The commands of the stories opened, by the JSON that names them, and
/// what was read of each, laid out, or null when Cox keeps no such
/// command; the texts they fold that the human opened, by their key.
const commandsOpened = new Set();
const commandsRead = new Map();
const piecesOpened = new Set();

/// How deep a chat is, under the chat at the top of its, whose subagents
/// are folded away at first.
const FOLDED = 2;
/// Whether the subagents under a chat are folded away, by its key, as the
/// human last chose: unless they chose, those deep enough are, but not
/// over the chat the window shows.
const folds = new Map();
const folded = (chat) => folds.get(chat.key) ?? (chat.depth >= FOLDED && !chat.below.some((sub) => sub.key === told.shown));

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
/// the class that colors it: at work, cut off, the human's turn, waiting
/// on CI, stopped with its plan not done, or done.
function standing(chat) {
  if (chat.working) return { as: "work", says: "at work" };
  if (chat.cut_off) return { as: "cut", says: "cut off" };
  const waits = chat.waits ?? chat.why;
  if (waits === "asks you" || waits === "finished a turn") return { as: "yours", says: waits };
  if (waits === "waits on CI") return { as: "ci", says: waits };
  if (waits === "waits on subagents") return { as: "subs", says: waits };
  // A chat only waiting, whose CI ended while the human did not look.
  if (!chat.plans && chat.why) return { as: "yours", says: chat.why };
  // Its plan done, a subagent that finished, or a chat marked done.
  if (chat.finished || waits === "finished" || waits === "done") return { as: "done", says: "done" };
  return { as: "stopped", says: "stopped" };
}

/// The chats of the page, one each: those with a plan or at work, the
/// subagents under them and the chats above them, and those that only
/// wait for the human, each with how it stands, how far its plan went,
/// as Cox counted it, the step under way, the subagents its agent opened,
/// in the order Cox told them, and all those under it, at any depth.
function chats() {
  const all = told.plans.map((chat) => ({ ...chat, plans: true }));
  for (const waiting of told.waiting) {
    const planned = all.find((chat) => chat.key === waiting.key);
    if (planned) Object.assign(planned, { asks: waiting.asks, why: waiting.why });
    else all.push({ ...waiting, plan: [], pulls: [], background: [], depth: 0 });
  }
  const chats = all.map((chat) => {
    const plan = chat.plan ?? [];
    return {
      ...chat,
      stands: standing(chat),
      progress: chat.progress ?? { done: 0, of: 0 },
      under: plan.find((step) => step.state === "in_progress"),
      next: plan.find((step) => step.state === "pending"),
      subs: [],
    };
  });
  for (const chat of chats) chats.find((one) => one.key === chat.parent)?.subs.push(chat);
  const below = (chat) => chat.subs.flatMap((sub) => [sub, ...below(sub)]);
  for (const chat of chats) chat.below = below(chat);
  return chats;
}

/// The order chats stand in: the human's turn first, done or stopped
/// last.
const ORDER = { cut: 0, yours: 0, work: 1, subs: 2, ci: 3, stopped: 4, done: 5 };

/// How many of the subagents under `chat`, at any depth, are at work.
const busyUnder = (chat) => chat.below.filter((sub) => sub.working).length;

/// What `chat` does now, in a line.
function now(chat) {
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

/// How far a plan went, as Cox counted it, as "2/3".
const counted = (progress) => `${progress.done}/${progress.of}`;

/// The bar of `plan`, a block a step, as every step on the page shows.
const bar = (plan) =>
  `<span class="bar">${plan.map((step) => `<i class="${esc(step.state)}" title="${esc(step.text)}"></i>`).join("")}</span>`;

/// A bar of blocks, one each of `blocks`, each its class and what it
/// says, as a chat's plan and a pull request's checks show: blocks of a
/// size, as many as fit.
const blocks = (all) =>
  `<span class="blocks">${all.map(([as, title]) => `<s class="${as}"${title ? ` title="${esc(title)}"` : ""}></s>`).join("")}</span>`;

/// How long ago `at`, in milliseconds since the epoch, was, roughly, as
/// "4 min ago", "2 h ago" or "3 d ago", or "now".
function ago(at) {
  const minutes = Math.max(0, Math.round((Date.now() - at) / 60000));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} h ago`;
  return `${Math.floor(minutes / (24 * 60))} d ago`;
}

/// How `chat` stands, as its badge says it, and the class that colors
/// it: what asks the human first, as the group it leads its card to.
function badge(chat) {
  switch (chat.stands.as) {
    case "cut":
      return { as: "bad", says: "Cut off", group: "yours" };
    case "yours":
      if (chat.asks || chat.waits === "asks you" || chat.why === "asks you") return { as: "ask", says: "Asking", group: "yours" };
      if (chat.waits === "finished a turn" || chat.why === "finished a turn") return { as: "ask", says: "Your turn", group: "yours" };
      return { as: "ask", says: "CI ended", group: "yours" };
    case "work":
      return { as: "work", says: "Working", group: "work" };
    case "subs":
      return { as: "work", says: "Waiting", group: "work" };
    case "ci":
      return { as: "work", says: "On CI", group: "work" };
    case "done":
      return { as: "quiet", says: chat.reported ? "Reported" : "Done", group: "rest" };
    default:
      return { as: "quiet", says: "Stopped", group: "rest" };
  }
}

/// What `chat` asks of the human, as a button that shows the chat for
/// them to do it there: to answer what it asks, or to reply once its
/// turn ended; or, once its turn was cut off, one that has it go on.
function act(chat) {
  if (chat.stands.as === "cut") {
    if (!chat.resume) return "";
    return `<button class="btn bad" data-resume="${esc(JSON.stringify({ key: chat.key }))}" title="sends the chat “${esc(RESUME)}”, as you would">Resume</button>`;
  }
  if (chat.stands.as !== "yours") return "";
  const says = badge(chat).says === "Asking" ? "Answer" : "Reply";
  return `<a class="btn" href="cox://chat/${chat.key}" title="shows the chat, to ${says.toLowerCase()} there">${says}</a>`;
}

/// The plan of `chat` as a bar, the step under way colored as the chat
/// stands, and how far it got, as "2/3".
function planBar(chat) {
  const plan = chat.plan ?? [];
  if (!plan.length) return "";
  const under = { yours: "a", cut: "f" }[chat.stands.as] ?? "r";
  const as = { completed: "p", in_progress: under };
  return `<span class="ci">${blocks(plan.map((step) => [as[step.state] ?? "", step.text]))}<span>${counted(chat.progress)}</span></span>`;
}

/// What `chat` does now, in the color of how it stands.
function nowLine(chat) {
  const as = { ask: "ask", bad: "bad", work: "run", quiet: "quiet" }[badge(chat).as];
  return `<span class="chip ${as} now" title="${esc(now(chat))}">${esc(now(chat))}</span>`;
}

/// When `chat` last moved.
const moved = (chat) => (chat.active ? `<span class="age">${ago(chat.active)}</span>` : "");

/// The line, or two, of `chat`, its name stepped in `indent` times, with
/// what folds away the subagents under it, or shows them, and, while they
/// are folded away, how many there are, wait for the human and are at
/// work, and what opens under it: the chat at the top of a card on two
/// lines, a subagent's on one.
function row(chat, indent) {
  const open = opened.has(chat.key);
  const hidden = folded(chat);
  const busy = busyUnder(chat);
  const yours = chat.below.filter((sub) => badge(sub).group === "yours").length;
  const title = `${hidden ? "shows" : "folds away"} ${many(chat.below.length, "subagent", "subagents")} under it`;
  const fold = chat.subs.length
    ? `<button class="fold" data-fold="${chat.key}" title="${title}">${hidden ? "▸" : "▾"}</button>`
    : `<span class="fold"></span>`;
  const more =
    chat.subs.length && hidden
      ? `<button class="fold more" data-fold="${chat.key}" title="${title}">+${chat.below.length}${yours ? ` · <span class="ask">${yours} for you</span>` : ""}${busy ? ` · ${busy} at work` : ""}</button>`
      : "";
  const stands = badge(chat);
  const named =
    `${indent ? `<span class="arrow">↳</span>` : ""}<a class="cname" href="cox://chat/${chat.key}" title="shows the chat">${esc(chat.name)}</a>` +
    `${hint(chat)}${chat.sketch ? ` <span class="where">sketch</span>` : ""}`;
  const shown = chat.key === told.shown;
  const head = `<div class="row is-${chat.stands.as}${indent ? " sub" : " root"}${open ? " open" : ""}${shown ? " shown" : ""}" style="--indent: ${indent}" data-key="${chat.key}" title="${esc(shown ? `${chat.stands.says}, shown in the window` : chat.stands.says)}">`;
  const badged = `<span class="badge ${stands.as}">${stands.says}</span>`;
  const line = indent
    ? `${head}<span class="name">${fold}${named}${more}</span>${badged}${planBar(chat)}${nowLine(chat)}<span class="push">${moved(chat)}${act(chat)}</span></div>`
    : `${head}<div class="l1">${fold}${badged}<span class="title">${named}</span>${more}` +
      `<span class="meta">${told.home && chat.where ? `<span class="where">${esc(chat.where)}</span>` : ""}${moved(chat)}</span></div>` +
      `<div class="l2">${planBar(chat)}${nowLine(chat)}<span class="push">${act(chat)}</span></div></div>`;
  return line + (open ? opening(chat) : "");
}

/// The lines of `chat`, stepped in `indent` times, and, unless they are
/// folded away, of the subagents under it, stepped in once more: those
/// that came from no step of its plan first, then those that came from
/// each, under a line of the step, in the plan's order.
function lines(chat, indent) {
  const drawn = row(chat, indent);
  if (!chat.subs.length || folded(chat)) return drawn;
  const plan = chat.plan ?? [];
  const from = (at) => chat.subs.filter((sub) => (sub.step ?? null) === at);
  return (
    drawn +
    from(null)
      .map((sub) => lines(sub, indent + 1))
      .join("") +
    plan
      .map((step, at) => {
        const subs = from(at);
        if (!subs.length) return "";
        const [mark, named] = STATES[step.state] ?? ["?", step.state];
        return (
          `<div class="from ${esc(step.state)}" style="--indent: ${indent + 1}"><span class="mark" title="${named}">${mark}</span><span class="text">${esc(step.text)}</span></div>` +
          subs.map((sub) => lines(sub, indent + 2)).join("")
        );
      })
      .join("")
  );
}

/// Every chat under `chat`, and itself.
const tree = (chat) => [chat, ...chat.below];

/// The group the card of the chat at the top, `chat`, falls in: the
/// human's turn, if any chat of it waits for them, then at work, if any
/// is, else the rest.
function treeGroup(chat) {
  const groups = tree(chat).map((one) => badge(one).group);
  if (groups.includes("yours")) return "yours";
  return groups.includes("work") ? "work" : "rest";
}

/// The card of the chat at the top, `chat`, with its subagents, marked
/// red when a chat of it was cut off.
const card = (chat) =>
  `<div class="card tree ${treeGroup(chat)}${tree(chat).some((one) => one.stands.as === "cut") ? " failed" : ""}">${lines(chat, 0)}</div>`;

/// The folds the human opened, of the chats stopped or done and the pull
/// requests merged or closed, by their name.
const foldsOpen = new Set();

/// A group of cards, under its name and how many it holds, of the class
/// `as`, or nothing when it holds none.
const group = (as, name, cards) =>
  cards.length
    ? `<div class="cards-group ${as}"><div class="group-head">${name} <span class="n">${cards.length}</span></div><div class="cards">${cards.join("")}</div></div>`
    : "";

/// A fold, by its name, `name`, which says `says`, of `inside`, open as the
/// human last left it.
const fold = (name, says, inside) =>
  `<details class="folded" data-folded="${name}"${foldsOpen.has(name) ? " open" : ""}><summary>${says}</summary>${inside}</details>`;

/// How each kind of what happened in a chat is marked.
const MARKS = {
  start: "▶",
  step_started: "◐",
  step_done: "✓",
  plan_revised: "≡",
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
  check: "◇",
  ran: "$",
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
  limited: "Stopped at a usage limit",
  closed: "The agent closed",
  other: "Ended its turn",
};

/// How a check that ran as a turn ended went, as a line says it.
const CHECKED = { passed: "passed", failed: "failed", broken: "could not run", running: "runs" };

/// What tells the line of `event`, of the story of `chat`, from the other
/// lines, for it to show what it printed as the story is read again.
const printedKey = (chat, event) => `${chat.chat}/${event.at}/${event.kind}/${event.text}`;

/// What `event` of `chat`'s story printed, a click away, under what it
/// says: as a check that failed, or could not run, says why.
function printed(event, chat) {
  if (event.said == null) return "";
  const key = printedKey(chat, event);
  const open = unfolded.has(key);
  const what = event.state === "broken" ? "why" : "what it printed";
  return (
    ` <span class="opener" data-printed="${esc(key)}" title="${open ? "hides" : "shows"} ${what}">${open ? "▾" : "▸"} ${what}</span>` +
    (open ? `<pre class="printed">${esc(event.said)}</pre>` : "")
  );
}

/// How long a command ran, from its seconds, as "42s" or "1m 3s".
function took(secs) {
  if (!secs) return "";
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/// How each way a command ended is marked.
const ENDED = { done: "✓", failed: "✗", cancelled: "–" };

/// A stretch of a command, colored as the code tabs color it.
const stretch = ([text, what]) => (what ? `<span class="hl-${what.replace(/\./g, "-")}">${esc(text)}</span>` : esc(text));

/// The command `read`, laid out, as Cox read it for the human to open,
/// under the key `key`, with a button that copies it as the agent gave
/// it: a row for each command of a chain, after what joins it to the one
/// before, the bodies of loops indented, what it feeds a program folded,
/// a click away.
function laidOut(read, key) {
  const laid = read.shown ?? [{ depth: 0, joint: null, code: [[read.line, null]] }];
  const rows = laid
    .map((row, r) => {
      const bodies = [];
      const code = row.code
        .map((piece, p) => {
          if (!piece.fold) return stretch(piece);
          const pieceKey = `${key}/${r}/${p}`;
          const open = piecesOpened.has(pieceKey);
          const { language, lines } = piece.fold;
          if (open) bodies.push(lines);
          const label = `${language ? `${language} · ` : ""}${many(lines.length, "line", "lines")}`;
          return `<span class="piece-fold" data-piece="${esc(pieceKey)}" title="${open ? "folds" : "shows"} the text">${open ? "▾" : "▸"} ${esc(label)}</span>`;
        })
        .join("");
      const shown = bodies
        .map((lines) => `<div class="piece-body" style="--depth:${row.depth}">${lines.map((line) => `<div>${line.map(stretch).join("") || " "}</div>`).join("")}</div>`)
        .join("");
      return `<div class="laid-row" style="--depth:${row.depth}"><span class="joint">${esc(row.joint ?? "")}</span><span class="code">${code}</span></div>${shown}`;
    })
    .join("");
  return `<div class="laid"><button class="copy" data-copy="${esc(key)}" title="copies the command as the agent gave it">Copy</button>${rows}</div>`;
}

/// `command`, of the commands of `chat`'s story, on a line of its own:
/// how it ended, what the agent said it is for, its gist, and how long
/// it ran, and, once the human opened it, it laid out, and the last lines
/// of what it printed.
function ranCommand(chat, command) {
  const key = JSON.stringify({ chat: chat.chat, number: command.number });
  const open = commandsOpened.has(key);
  const named = command.about
    ? `<span class="about">${esc(command.about)}</span><span class="gist">${esc(command.gist)}</span>`
    : `<span class="gist">${esc(command.gist)}</span>`;
  const read = commandsRead.get(key);
  const under = !open
    ? ""
    : read === undefined
      ? `<div class="none">Reading the command…</div>`
      : read === null
        ? `<div class="none">Cox keeps no such command.</div>`
        : `<div class="under">${laidOut(read, key)}<pre class="printed">${esc(read.printed) || "(printed nothing)"}</pre></div>`;
  return (
    `<div class="command ${esc(command.state)}${open ? " open" : ""}" data-command="${esc(key)}" title="${open ? "folds" : "shows"} the command and what it printed">` +
    `<span class="ended">${ENDED[command.state] ?? "?"}</span><span class="line">${named}</span><span class="secs">${took(command.secs)}</span></div>` +
    under
  );
}

/// What `event` of a chat's story says, as HTML, with the chats its
/// subagents are, of `chat`, to link to.
function says(event, chat) {
  const sub = chat.subs.find((sub) => sub.name === event.text);
  const subagent = sub ? `<a href="cox://chat/${sub.key}" title="shows the chat">${esc(event.text)}</a>` : esc(event.text);
  switch (event.kind) {
    case "start":
      return esc(event.text);
    case "step_started":
      return `Started <b>${esc(event.text)}</b>`;
    case "step_done":
      return `Done <b>${esc(event.text)}</b>`;
    case "plan_revised":
      return `Plan <b>${esc(event.text)}</b>${revised(event)}`;
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
    case "ran": {
      const shown = event.commands.length;
      const of = event.of > shown ? `${shown} of ${many(event.of, "command", "commands")}` : many(shown, "command", "commands");
      const head = event.text ? `<span class="said">${esc(event.text)}</span>` : "Ran";
      return `${head} <span class="lines">${of}</span>${event.commands.map((command) => ranCommand(chat, command)).join("")}`;
    }
    case "check":
      return `Check <b>${esc(event.text)}</b> ${esc(CHECKED[event.state] ?? event.state)}${event.on ? ` on ${esc(event.on)}` : ""}${printed(event, chat)}`;
    case "turn_end":
      return esc(event.cut != null ? `Cut off: ${event.cut}` : (TURN_ENDS[event.text] ?? "Ended its turn"));
    default:
      return esc(event.text);
  }
}

/// The steps a change of the plan, `event`, tells, as Cox told them: each
/// that came, went, was reworded and was undone, a line each, then how
/// far the plan went before and after.
function revised(event) {
  const line = (as, mark, text) => `<span class="${as}">${mark} ${esc(text)}</span><br>`;
  const lines = [
    ...(event.added ?? []).map((text) => line("added", "+", text)),
    ...(event.removed ?? []).map((text) => line("removed", "−", text)),
    ...(event.reworded ?? []).map((step) => line("reworded", "~", `${step.from} → ${step.to}`)),
    ...(event.undone ?? []).map((text) => line("added", "↺", `${text} (no longer done)`)),
  ];
  const far = event.before && event.after ? `${counted(event.before)} → ${counted(event.after)} steps done` : "";
  return `<span class="revised">${lines.join("")}<span class="lines">${esc(far)}</span></span>`;
}

/// Whether `event` tells of something that failed: CI, a turn, or a
/// check that failed or could not run.
const failed = (event) =>
  ((event.kind === "ci" || event.kind === "turn_end") && event.text === "failed") ||
  (event.kind === "turn_end" && event.cut != null) ||
  (event.kind === "check" && (event.state === "failed" || event.state === "broken"));

/// A line of the day `time` is of, before the first line of the story
/// that is of another day than the line before it, as `days` follows it,
/// of the class `as` besides: the column of times has room for the time
/// of day alone.
function dayOf(time, days, as = "") {
  const day = time?.day ?? null;
  if (day === days.last) return "";
  days.last = day;
  return `<div class="day${as ? ` ${as}` : ""}">${esc(day ?? "Today")}</div>`;
}

/// How `chat` ended, as its story tells, or that it goes on, as a line,
/// under the line of its day once it is another than `days` follows.
function ending(chat, story, days) {
  const ended = story.ended;
  const since = story.started.at;
  const at = (when) => `<span class="at">${esc(when)}</span>`;
  const line = (as, mark, how, more, when) =>
    `${dayOf(ended, days, "ending")}<div class="end ${as}">${at(when)}<span class="mark">${mark}</span><span class="how">${how} <span>· ${esc(more)}</span></span></div>`;
  const clock = new Date().toTimeString().slice(0, 5);
  if (!ended) {
    const what = chat.stands.as === "ci" ? "Waits on CI" : "At work";
    return line("work", "◌", what, `for ${long(since, Date.now())}, since ${story.started.when}`, clock);
  }
  const ago = `${long(ended.at, Date.now())} ago`;
  const took = `took ${long(since, ended.at)}`;
  switch (ended.how) {
    case "resolved":
      return line("done", "✓", "Done", `marked done by ${ended.by === "human" ? "you" : "Cox"} · ${took}`, ended.clock ?? ended.when);
    case "asks":
      return line("yours", "?", "Waits for your answer", `since ${long(ended.at, Date.now())}`, ended.clock ?? ended.when);
    case "done":
      if (chat.stands.as === "subs") return line("work", "◌", "Waits on its subagents", `${busyUnder(chat)} at work · ended its turn ${ago}`, ended.clock ?? ended.when);
      if (chat.finished) return line("done", "✓", "Done", took, ended.clock ?? ended.when);
      if (chat.stands.as === "yours") return line("yours", "●", "Finished a turn", ago, ended.clock ?? ended.when);
      return line("stopped", "■", chat.progress.of ? `Stopped at ${counted(chat.progress)}` : "Stopped", `after ${long(since, ended.at)} · ${ago}`, ended.clock ?? ended.when);
    case "stopped":
      return line("stopped", "■", "Stopped", `after ${long(since, ended.at)} · ${ago}`, ended.clock ?? ended.when);
    default:
      return line("failed", "×", esc(ended.cut != null ? `Cut off: ${ended.cut}` : (TURN_ENDS[ended.how] ?? "Ended")), ago, ended.clock ?? ended.when);
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
  // The day of the line before, as the story goes, from today's.
  const days = { last: null };
  const toldStory =
    story === undefined
      ? `<div class="none">Reading what it did…</div>`
      : story === null
        ? ""
        : story.events
            .map(
              (event) =>
                dayOf(event, days) +
                `<div class="event ${esc(event.kind)}${failed(event) ? " failed" : ""}${event.dropped ? " dropped" : ""}"><span class="at">${esc(event.clock ?? event.when)}</span>` +
                `<span class="mark">${MARKS[event.kind] ?? "·"}</span><span class="text">${says(event, chat)}</span></div>`,
            )
            .join("") +
          (chat.running
            ? `<div class="event running"><span class="at"></span><span class="mark">◌</span><span class="text" title="${esc(chat.running.line)}">Running: ${esc(chat.running.called)}</span></div>`
            : "") +
          ending(chat, story, days);
  const background = (chat.background ?? [])
    .map((what) => `<div class="background">${what.command ? "&gt;_ " : ""}${esc(what.what)} <span class="on">in the background</span></div>`)
    .join("");
  const pulls = (chat.pulls ?? [])
    .map(
      (pull) =>
        `<div class="pull"><span class="number">#${pull.number}</span>${pull.draft ? " draft" : ""} ${esc(pull.title)}` +
        (pull.ci ? ` <span class="pci ${esc(pull.ci)}">CI ${esc(pull.ci)}</span>` : "") +
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

/// The steps of `chat`'s plan, each with its thread, which a click on it
/// opens: the subagents that came from it show under the chat's line.
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
      return (
        `<div class="step ${esc(step.state)}"><span class="mark" title="${named}">${mark}</span>` +
        `<span>${text}${shows ? thread(chat) : ""}</span></div>`
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
  const yours = all.filter((chat) => badge(chat).group === "yours").length;
  // Every plan on the page, the subagents' at any depth too.
  const planned = all.filter((chat) => (chat.plan ?? []).length);
  const steps = planned.flatMap((chat) => chat.plan);
  const rank = { completed: 0, in_progress: 1, pending: 2 };
  steps.sort((one, other) => (rank[one.state] ?? 3) - (rank[other.state] ?? 3));
  // How far all the plans went, as Cox counted each.
  const done = planned.reduce((sum, chat) => sum + chat.progress.done, 0);
  return (
    `<div class="numbers">` +
    `<div class="number yours"><div class="count">${yours}</div><div class="what">${yours === 1 ? "waits" : "wait"} for you</div></div>` +
    `<div class="number work"><div class="count">${told.busy}</div><div class="what">at work</div></div>` +
    `<div class="number"><div class="count">${shown?.steps ?? 0}</div><div class="what">${shown?.steps === 1 ? "step" : "steps"} done in ${picks}</div></div>` +
    `</div>` +
    (steps.length
      ? `<div class="overall"><span>${done} of ${many(planned.reduce((sum, chat) => sum + chat.progress.of, 0), "step", "steps")}</span>${bar(steps)}<span>${many(planned.length, "chat", "chats")}</span></div>`
      : "")
  );
}

/// Whether `pull` is merged or closed.
const ended = (pull) => pull.state === "merged" || pull.state === "closed";

/// The group `pull` falls in, by what it needs: merged or closed, a
/// draft, the human, as its CI failed, a review asked for changes or it
/// conflicts, its CI to end, nothing, as it is ready to merge, a review,
/// or Cox to read it.
function need(pull) {
  if (ended(pull)) return "done";
  if (!pull.state) return "unread";
  if (pull.state === "draft") return "draft";
  if (pull.ci === "failed" || pull.review === "changes" || pull.conflicts) return "yours";
  if (pull.ci === "running") return "running";
  if (pull.review === "required") return "review";
  return "ready";
}

/// The groups of pull requests, in the order they show, by their class
/// and name.
const NEEDS = [
  ["yours", "Needs you"],
  ["running", "CI running"],
  ["ready", "Ready to merge"],
  ["review", "Waiting on review"],
  ["draft", "Drafts"],
  ["unread", "Not read yet"],
];

/// How `pull` stands, as its badge says it.
function pullBadge(pull) {
  const says = { open: "Open", draft: "Draft", merged: "Merged", closed: "Closed" }[pull.state];
  return says ? `<span class="badge ${pull.state}">${says}</span>` : "";
}

/// How the checks of `pull` stand: a block each, then how many passed,
/// failed, run and were skipped, of how many.
function checks(pull) {
  const checks = pull.checks;
  if (!checks || !checks.total) return `<span class="chip quiet">${pull.state ? "No CI" : ""}</span>`;
  const all = [
    ...Array(checks.passed).fill(["p"]),
    ...Array(checks.failed).fill(["f"]),
    ...Array(checks.running).fill(["r"]),
    ...Array(checks.skipped).fill([""]),
  ];
  const parts = [`<span class="${checks.passed + checks.skipped === checks.total ? "p" : ""}">${checks.passed}/${checks.total} passed</span>`];
  if (checks.failed) parts.push(`<span class="f">${checks.failed} failed</span>`);
  if (checks.running) parts.push(ended(pull) ? `<span>${checks.running} cancelled</span>` : `<span class="r">${checks.running} running</span>`);
  if (checks.skipped) parts.push(`<span>${checks.skipped} skipped</span>`);
  return `<span class="ci">${blocks(all)}<span>${parts.join(` <span class="sep">·</span> `)}</span></span>`;
}

/// What the reviews of `pull` decided, while it is open.
function review(pull) {
  if (ended(pull) || !pull.state) return "";
  switch (pull.review) {
    case "approved":
      return `<span class="chip good"><b>✓</b> Approved</span>`;
    case "changes":
      return `<span class="chip ask"><b>±</b> Changes requested</span>`;
    case "required":
      return `<span class="chip quiet"><b>○</b> Review required</span>`;
    default:
      return `<span class="chip quiet">No review needed</span>`;
  }
}

/// How much `pull` changes, once Cox read it.
const size = (pull) =>
  pull.size
    ? `<span class="diff"><span class="a">+${pull.size.added}</span> <span class="d">−${pull.size.removed}</span> <span>· ${many(pull.size.files, "file", "files")}</span></span>`
    : "";

/// When `pull` was opened, or merged or closed.
function opens(pull) {
  if (ended(pull) && pull.ended) return `<span class="age">${pull.state} ${ago(pull.ended)}</span>`;
  return pull.opened ? `<span class="age">opened ${ago(pull.opened)}</span>` : "";
}

/// The card of `pull`: how it stands, its number, which opens it on
/// GitHub, its title, where it is and how much it changes; how its checks
/// stand, those that failed, what its reviews decided and whether it
/// conflicts, when it was opened, the chat that opened it, which a click
/// shows, and what asks that chat to fix its CI.
function pullCard(pull) {
  const where = told.home ? pull.where : pull.repo?.split("/").pop();
  const failed = !ended(pull) && pull.failed.length ? `<span class="failnames">${pull.failed.map((name) => `<code>${esc(name)}</code>`).join(" ")}</span>` : "";
  const conflicts = !ended(pull) && pull.conflicts ? `<span class="chip ask"><b>⚠</b> Conflicts</span>` : "";
  const by = pull.author
    ? `<a class="chatlink${pull.author.resolved ? " done" : ""}" href="cox://chat/${pull.author.key}" title="shows the chat">${esc(pull.author.name)}${hint(pull.author)}${pull.author.resolved ? " (done)" : ""}</a>`
    : `<span class="chatlink none">opened by hand</span>`;
  const fix = pull.fix
    ? `<button class="btn fix" data-fix="${esc(JSON.stringify({ key: pull.author.key, repo: pull.repo, number: pull.number }))}" title="sends the chat that opened it a line asking it to fix its CI, reopening it if it is done">Ask to fix</button>`
    : "";
  const when = opens(pull);
  return (
    `<div class="card pr ${need(pull)}${pull.ci === "failed" && !ended(pull) ? " failed" : ""}">` +
    `<div class="l1">${pullBadge(pull)}<a class="num" href="${esc(pull.url)}" target="_blank" title="${esc(pull.repo)}">#${pull.number}</a>` +
    `<span class="title">${esc(pull.title ?? pull.url)}</span>` +
    `<span class="meta">${where ? `<span class="where">${esc(where)}</span>` : ""}${size(pull)}</span></div>` +
    `<div class="l2">${checks(pull)}${failed}${review(pull)}${conflicts}` +
    `<span class="push">${when}${when ? `<span class="sep">·</span>` : ""}${by}${fix}</span></div>` +
    `</div>`
  );
}

/// The pull requests the project holds, as a section of their own, in
/// groups by what they need, those merged or closed folded away; or
/// nothing when it holds none.
function pullRequests() {
  const pulls = told.pulls ?? [];
  if (!pulls.length) return "";
  const count = (as) => pulls.filter((pull) => need(pull) === as).length;
  const going = count("running") + count("review") + count("draft") + count("unread");
  const sum = [
    count("yours") ? `${count("yours")} ${count("yours") === 1 ? "needs" : "need"} you` : null,
    count("ready") ? `${count("ready")} ready to merge` : null,
    going ? `${going} in progress` : null,
  ].filter(Boolean);
  const done = pulls.filter(ended);
  return (
    `<div class="section"><div class="section-head"><h2>Pull requests</h2><span class="sum">${sum.join(" · ")}</span></div>` +
    NEEDS.map(([as, name]) => group(as, name, pulls.filter((pull) => need(pull) === as).map(pullCard))).join("") +
    (done.length ? fold("pulls", `${done.length} merged or closed lately`, `<div class="cards-group done"><div class="cards">${done.map(pullCard).join("")}</div></div>`) : "") +
    `</div>`
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
  // A subagent's line stays under the chat that opened it, in the card
  // of the chat at the top.
  const tops = all.filter((chat) => chat.parent == null);
  tops.sort((one, other) => ORDER[one.stands.as] - ORDER[other.stands.as]);
  const of = (as) => tops.filter((chat) => treeGroup(chat) === as).map(card);
  const rest = of("rest");
  const cards = tops.length
    ? `<div class="section">` +
      group("yours", "Needs you", of("yours")) +
      group("work", "At work", of("work")) +
      (rest.length ? fold("chats", `${rest.length} stopped or done`, `<div class="cards-group rest"><div class="cards">${rest.join("")}</div></div>`) : "") +
      `</div>`
    : `<div class="none">No chat ${told.home ? "" : "here "}is at work, has a plan to finish, or waits for you.</div>`;
  app.innerHTML =
    `<div class="head"><h1>Activity</h1><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    (told.github ? `<div class="notice">${esc(told.github)}</div>` : "") +
    numbers(all) +
    cards +
    pullRequests();
}

// A fold stays as the human left it, as the page is drawn again.
app.addEventListener(
  "toggle",
  (event) => {
    const name = event.target.dataset?.folded;
    if (!name) return;
    if (event.target.open) foldsOpen.add(name);
    else foldsOpen.delete(name);
  },
  true,
);

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
  const copying = event.target.closest("[data-copy]");
  if (copying) {
    const read = commandsRead.get(copying.dataset.copy);
    if (!read) return;
    copy(read.line).then(() => {
      copying.textContent = "Copied";
      setTimeout(() => (copying.textContent = "Copy"), 1200);
    });
    return;
  }
  const piece = event.target.closest("[data-piece]");
  if (piece) {
    const key = piece.dataset.piece;
    if (!piecesOpened.delete(key)) piecesOpened.add(key);
    draw();
    return;
  }
  const command = event.target.closest("[data-command]");
  if (command) {
    const key = command.dataset.command;
    if (!commandsOpened.delete(key)) {
      commandsOpened.add(key);
      if (!commandsRead.has(key)) readCommand(key);
    }
    draw();
    return;
  }
  const opener = event.target.closest("[data-printed]");
  if (opener) {
    const key = opener.dataset.printed;
    if (!unfolded.delete(key)) unfolded.add(key);
    draw();
    return;
  }
  const fold = event.target.closest("[data-fold]");
  if (fold) {
    const chat = chats().find((chat) => chat.key === Number(fold.dataset.fold));
    if (chat) folds.set(chat.key, !folded(chat));
    draw();
    return;
  }
  const resuming = event.target.closest("[data-resume]");
  if (resuming) {
    resuming.disabled = true;
    resuming.textContent = "Asked";
    load(`${API}/resume/${encodeURIComponent(resuming.dataset.resume)}`);
    return;
  }
  const fixing = event.target.closest("[data-fix]");
  if (fixing) {
    fixing.disabled = true;
    fixing.textContent = "Asked";
    load(`${API}/fix/${encodeURIComponent(fixing.dataset.fix)}`);
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

/// Reads the command `key` names, laid out, and draws it.
async function readCommand(key) {
  const text = await read(`${API}/command/${encodeURIComponent(key)}`);
  commandsRead.set(key, text ? JSON.parse(text) : null);
  draw();
}

/// Copies `text`: through the page's clipboard, or, where the page may not
/// write to it, as a selection is copied.
async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

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
