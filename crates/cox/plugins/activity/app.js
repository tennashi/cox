// What goes on in the folder of the stage, drawn by a page of the stage:
// read as JSON from `cox://api/activity`, and again every two seconds, as
// Cox's own tabs follow what they show. First what waits for the human,
// the chats of the folder, and of its sketches, that ask them something
// or whose turn ended while they were not looking, each of which a click
// shows, by going to `cox://chat/<key>`; then what happened without them,
// the last first: the schedules that ran there, the pages of the wiki
// written from there, and the sources kept there, each opening where it
// is shown. The home, where Cox lands, tells of every folder, each chat
// with the project it is of.

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

function waiting(chat) {
  const goal = chat.goal ? `<div class="goal">◎ ${esc(chat.goal)}</div>` : "";
  return (
    `<a class="row" href="cox://chat/${chat.key}" title="shows the chat">` +
    `<span class="dot ${chat.why === "asks you" ? "asks" : "unread"}"></span>` +
    `<span class="what"><span class="name">${chat.sketch ? "↳ " : ""}${esc(chat.name)}</span> <span class="why">${esc(chat.why)}</span>` +
    `${told.home && chat.where ? ` <span class="in">in ${esc(chat.where)}</span>` : ""}${goal}</span></a>`
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
  app.innerHTML =
    `<div class="head"><h1>Activity</h1><span class="where">${esc(told.folder)}</span></div>` +
    `<div class="tier">Waiting for you${count ? `<b>${count}</b>` : ""}</div>` +
    `<div class="waiting">${count ? told.waiting.map(waiting).join("") : `<div class="none">No chat ${told.home ? "" : "here "}waits for you.</div>`}</div>` +
    `<div class="tier">What happened</div>` +
    (told.happened.length
      ? told.happened.map(happened).join("")
      : `<div class="none">Nothing yet: the schedules that run ${told.home ? "anywhere" : "here"}, the pages of the wiki written from ${told.home ? "any chat" : "here"}, and the sources kept ${told.home ? "" : "here "}show as they come.</div>`);
}

load();
setInterval(load, EVERY);
