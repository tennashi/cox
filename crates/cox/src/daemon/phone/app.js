// The phone's page: the projects and chats coxd runs, and a chat's
// conversation, as coxd tells them on /events. Everything the agents
// wrote is put in as text, never as HTML.

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
  failed: document.getElementById("failed"),
  held: document.getElementById("held"),
};
// The line the human sent while the agent was at work, which goes once
// the turn ends, as the window's does.
let held = null;
const head = {
  back: document.getElementById("back"),
  over: document.getElementById("over"),
  title: document.getElementById("title"),
  dot: document.getElementById("dot"),
  lost: document.getElementById("lost"),
};

let listing = null;
let chat = null; // { key, entries, state, steers, title, goal }
let events = null;
let drawing = false;
const opened = new Set(); // the folded groups the human opened, by their first entry

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
  chat = key === null ? null : { key, entries: [], state: null, steers: false, title: null, goal: null, came: false };
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
  } else if (told.Patch && chat && told.Patch.key === chat.key) {
    const patch = told.Patch.patch;
    chat.entries.length = patch.len;
    for (const [at, entry] of patch.entries) chat.entries[at] = entry;
    chat.state = patch.state;
    chat.steers = patch.steers;
    chat.title = patch.title;
    chat.goal = patch.goal;
    chat.came = true;
    if (held !== null && takesLines(chat)) sendHeld();
  } else {
    return;
  }
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

function chatRow(row, child) {
  const stands = standing(row);
  const goal = goalText(row.goal);
  const about = [stands.words, goal && `◎ ${goal}`].filter(Boolean).join(" · ");
  return el("li", { class: row.open ? null : "closed" },
    el("a", { href: `#chat/${row.key}`, class: child ? "child" : null },
      el("span", { class: `dot ${stands.dot}` }),
      el("span", { class: "names" },
        el("span", { class: "name" }, row.parent !== null ? `↳ ${row.name}` : row.name),
        el("span", { class: "about" }, about))));
}

function drawHome() {
  compose.form.hidden = true;
  compose.goal.hidden = true;
  compose.goalButton.hidden = true;
  head.back.hidden = true;
  head.over.textContent = "";
  head.title.textContent = "Cox";
  head.dot.hidden = true;
  document.title = "Cox";
  const shelves = [];
  if (!listing) {
    shelves.push(el("p", { class: "empty" }, "Reaching Cox…"));
  } else {
    const asking = listing.chats.filter((row) => row.state === "Asking" || row.unread);
    if (asking.length) {
      shelves.push(el("section", { class: "shelf asking" },
        el("h2", {}, "Waits for you"),
        el("ul", { class: "chats" }, asking.map((row) =>
          chatRow({ ...row, name: `${row.name}`, parent: null }, false)))));
    }
    listing.projects.forEach((project, index) => {
      const rows = listing.chats.filter((row) => row.project === index);
      if (!rows.length) return;
      shelves.push(el("section", { class: "shelf" },
        el("h2", {}, projectName(index)),
        el("ul", { class: "chats" }, rows.map((row) => chatRow(row, row.parent !== null)))));
    });
    if (!shelves.length) shelves.push(el("p", { class: "empty" }, "No chats yet. Open one on the Mac."));
  }
  main.replaceChildren(...shelves);
}

function drawChat() {
  const row = listing && listing.chats.concat(listing.archived).find((one) => one.key === chat.key);
  head.back.hidden = false;
  head.dot.hidden = !row;
  if (row) {
    const stands = standing({ ...row, state: chat.state || row.state });
    head.over.textContent = `${projectName(row.project)} · ${stands.words}`;
    head.title.textContent = row.name;
    head.dot.className = `dot ${stands.dot}`;
    document.title = row.name;
  } else {
    head.over.textContent = "";
    head.title.textContent = listing ? "No such chat" : "Cox";
  }
  compose.form.hidden = !row;
  compose.goalButton.hidden = !row;
  if (row) drawCompose(row);
  if (listing && !row) {
    main.replaceChildren(el("p", { class: "empty" }, "This chat is not there any more. ",
      el("a", { href: "#" }, "Back to the chats")));
    return;
  }
  const atEnd = window.innerHeight + window.scrollY >= document.body.scrollHeight - 40;
  const agent = ((row && listing.agents && listing.agents[row.agent]) || "Agent").toUpperCase();
  const nodes = [];
  const goal = goalText(chat.goal);
  if (goal) nodes.push(el("div", { class: "goal" }, "◎ ", goal.startsWith("$ ") ? el("code", {}, goal) : goal));
  if (!chat.came) nodes.push(el("p", { class: "empty" }, "Reading the conversation…"));
  nodes.push(...conversation(chat.entries, agent));
  if (chat.state === "Busy" || chat.state === "Checking" || chat.state === "Stopping") {
    nodes.push(el("p", { class: "waits" }, el("span", { class: "dot busy" }),
      chat.state === "Checking" ? "Checking the turn…" : `${titleCase(agent)} is at work…`));
  }
  main.replaceChildren(...nodes);
  if (atEnd) window.scrollTo(0, document.body.scrollHeight);
}

function working(state) {
  return state === "Busy" || state === "Checking" || state === "Stopping";
}

// The input under the conversation: the line to send, or Stop while the
// agent is at work.
function drawCompose(row) {
  const state = chat.state || row.state;
  const busy = working(state);
  compose.stop.hidden = !busy;
  compose.stop.disabled = state === "Stopping";
  compose.line.placeholder = state === "Asking" ? "Answer above first…"
    : busy && chat.steers && state === "Busy" ? "Goes into this turn…"
    : busy ? "Waits for this turn to end…"
      : row.open || row.again ? "A line to the agent…" : "This chat cannot start again";
}

function titleCase(word) {
  return word.charAt(0) + word.slice(1).toLowerCase();
}

// The kind of an entry and what it holds, as serde writes it: a unit
// variant is its name, any other an object of one key.
function kindOf(entry) {
  if (typeof entry === "string") return [entry, null];
  const kind = Object.keys(entry)[0];
  return [kind, entry[kind]];
}

const WORK = new Set(["tool_call", "thought"]);

function conversation(entries, agent) {
  const nodes = [];
  let speaker = null;
  const speak = (who, kind) => {
    if (speaker === who) return;
    speaker = who;
    nodes.push(el("div", { class: `who ${kind}` }, who));
  };
  for (let at = 0; at < entries.length; at++) {
    const entry = entries[at];
    if (!entry) continue;
    const [kind, value] = kindOf(entry);
    if (WORK.has(kind)) {
      const group = [];
      let end = at;
      while (end < entries.length && entries[end] && WORK.has(kindOf(entries[end])[0])) {
        group.push(kindOf(entries[end]));
        end++;
      }
      speak(agent, "agent");
      nodes.push(work(at, group));
      at = end - 1;
      continue;
    }
    switch (kind) {
      case "human":
        speak("YOU", "you");
        nodes.push(el("div", { class: "human" }, value));
        break;
      case "cox":
        speak("COX", "cox");
        nodes.push(el("div", { class: "human cox" }, value));
        break;
      case "attached":
        nodes.push(el("div", { class: "entry checks" }, value.map((one) => el("span", { class: "chip" }, one.label))));
        break;
      case "reply":
        speak(agent, "agent");
        nodes.push(markdown(value));
        break;
      case "plan":
        speak(agent, "agent");
        nodes.push(plan(value));
        break;
      case "permission":
        speak(agent, "agent");
        nodes.push(permission(value, at === lastIndexOf(entries, "permission")));
        break;
      case "proposal":
        speak(agent, "agent");
        nodes.push(proposal(value, at));
        break;
      case "keep":
        speak("COX", "cox");
        nodes.push(el("div", { class: `entry card ${value.kept === null || value.kept === undefined ? "asks" : ""}` },
          el("div", { class: "label" }, "Keep in the wiki?"),
          el("div", { class: "why" }, value.why),
          value.kept === null || value.kept === undefined
            ? buttons([
              ["Save to wiki", { AnswerKeep: { key: chat.key, index: at, keep: true } }, true],
              ["Not now", { AnswerKeep: { key: chat.key, index: at, keep: false } }],
            ])
            : el("p", { class: "hint" }, value.kept ? "You saved it." : "Not now.")));
        break;
      case "checks":
        speaker = null;
        nodes.push(checks("Turn end", value));
        break;
      case "event":
        speaker = null;
        nodes.push(checks(value.on.replace(".", " "), value.checks));
        break;
      case "note":
        speaker = null;
        nodes.push(el("p", { class: "entry note" }, value));
        break;
      case "switched":
        speaker = null;
        nodes.push(el("p", { class: "entry note" }, `Switched from ${value.from} to ${value.to}.`));
        break;
      case "login":
        speaker = null;
        nodes.push(el("div", { class: "entry card asks" },
          el("div", { class: "label" }, "Log in"),
          el("ul", { class: "options" }, value.options.map((option, index) =>
            el("li", { class: value.chosen === index ? "chosen" : null }, option.name))),
          value.failed ? el("p", { class: "hint" }, value.failed) : null,
          value.chosen === null ? el("p", { class: "hint" }, "Log in on the Mac.") : null));
        break;
      default:
        break;
    }
  }
  return nodes;
}

function lastIndexOf(entries, kind) {
  for (let at = entries.length - 1; at >= 0; at--) {
    if (entries[at] && kindOf(entries[at])[0] === kind) return at;
  }
  return -1;
}

function mark(result) {
  if (result === null || result === undefined) return el("span", { class: "mark" });
  if (result === "completed") return el("span", { class: "mark done" });
  if (result === "failed") return el("span", { class: "mark failed" });
  return el("span", { class: "mark stopped" });
}

// The agent's tool calls and thoughts, one after another, folded under
// a line of chips.
function work(first, group) {
  const calls = group.filter(([kind]) => kind === "tool_call").map(([, call]) => call);
  const thoughts = group.filter(([kind]) => kind === "thought").map(([, thought]) => thought);
  if (!calls.length) return el("div", { class: "entry thought" }, thoughts.join("\n\n"));
  const details = el("details", { class: "entry work", open: opened.has(first) });
  details.addEventListener("toggle", () => {
    if (details.open) opened.add(first);
    else opened.delete(first);
  });
  const summary = el("summary", {},
    el("span", { class: "label" }, `${calls.length} ${calls.length === 1 ? "step" : "steps"}`),
    calls.map((call) => el("span", { class: `chip ${call.result === "failed" ? "failed" : ""}` },
      mark(call.result), shorten(call.title, 40))),
    thoughts.length ? el("span", { class: "last-thought" }, shorten(firstLine(thoughts[thoughts.length - 1]), 120)) : null);
  const steps = el("div", { class: "steps" }, group.map(([kind, value]) => {
    if (kind === "thought") return el("div", { class: "step thought" }, value);
    return el("div", { class: "step" },
      el("div", { class: "head" }, mark(value.result), value.title),
      value.content.map(content));
  }));
  details.append(summary, steps);
  return details;
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

function plan(steps) {
  const marks = { completed: "✓", in_progress: "◌", pending: "" };
  return el("ul", { class: "entry plan" }, steps.map((step) =>
    el("li", { class: step.state }, el("span", { class: "state" }, marks[step.state] || ""), step.text)));
}

function permission(asked, last) {
  const waiting = asked.chosen === null || asked.chosen === undefined;
  const asks = waiting && last && chat.state === "Asking";
  return el("div", { class: `entry card ${asks ? "asks" : ""}` },
    el("div", { class: "label" }, asks ? "Asks you" : "Asked"),
    el("div", { class: "what" }, asked.title),
    asked.content.map(content),
    el("ul", { class: "options" }, asked.options.map((option, index) => {
      let how = null;
      if (asks && index === 0) how = "first";
      else if (asked.chosen === index) how = "chosen";
      const label = `${index + 1} · ${option}`;
      if (!asks) return el("li", { class: how }, label);
      return el("li", { class: how }, el("button", { onclick: (event) => choose(event, index) }, label));
    })),
    asked.by_setup ? el("p", { class: "hint" }, "Cox answered in your place, as the chat is set up.") : null);
}

// Answers what the agent asks with the option at `index`, once.
function choose(event, index) {
  for (const button of event.target.closest(".options").querySelectorAll("button")) button.disabled = true;
  act({ Choose: { key: chat.key, index } }).then((done) => {
    if (!done) for (const button of event.target.closest(".options").querySelectorAll("button")) button.disabled = false;
  });
}

// Buttons that each do a message once, all of them off while it goes.
function buttons(list) {
  const row = el("div", { class: "answers" });
  for (const [label, message, first] of list) {
    row.append(el("button", {
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

function proposal(proposed, index) {
  const answers = { set: "You set it.", changed: "You changed it, then set it.", dismissed: "You let it be." };
  return el("div", { class: `entry card ${proposed.answer ? "" : "asks"}` },
    el("div", { class: "label" }, "Proposes a goal"),
    el("div", { class: "what" }, proposed.goal),
    proposed.why ? el("div", { class: "why" }, proposed.why) : null,
    proposed.answer ? el("p", { class: "hint" }, answers[proposed.answer]) : buttons([
      ["Set goal", { AnswerProposal: { key: chat.key, index, answer: "set" } }, true],
      ["Dismiss", { AnswerProposal: { key: chat.key, index, answer: "dismissed" } }],
    ]));
}

function checks(title, list) {
  return el("div", { class: "entry checks" },
    el("span", { class: "label" }, title),
    list.map((check) => {
      const state = { waiting: "", running: "", passed: "done", failed: "found", broken: "failed" }[check.state];
      return el("span", { class: "check", title: check.said || null }, el("span", { class: `mark ${state}` }), check.name);
    }));
}

function firstLine(text) {
  return text.trim().split("\n")[0];
}

function shorten(text, most) {
  return text.length > most ? `${text.slice(0, most - 1)}…` : text;
}

// Markdown, as far as a phone needs it: paragraphs, headings, lists,
// quotes, code, and inline code, bold, italics and links.
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
      root.append(el("pre", {}, el("code", {}, code.join("\n"))));
      continue;
    }
    if (!line.trim()) { at++; continue; }
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
    while (at < lines.length && lines[at].trim() && !/^\s*(```|~~~|#{1,6}\s|>|[-*+]\s|\d+[.)]\s)/.test(lines[at])) {
      paragraph.push(lines[at++]);
    }
    if (!paragraph.length) paragraph.push(lines[at++]);
    root.append(el("p", {}, inline(paragraph.join(" "))));
  }
  return root;
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
  opened.clear();
  compose.line.value = "";
  hold(null);
  compose.goal.hidden = true;
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
