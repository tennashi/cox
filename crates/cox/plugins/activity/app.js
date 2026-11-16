// What goes on in the folder of the stage, drawn by a page of the stage,
// the human's inbox: read as JSON from `cox://api/activity`, and again
// every two seconds, as Cox's own tabs follow what they show. On the left,
// what is asked of the human, the chats of the folder, and of its
// sketches, that ask them something, with what they ask, or whose turn
// ended while they were not looking; and what is at work, the chats whose
// agents take a turn, each with what it does now, its subagents under it.
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

async function load() {
  let text = null;
  try {
    const answer = await fetch(API);
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

function working(chat) {
  const goal = chat.goal ? `<div class="goal">◎ ${esc(chat.goal)}</div>` : "";
  const said = chat.said ? `<div class="line">${esc(chat.said)}</div>` : "";
  const running = chat.running ? `<div class="running"><span class="prompt">$</span> ${esc(chat.running)}</div>` : "";
  return (
    `<a class="row" href="cox://chat/${chat.key}" title="shows the chat" style="margin-left: ${chat.depth * 18}px">` +
    `<span class="dot busy"></span>` +
    `<span class="what"><span class="name">${chat.depth || chat.sketch ? "↳ " : ""}${esc(chat.name)}</span>` +
    `${where(chat)}${goal}${said || running ? "" : `<div class="line">at work</div>`}${said}${running}</span></a>`
  );
}

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
  const busy = told.working.length;
  const said = [
    count === 1 ? "1 chat waits for you" : count ? `${count} chats wait for you` : "nothing waits for you",
    busy === 1 ? "1 at work" : busy ? `${busy} at work` : "",
  ].filter(Boolean).join(" · ");
  const here = told.home ? "" : "here ";
  app.innerHTML =
    `<div class="head"><h1>Activity</h1><span class="summary">${said}</span><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    `<div class="columns"><div>` +
    `<div class="tier">Asked of you${count ? `<b>${count}</b>` : ""}</div>` +
    `<div class="waiting">${count ? told.waiting.map(waiting).join("") : `<div class="none">Nothing ${here}is asked of you.</div>`}</div>` +
    `<div class="tier">At work${busy ? `<i>${busy}</i>` : ""}</div>` +
    `<div class="waiting">${busy ? told.working.map(working).join("") : `<div class="none">No chat ${here}is at work.</div>`}</div>` +
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

load();
setInterval(load, EVERY);
