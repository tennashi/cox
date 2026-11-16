// What the agents of the folder's chats run, drawn by a page of the stage:
// read as JSON from `cox://api/commands`, and again every second. Each
// chat that ran commands, those at work first, each a link to the chat;
// under it its last steps, the commands the agent ran one after another,
// headed by what it said before them. A step folds once all its commands
// ended, but the last; a click on its heading folds or unfolds it. A
// command shows as a line, but one that ran long, as a test or a build,
// which shows big, with what it printed once it ended, so that what the
// agent waits on stands out.

const API = "cox://api/commands";
const EVERY = 1000;

const app = document.getElementById("app");
let told = null;
let toldText = null;
// The steps the human folded or unfolded, by their first command's id,
// as they chose.
const chosen = new Map();

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

const MARKS = { running: "…", done: "✓", failed: "✗", cancelled: "–" };

function took(secs) {
  if (!secs) return "";
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

function small(command) {
  return (
    `<div class="small" title="${esc(command.line)}">` +
    `<span class="mark ${command.state}">${MARKS[command.state]}</span>` +
    `<span class="line">$ ${esc(command.line)}</span>` +
    `<span class="secs">${took(command.secs)}</span></div>`
  );
}

function big(command) {
  const state =
    command.state === "running"
      ? `running · ${took(command.secs)}`
      : `${command.state}${command.secs ? ` in ${took(command.secs)}` : ""}`;
  const body =
    command.printed != null
      ? `<pre>${esc(command.printed) || "(printed nothing)"}</pre>`
      : command.state === "running"
        ? `<div class="why">${command.why ? `${esc(command.why)}. ` : ""}What it prints shows once it ends.</div>`
        : "";
  return (
    `<div class="big ${command.state}"><div class="top"><span class="line">$ ${esc(command.line)}</span>` +
    `<span class="state ${command.state}">${state}</span></div>${body}</div>`
  );
}

function step(chat, step, last) {
  const key = `${chat.key}/${step.id}`;
  const unended = step.commands.some((c) => c.state === "running" || c.state === "failed");
  const open = chosen.has(key) ? chosen.get(key) : last || unended;
  const ran = step.commands.length === 1 ? "1 command" : `${step.commands.length} commands`;
  const all = step.commands.every((c) => c.state === "done") ? " · all done" : "";
  return (
    `<div class="step"><div class="said" data-key="${esc(key)}" data-open="${open}">` +
    `<span class="fold">${open ? "▾" : "▸"}</span>` +
    `<span class="text">${esc(step.said ?? "Before saying anything")}</span>` +
    `<span class="count">${ran}${open ? "" : all}</span></div>` +
    (open ? step.commands.map((c) => (c.long || c.printed != null ? big(c) : small(c))).join("") : "") +
    `</div>`
  );
}

function chat(chat) {
  return (
    `<div class="chat"><span class="dot${chat.working ? " working" : ""}"></span>` +
    `<a href="cox://chat/${chat.key}" title="shows the chat">${chat.sketch ? "↳ " : ""}${esc(chat.name)}</a></div>` +
    chat.steps.map((s, i) => step(chat, s, i === chat.steps.length - 1)).join("")
  );
}

function draw() {
  if (!told) {
    app.innerHTML = `<div class="none">Cox did not answer with what runs here.</div>`;
    return;
  }
  const running = told.chats.flatMap((c) => c.steps.flatMap((s) => s.commands)).filter((c) => c.state === "running").length;
  const said = running === 1 ? "1 command runs" : running ? `${running} commands run` : "nothing runs";
  app.innerHTML =
    `<div class="head"><h1>Commands</h1><span class="summary">${said}</span><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    (told.chats.length
      ? told.chats.map(chat).join("")
      : `<div class="none">No agent ran a command here yet. What the agents of the chats here run shows as they run it.</div>`);
}

app.addEventListener("click", (event) => {
  const said = event.target.closest(".said");
  if (!said) return;
  chosen.set(said.dataset.key, said.dataset.open !== "true");
  draw();
});

load();
setInterval(load, EVERY);
