// The schedules that run in the folder of the stage, drawn by a page of
// the stage: read as JSON from `cox://api/schedules`, and again every two
// seconds, as Cox's own tabs follow what they show. The project's own
// schedules, in `.cox/schedules`, come first, then those of the human's
// workflow whose chats work in the folder; in the home, every schedule:
// the workflow's, each with where it works, then each project's own.
// Each says when it runs, when it ran last and runs next, what its agent
// does unasked, and its first line, and its file opens on a tab. A
// schedule Cox cannot read says why, and with none, the page says how
// one is written.

const API = "cox://api/schedules";
const EVERY = 2000;

const app = document.getElementById("app");
let told = null;
let toldText = null;

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

/// A file of the machine, as a link that opens it on a tab.
const open = (path, says) => `<a href="${esc(`cox://open${path}`)}" title="opens ${esc(path)} on a tab">${esc(says)}</a>`;

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

function schedule(s, where) {
  const next = s.due ? `<span class="due">due now</span>` : esc(s.next ?? "—");
  const allow = s.allow.length
    ? `<div class="allow">unasked:${s.allow.map((a) => `<span class="chip">${esc(a)}</span>`).join("")}</div>`
    : "";
  const file = s.file.split("/").pop();
  return `<div class="schedule">
    <div class="top"><span class="name">${esc(s.name)}</span><span class="every">${esc(s.every)}</span>${where ? `<span class="chip" title="where its chat works">${esc(s.where)}</span>` : ""}<span class="file">${open(s.file, file)}</span></div>
    <div class="times"><span><b>next</b>${next}</span><span><b>last</b>${esc(s.last ?? "never")}</span></div>
    <div class="line">${esc(s.line.trim())}</div>
    ${allow}
  </div>`;
}

function draw() {
  if (!told) {
    app.innerHTML = `<div class="empty">Cox did not answer with the schedules.</div>`;
    return;
  }
  const own = told.schedules.filter((s) => s.of === "project" && !s.project);
  const theirs = told.schedules.filter((s) => s.of === "workflow");
  const unread = told.unread.map((why) => `<div class="unread">${esc(why)}</div>`).join("");
  let body = unread;
  if (own.length) {
    body += `<div class="group">This folder's<span class="path">.cox/schedules</span></div>${own.map((s) => schedule(s, false)).join("")}`;
  }
  if (theirs.length) {
    body += `<div class="group">Your workflow's<span class="path">${esc(told.workflow)}</span></div>${theirs.map((s) => schedule(s, told.home)).join("")}`;
  }
  // In the home, each project's own, under its name.
  const projects = [...new Set(told.schedules.filter((s) => s.project).map((s) => s.project))];
  for (const project of projects) {
    const of = told.schedules.filter((s) => s.project === project);
    body += `<div class="group">${esc(project)}'s<span class="path">.cox/schedules</span></div>${of.map((s) => schedule(s, false)).join("")}`;
  }
  if (!told.schedules.length) {
    const where = told.home ? told.workflow : `${told.project}`;
    body += `<div class="empty">
      <p>No schedule runs here. A schedule is a Markdown file in <code>${esc(where)}</code>, named after it, whose properties say when it runs, and whose rest is the first line of the chat it opens:</p>
      <pre>---
every: daily 09:00
allow:
  - fetch:*
---

Find out what changed since {last}.</pre>
      <p><code>every</code> is <code>daily 09:00</code>, <code>weekly mon 09:00</code> or <code>every 6h</code>.${told.home ? " A schedule of the workflow with no <code>folder</code> runs here, in the home; the home shows every project's too." : ` A schedule of your workflow whose <code>folder</code> is this one shows here too.`}</p>
    </div>`;
  }
  app.innerHTML = `<div class="head"><h1>Schedules</h1><span class="where">${esc(told.folder)}</span></div>${body}`;
}

load();
setInterval(load, EVERY);
