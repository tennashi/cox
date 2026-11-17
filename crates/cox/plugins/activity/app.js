// What goes on in the folder of the stage, drawn by a page of the stage,
// the human's inbox: read as JSON from `cox://api/activity`, and again
// every two seconds, as Cox's own tabs follow what they show. On the left,
// what is asked of the human, the chats of the folder, and of its
// sketches, that ask them something, with what they ask, or whose turn
// ended while they were not looking, or whose CI ended; and the plans, of
// the chats whose agents take a turn, or work in the background once it
// ended, or wait on CI, or whose plan is not done, or done while the human
// did not look yet, each with what it does now, or runs in the background,
// its open pull requests, and the steps of its plan, each with the
// subagents' chats that work on it, and the subagents no step has under
// it. A plan of a chat not at work the human may dismiss, one by one or
// all at once, until its agent sends another.
// On the right, what was decided, the questions agents asked that the
// human answered, each with what they chose and why, the last first. A
// click on a chat shows it, by going to `cox://chat/<key>`. Below, what
// happened without them, the last first: the schedules that ran there,
// the pages of the wiki written from there, and the sources kept there,
// each opening where it is shown. The home, where Cox lands, tells of
// every folder, each chat with the project it is of.

const API = "cox://api/activity";
const EVERY = 2000;

const app = document.getElementById("app");
let told = null;
let toldText = null;

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
  draw();
}

/// Where `chat` is, in the home, which tells of every folder.
const where = (chat) => (told.home && chat.where ? ` <span class="in">in ${esc(chat.where)}</span>` : "");

function waiting(chat) {
  const asks = chat.asks ? `<div class="line">${esc(chat.asks)}</div>` : "";
  const goal = chat.goal ? `<div class="goal">◎ ${esc(chat.goal)}</div>` : "";
  return (
    `<a class="row" href="cox://chat/${chat.key}" title="shows the chat">` +
    `<span class="dot ${chat.why === "asks you" ? "asks" : "unread"}"></span>` +
    `<span class="what"><span class="name">${chat.sketch ? "↳ " : ""}${esc(chat.name)}</span> <span class="why">${esc(chat.why)}</span>` +
    `${where(chat)}${asks}${goal}</span></a>`
  );
}

/// The pull requests `chat`'s agent opened that are still open, each with
/// how its CI does.
function pulls(chat) {
  return (chat.pulls ?? [])
    .map(
      (pull) =>
        `<div class="pull"><span class="number">#${pull.number}</span>${pull.draft ? ` <span class="draft">draft</span>` : ""} ${esc(pull.title)}` +
        (pull.ci ? ` <span class="ci ${esc(pull.ci)}">CI ${esc(pull.ci)}</span>` : "") +
        `</div>`,
    )
    .join("");
}

function working(chat) {
  const goal = chat.goal ? `<div class="goal">◎ ${esc(chat.goal)}</div>` : "";
  const said = chat.said ? `<div class="line">${esc(chat.said)}</div>` : "";
  const running = chat.running
    ? `<div class="running" title="${esc(chat.running.line)}"><span class="prompt">$</span> ${esc(chat.running.called)}</div>`
    : "";
  const background = (chat.background ?? []).length
    ? `<div class="line">in the background:</div>` +
      chat.background
        .map(
          (what) =>
            `<div class="background">${what.command ? `<span class="cue" title="a command left running">&gt;_</span>` : ""}${esc(what.what)}</div>`,
        )
        .join("")
    : "";
  const pull = pulls(chat);
  const dot = chat.working ? "busy" : { "asks you": "asks", "finished a turn": "unread", "waits on CI": "busy" }[chat.waits] ?? "idle";
  const why = chat.waits ? ` <span class="why">${esc(chat.waits)}</span>` : "";
  return (
    `<a class="row" href="cox://chat/${chat.key}" title="shows the chat" style="margin-left: ${chat.depth * 18}px">` +
    `<span class="dot ${dot}"></span>` +
    `<span class="what"><span class="name">${chat.depth || chat.sketch ? "↳ " : ""}${esc(chat.name)}</span>${why}` +
    `${where(chat)}${goal}${said || running || background || chat.waits ? "" : `<div class="line">at work</div>`}${said}${running}${background}${pull}</span></a>` +
    plan(chat)
  );
}

/// How each state of a step is marked, and named to the human.
const STATES = {
  pending: ["○", "not started"],
  in_progress: ["◐", "in progress"],
  completed: ["✓", "done"],
};

/// The steps of `chat`'s plan, each with how far it went and the
/// subagents' chats that work on it, which a click shows.
function plan(chat) {
  if (!(chat.plan ?? []).length) return "";
  const done = chat.plan.filter((step) => step.state === "completed").length;
  const steps = chat.plan
    .map((step) => {
      const [mark, named] = STATES[step.state] ?? ["?", step.state];
      const chats = step.chats
        .map(
          (sub) =>
            `<a class="sub" href="cox://chat/${sub.key}" title="shows the chat">` +
            `<span class="dot ${sub.working ? "busy" : "idle"}"></span>↳ ${esc(sub.name)}</a>` +
            (sub.said ? `<div class="line">${esc(sub.said)}</div>` : "") +
            pulls(sub),
        )
        .join("");
      return (
        `<div class="step ${step.state}"><span class="mark" title="${named}">${mark}</span>` +
        `<span class="what"><span class="text">${esc(step.text)}</span>${chats}</span></div>`
      );
    })
    .join("");
  return (
    `<div class="plan" style="margin-left: ${chat.depth * 18 + 28}px">` +
    `<div class="count">plan · ${done}/${chat.plan.length} done${chat.dismiss ? dismiss([chat.dismiss], "dismiss", "hides this plan until the agent sends another") : ""}</div>${steps}</div>`
  );
}

/// A button that hides the plans `plans` name, as the page was told of
/// them, until their agents send others.
const dismiss = (plans, label, title) =>
  `<button class="dismiss" data-dismiss="${esc(JSON.stringify(plans))}" title="${esc(title)}">${esc(label)}</button>`;

function decided(decision) {
  const chat = decision.key != null
    ? `<a href="cox://chat/${decision.key}" title="shows the chat">${esc(decision.chat)}</a>`
    : esc(decision.chat);
  const header = decision.header && decision.header !== decision.question ? `<span class="header">${esc(decision.header)}</span> ` : "";
  const other = decision.other ? ` <span class="other">“${esc(decision.other)}”</span>` : "";
  return (
    `<div class="decision">` +
    `<div>${header}<span class="question">${esc(decision.question)}</span> <span class="arrow">→</span> <span class="choice">${esc(decision.choice)}</span>${other}</div>` +
    (decision.why ? `<div class="line">${esc(decision.why)}</div>` : "") +
    `<div class="meta"><span>${esc(decision.when)}</span> · ${chat}${where(decision)}</div>` +
    `</div>`
  );
}

/// What each kind of what happened is called.
const KINDS = { schedule: "schedule", wiki: "wiki", source: "source" };

function happened(event) {
  return (
    `<a class="row" href="${esc(event.href)}">` +
    `<span class="when">${esc(event.when)}</span>` +
    `<span class="kind">${esc(KINDS[event.kind] ?? event.kind)}</span>` +
    `<span class="what"><span class="name">${esc(event.title)}</span> <span class="says">${esc(event.says)}</span></span></a>`
  );
}

function draw() {
  if (!told) {
    app.innerHTML = `<div class="none">Cox did not answer with what goes on here.</div>`;
    return;
  }
  const count = told.waiting.length;
  const busy = told.busy;
  const plans = told.plans.length;
  const dismissable = told.plans.filter((chat) => chat.dismiss).map((chat) => chat.dismiss);
  const said = [
    count === 1 ? "1 chat waits for you" : count ? `${count} chats wait for you` : "nothing waits for you",
    busy === 1 ? "1 at work" : busy ? `${busy} at work` : "",
  ].filter(Boolean).join(" · ");
  const here = told.home ? "" : "here ";
  app.innerHTML =
    `<div class="head"><h1>Activity</h1><span class="summary">${said}</span><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    (told.github ? `<div class="notice">${esc(told.github)}</div>` : "") +
    `<div class="columns"><div>` +
    `<div class="tier">Asked of you${count ? `<b>${count}</b>` : ""}</div>` +
    `<div class="waiting">${count ? told.waiting.map(waiting).join("") : `<div class="none">Nothing ${here}is asked of you.</div>`}</div>` +
    `<div class="tier">Plans${busy ? `<i title="chats at work">${busy}</i>` : ""}` +
    `${dismissable.length > 1 ? dismiss(dismissable, `dismiss ${dismissable.length} not at work`, "hides each plan of a chat not at work until its agent sends another") : ""}</div>` +
    `<div class="waiting">${plans ? told.plans.map(working).join("") : `<div class="none">No chat ${here}is at work or has a plan to finish.</div>`}</div>` +
    `</div><div>` +
    `<div class="tier">Decided</div>` +
    (told.decided.length
      ? told.decided.map(decided).join("")
      : `<div class="none">Nothing yet: what you answer when an agent ${here}asks you to choose shows here, with why.</div>`) +
    `</div></div>` +
    `<div class="tier">What happened</div>` +
    (told.happened.length
      ? told.happened.map(happened).join("")
      : `<div class="none">Nothing yet: the schedules that run ${told.home ? "anywhere" : "here"}, the pages of the wiki written from ${told.home ? "any chat" : "here"}, and the sources kept ${here}show as they come.</div>`);
}

app.addEventListener("click", (event) => {
  const button = event.target.closest("[data-dismiss]");
  if (!button) return;
  button.disabled = true;
  load(`${API}/dismiss/${encodeURIComponent(button.dataset.dismiss)}`);
});

load();
setInterval(() => load(), EVERY);
