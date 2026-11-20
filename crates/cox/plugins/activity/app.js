// What goes on in the project of the stage, drawn by a page of the stage,
// alike in every stage of the project, whichever of its chats the window
// shows: read as JSON from `cox://api/activity`, and again every two
// seconds, as Cox's own tabs follow what they show, and drawn afresh only
// when it changed.
//
// On top, how it all stands: how many chats wait for the human, how many
// are at work, and how many steps got done over the last 15 minutes,
// hour or four hours, as the human picks; then every step of every plan
// in one bar. Below, one line a chat, grouped by project in the home:
// how it stands, its plan as a bar, how far it got, and what it does
// now, those waiting for the human first, those done or stopped last,
// stepped back; under it, stepped in, the subagents its agent opened,
// those that came from a step of its plan under the step, and theirs
// under them, as deep as they go, which a click on how many there are
// folds away, as those three deep are at first, but for those over the
// chat the window shows, whose line is marked. A click on its name
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
/// the class that colors it: the human's turn, at work, waiting on CI,
/// stopped with its plan not done, or done.
function standing(chat) {
  if (chat.working) return { as: "work", says: "at work" };
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
const ORDER = { yours: 0, work: 1, subs: 2, ci: 3, stopped: 4, done: 5 };

/// How many of the subagents under `chat`, at any depth, are at work.
const busyUnder = (chat) => chat.below.filter((sub) => sub.working).length;

/// What `chat` does now, in a line.
function now(chat) {
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

/// The bar of `plan`, a block a step.
const bar = (plan) =>
  `<span class="bar">${plan.map((step) => `<i class="${esc(step.state)}" title="${esc(step.text)}"></i>`).join("")}</span>`;

/// The line of `chat`, its name stepped in `indent` times, with what
/// folds away the subagents under it, or shows them, and, while they are
/// folded away, how many there are and are at work, and what opens under
/// it.
function row(chat, indent) {
  const open = opened.has(chat.key);
  const hidden = folded(chat);
  const busy = busyUnder(chat);
  const title = `${hidden ? "shows" : "folds away"} ${many(chat.below.length, "subagent", "subagents")} under it`;
  const fold = chat.subs.length
    ? `<button class="fold" data-fold="${chat.key}" title="${title}">${hidden ? "▸" : "▾"}</button>`
    : `<span class="fold"></span>`;
  const more =
    chat.subs.length && hidden
      ? `<button class="fold more" data-fold="${chat.key}" title="${title}">+${chat.below.length}${busy ? ` · ${busy} at work` : ""}</button>`
      : "";
  const named =
    `${fold}<span class="dot"></span>` +
    `<a href="cox://chat/${chat.key}" title="shows the chat">${chat.sketch ? "↳ " : ""}${esc(chat.name)}</a>${hint(chat)}${more}`;
  const plan = chat.plan ?? [];
  return (
    `<div class="row ${chat.stands.as}${open ? " open" : ""}${chat.key === told.shown ? " shown" : ""}" style="--indent: ${indent}" data-key="${chat.key}" title="${esc(chat.key === told.shown ? `${chat.stands.says}, shown in the window` : chat.stands.says)}">` +
    `<span class="name">${named}</span>` +
    `${plan.length ? bar(plan) : `<span class="bar"></span>`}<span class="steps">${plan.length ? counted(chat.progress) : ""}</span>` +
    `<span class="now">${esc(now(chat))}</span></div>` +
    (open ? opening(chat) : "")
  );
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
      return esc(TURN_ENDS[event.text] ?? "Ended its turn");
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
      return line("failed", "×", esc(TURN_ENDS[ended.how] ?? "Ended"), ago, ended.clock ?? ended.when);
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
  const yours = all.filter((chat) => chat.stands.as === "yours").length;
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
  const tops = all.filter((chat) => chat.parent == null);
  tops.sort((one, other) => ORDER[one.stands.as] - ORDER[other.stands.as]);
  const places = told.home ? [...new Set(tops.map((chat) => chat.where ?? ""))] : [""];
  const groups = tops.length
    ? places
        .map((place) => {
          const of = tops.filter((chat) => !told.home || (chat.where ?? "") === place);
          return `<div class="group">${told.home ? `<h2>${esc(place || "~")}</h2>` : ""}${of.map((chat) => lines(chat, 0)).join("")}</div>`;
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
