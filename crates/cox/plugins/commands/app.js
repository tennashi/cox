// What the agents of the folder's chats run, drawn by a page of the stage:
// read as JSON from `cox://api/commands`, and again every second. Each
// chat that ran commands, those at work first, each a link to the chat;
// under it its last steps, the commands the agent ran one after another,
// headed by what it said before them. A step folds once all its commands
// ended, but the last; a click on its heading folds or unfolds it.
//
// A command shows as what the agent said it is for, with the gist of the
// command beside it, a click away from all of it laid out to read (see
// `stage::commands::layout`): a row for each command of a chain, the
// bodies of loops indented, what it feeds a program folded. One that ran
// long, as a test or a build, shows big, laid out, with what it printed
// once it ended, so that what the agent waits on stands out.

const API = "cox://api/commands";
const EVERY = 1000;

const app = document.getElementById("app");
let told = null;
let toldText = null;
// The steps the human folded or unfolded, by their first command's id,
// and the commands and folds they opened or closed, as they chose.
const chosen = new Map();
// The commands drawn, by their key, for the copy button to copy as the
// agent gave them.
const lines = new Map();

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

const isOpen = (key, otherwise) => (chosen.has(key) ? chosen.get(key) : otherwise);

// A stretch of code, colored as the code tabs color it.
const stretch = ([text, what]) => (what ? `<span class="hl-${what.replace(/\./g, "-")}">${esc(text)}</span>` : esc(text));

// The rows of a command laid out, or the command as one row when Cox did
// not lay it out.
const rowsOf = (command) => command.shown ?? [{ depth: 0, joint: null, code: [[command.line, null]] }];

// The row that says most of what a command does: the first that is not
// a `cd`, nor only what sets up the environment.
function gist(command) {
  const rows = rowsOf(command);
  const said = rows.find((row) => {
    const text = row.code.map((piece) => (piece.fold ? "" : piece[0])).join("").trim();
    return !/^cd(\s|$)/.test(text) && !row.code.every((piece) => piece.fold || piece[1] === "quiet" || !piece[0].trim());
  });
  return { row: said ?? rows[0], more: rows.length - 1 };
}

const linesOf = (n) => (n === 1 ? "1 line" : `${n} lines`);

function laidOut(command, key) {
  const rows = rowsOf(command)
    .map((row, r) => {
      const bodies = [];
      const code = row.code
        .map((piece, p) => {
          if (!piece.fold) return stretch(piece);
          const foldKey = `${key}/${r}/${p}`;
          const open = isOpen(foldKey, false);
          const { language, lines } = piece.fold;
          if (open) bodies.push(lines);
          const label = `${language ? `${language} · ` : ""}${linesOf(lines.length)}`;
          return `<span class="fold" data-key="${esc(foldKey)}" data-open="${open}" title="${open ? "folds" : "shows"} the text">${open ? "▾" : "▸"} ${esc(label)}</span>`;
        })
        .join("");
      const shown = bodies
        .map((lines) => `<div class="body" style="--depth:${row.depth}">${lines.map((line) => `<div>${line.map(stretch).join("") || " "}</div>`).join("")}</div>`)
        .join("");
      return (
        `<div class="row" style="--depth:${row.depth}"><span class="joint">${esc(row.joint ?? "")}</span>` +
        `<span class="code">${code}</span></div>${shown}`
      );
    })
    .join("");
  return `<div class="laid"><button class="copy" data-key="${esc(key)}" title="copies the command as the agent gave it">Copy</button>${rows}</div>`;
}

function small(chat, command) {
  const key = `command/${chat.key}/${command.id}`;
  lines.set(key, command.line);
  const open = isOpen(key, false);
  const { row, more } = gist(command);
  const code = `<span class="gist">${row.code.map((piece) => (piece.fold ? "…" : stretch(piece))).join("")}</span>`;
  const head = command.about ? `<span class="about">${esc(command.about)}</span>${code}` : code;
  return (
    `<div class="small${open ? " open" : ""}" data-key="${esc(key)}" data-open="${open}" title="${open ? "folds the command" : "shows all of the command"}">` +
    `<span class="mark ${command.state}">${MARKS[command.state]}</span>` +
    `<span class="line">${head}</span>` +
    `<span class="more">${more > 0 && !open ? `+${more}` : ""}</span>` +
    `<span class="secs">${took(command.secs)}</span></div>` +
    (open ? `<div class="under">${laidOut(command, key)}</div>` : "")
  );
}

function big(chat, command) {
  const key = `command/${chat.key}/${command.id}`;
  lines.set(key, command.line);
  const state =
    command.state === "running"
      ? `running · ${took(command.secs)}`
      : `${command.state}${command.secs ? ` in ${took(command.secs)}` : ""}`;
  const about = command.about ?? command.why;
  const body =
    command.printed != null
      ? `<pre>${esc(command.printed) || "(printed nothing)"}</pre>`
      : command.state === "running"
        ? `<div class="why">What it prints shows once it ends.</div>`
        : "";
  return (
    `<div class="big ${command.state}"><div class="top"><span class="about">${about ? esc(about) : "A command"}</span>` +
    `<span class="state ${command.state}">${state}</span></div>${laidOut(command, key)}${body}</div>`
  );
}

function step(chat, step, last) {
  const key = `step/${chat.key}/${step.id}`;
  const unended = step.commands.some((c) => c.state === "running" || c.state === "failed");
  const open = isOpen(key, last || unended);
  const ran = step.commands.length === 1 ? "1 command" : `${step.commands.length} commands`;
  const all = step.commands.every((c) => c.state === "done") ? " · all done" : "";
  return (
    `<div class="step"><div class="said" data-key="${esc(key)}" data-open="${open}">` +
    `<span class="fold-mark">${open ? "▾" : "▸"}</span>` +
    `<span class="text">${esc(step.said ?? "Before saying anything")}</span>` +
    `<span class="count">${ran}${open ? "" : all}</span></div>` +
    (open ? step.commands.map((c) => (c.long || c.printed != null ? big(chat, c) : small(chat, c))).join("") : "") +
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
  lines.clear();
  const running = told.chats.flatMap((c) => c.steps.flatMap((s) => s.commands)).filter((c) => c.state === "running").length;
  const said = running === 1 ? "1 command runs" : running ? `${running} commands run` : "nothing runs";
  app.innerHTML =
    `<div class="head"><h1>Commands</h1><span class="summary">${said}</span><span class="where" title="${esc(told.folder)}">${esc(told.folder)}</span></div>` +
    (told.chats.length
      ? told.chats.map(chat).join("")
      : `<div class="none">No agent ran a command here yet. What the agents of the chats here run shows as they run it.</div>`);
}

// Copies `text`: through the page's clipboard, or, where the page may not
// write to it, as a selection is copied.
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

app.addEventListener("click", (event) => {
  const button = event.target.closest(".copy");
  if (button) {
    const text = lines.get(button.dataset.key);
    if (text == null) return;
    copy(text).then(() => {
      button.textContent = "Copied";
      setTimeout(() => (button.textContent = "Copy"), 1200);
    });
    return;
  }
  const toggled = event.target.closest(".said, .small, .fold");
  if (!toggled) return;
  chosen.set(toggled.dataset.key, toggled.dataset.open !== "true");
  draw();
});

load();
setInterval(load, EVERY);
