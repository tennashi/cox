// A map of the project, drawn by a page of the stage: one of the
// project's maps, each a viewpoint of it, as coxd draws it from the map's
// vocabulary, a file of the project's `maps`, and the indexes of its
// code, read as JSON from `cox://api/map/<name>` and read again every few
// seconds, as Cox's own tabs follow what they show: what changed in the
// code falls on its boxes as it changes, and the map is drawn anew once
// an index of the code checked out is made.
//
// Where the page is open is its hash: the map's name, encoded, and `:`,
// unless it is the project's first, then the nodes opened from the top
// down, and last the node or the function chosen, each by its id,
// encoded, between `/`s, as `#use:cox/coxd`, `#cox%2Fuse:cox/coxd` for a
// repository's map, or `#cox/coxd/runner/cox%2Fdaemon%2Fchat%2FChat`.
// The last alone will do: the boxes above it open for it. A box that
// opens is opened, one that does not is chosen.
//
// The map itself, megabytes of JSON for a large project, is read only as
// it changes: the page gives back the tag of the map it has, and is told
// the rest alone, as what changed in the code, while the map is the same.
// Each tool of the browser's for developers attached to the page, as the
// one each agent's chat drives the stage with, keeps what the page read,
// a few hundred megabytes of it each, so that the map read whole every
// few seconds filled the page's memory by gigabytes.
//
// The page draws from the JSON alone: on the phone, where nothing else
// of Cox's is there, it is the same, but for the JSON's address. On the
// stage, a file of the code opens on a tab of its own; on the phone, its
// place is told as text.

import { afterRender, canOpen, chain, esc, goTo, node, parentOf, render, setMap, view, whenStill } from "./canvas.js";
import { chosenFunction, jump, setTypes } from "./detail.js";
import { setDrift, show as showDrift } from "./drift.js";

// Where the map's JSON is: on the stage, Cox's own address; elsewhere,
// as on the phone, beside the page, asked with the page's own query,
// which names the chat whose project it is, as `?kept=<n>`.
const onStage = location.protocol === "cox:";
const SOURCE = onStage ? "cox://api/map" : new URL("api/map", location.href).href;
const QUERY = onStage ? "" : location.search;
// How often the map is read again: often while an index is made, to
// draw it once it is, and less often after, for what changed in the code.
const EVERY = 3000;
const EVERY_READY = 6000;

const note = document.getElementById("note");
const empty = document.getElementById("empty");
const crumb = document.getElementById("crumb");

let told = null;
let lastText = null;
// The tag of the map drawn, which coxd leaves the map out for while it is
// the same.
let tag = null;
// The map the page has drawn, by its name, to tell a new one from the
// same drawn again.
let drawn = null;
// Where the hash asks the page to be once its map is drawn.
let wanted = null;

// A part of the hash, decoded; as it is, when it is no encoding.
const decoded = (part) => {
  try {
    return decodeURIComponent(part);
  } catch {
    return part;
  }
};

// The hash: the map named, if it is, and the ids after it.
function parsed(hash) {
  const place = (hash || "").replace(/^#/, "");
  const colon = place.indexOf(":");
  const map = colon < 0 ? null : decoded(place.slice(0, colon));
  const rest = place.slice(colon + 1);
  const at = rest ? rest.split("/").map(decoded).filter(Boolean) : [];
  return { map, at };
}
/// The map the page is to show: the one the hash names, or else the
/// project's first.
const mapAsked = () => parsed(location.hash).map;
const address = (again) => {
  const name = mapAsked() ?? (again ? told?.name : null);
  const path = name ? `${SOURCE}/${again ? "again/" : ""}${encodeURIComponent(name)}` : SOURCE;
  // Another map's tag is not this one's: coxd tells all of it.
  const query = new URLSearchParams(QUERY);
  if (tag) query.set("seen", tag);
  const asked = query.toString();
  return asked ? `${path}?${asked}` : path;
};

// How many times the page asked for the map: only the last ask's answer
// is drawn.
let asks = 0;

/// Reads the map, and draws it when it changed.
async function load(again = false) {
  const ask = ++asks;
  const map = mapAsked();
  let text;
  try {
    const response = await fetch(address(again));
    text = await response.text();
  } catch (error) {
    text = JSON.stringify({ state: "unanswered", problems: [String(error)] });
  }
  // Overtaken by a later ask, or of a map the page has left meanwhile,
  // the answer is no longer the page's.
  if (ask !== asks || mapAsked() !== map) return;
  if (text === lastText) return;
  let json;
  try {
    json = JSON.parse(text);
  } catch (error) {
    json = { state: "unanswered", problems: [String(error)] };
  }
  if (json.tag && !json.map) {
    // Of a map the page has no more, as one it left meanwhile: asked
    // again, without a tag, it is told whole.
    if (json.tag !== tag || !told?.map) {
      tag = null;
      lastText = null;
      return load(again);
    }
    json.map = told.map;
  }
  lastText = text;
  tag = json.map ? (json.tag ?? null) : null;
  shown(json);
}

// Opens the map where `at` says: the last id's way down, opening it if it
// opens, choosing it if not, and opening the function it is, if it is one;
// once a level opening or closing meanwhile is done.
function goWhere(at) {
  whenStill(() => goWhereNow(at));
}

function goWhereNow(at) {
  const map = told.map;
  if (!map) return;
  const last = at.at(-1);
  let id = last;
  let fn = null;
  if (last && !map.nodes[last]) {
    const sep = last.lastIndexOf("::");
    const ty = sep < 0 ? null : map.types[last.slice(0, sep)];
    if (ty && ty.functions.some((f) => f.key === last)) {
      id = ty.id;
      fn = last;
    } else id = null;
  }
  if (!id) goTo("root");
  else if (canOpen(id)) goTo(id);
  else {
    const way = chain(id);
    goTo(way.length > 1 ? way.at(-2) : "root", id);
  }
  if (fn) jump(fn);
}

// The hash of where the page is now.
function fragment() {
  const first = told.maps[0]?.name;
  const name = mapAsked() !== null || told.name !== first ? `${encodeURIComponent(told.name)}:` : "";
  const ids = view.stack.slice(1);
  if (view.sel) ids.push(view.sel);
  const fn = chosenFunction();
  if (fn) ids.push(fn);
  return `#${name}${ids.map(encodeURIComponent).join("/")}`;
}
afterRender(() => {
  if (!told?.map) return;
  const hash = fragment();
  if (hash !== location.hash && !(hash === "#" && !location.hash)) history.replaceState(null, "", hash);
  bar();
});

// The bar: the project, then the map's tabs at the top, or the levels
// opened below it.
function bar() {
  const project = `<b>${esc(told.project)}</b><span>›</span>`;
  const label = (m) => m.says || m.name;
  if (view.stack.length <= 1) {
    crumb.innerHTML =
      project +
      told.maps
        .map(
          (m) =>
            `<button class="tab${m.name === told.name ? " on" : ""}" data-map="${esc(m.name)}" title="${esc(m.says || m.name)}">${esc(label(m))}</button>`,
        )
        .join("");
    return;
  }
  const here = told.maps.find((m) => m.name === told.name);
  crumb.innerHTML =
    project +
    `<button data-up="0">${esc(here ? label(here) : told.name)}</button>` +
    view.stack
      .slice(1)
      .map((f, i) =>
        i === view.stack.length - 2
          ? `<span>›</span><span class="here">${esc(node(f).name)}</span>`
          : `<span>›</span><button data-up="${i + 1}">${esc(node(f).name)}</button>`,
      )
      .join("");
}

// How each index of the code stands, a line for each that is not ready.
function noteLines() {
  const commit = (c) => (c ? `<span class="mono">${esc(c.slice(0, 7))}</span>` : "");
  return (told.indexes ?? [])
    .filter((x) => x.state !== "ready")
    .map((x) => {
      const code = `${x.language ? `The ${esc(x.language)} code` : "The code"}${x.directory ? ` of ${esc(x.directory)}` : ""}`;
      const tool = esc(x.indexer ?? "its indexer");
      const extra = x.note ? ` ${esc(x.note)}` : "";
      switch (x.state) {
        case "indexing": {
          const minutes = Math.max(0, Math.round((Date.now() / 1000 - x.since) / 60));
          const since = minutes < 60 ? (minutes ? `${minutes} min` : "") : `${Math.round(minutes / 60)} h`;
          const meanwhile = x.drawn
            ? `the map is drawn from ${commit(x.drawn)} meanwhile`
            : "its boxes show their types once it is done";
          return `<div class="line indexing"><span class="dot"></span><span class="text">${code} at ${commit(x.commit)} is being indexed by ${tool}${since ? ` (${since})` : ""}: ${meanwhile}.${extra}</span></div>`;
        }
        case "failed":
          return `<div class="line failed"><span class="dot"></span><span class="text">${code} at ${commit(x.commit)} could not be indexed by ${tool}: <span class="mono">${esc(x.reason)}</span></span><button class="again" data-again>Try again</button></div>`;
        case "missing":
          return `<div class="line missing"><span class="dot"></span><span class="text">${code} is not in the map: ${esc(x.how)}</span></div>`;
        case "unknown":
          return `<div class="line"><span class="dot"></span><span class="text">${code} is not in the map: Cox indexes ${esc((x.known ?? []).join(", "))} only.</span></div>`;
        default:
          return `<div class="line"><span class="dot"></span><span class="text">${code} is not in the map: ${esc(x.why ?? x.state)}</span></div>`;
      }
    })
    .join("");
}

// Where there is no map to draw, why.
function why() {
  const problems = (told.problems ?? []).map((p) => `<li>${esc(p)}</li>`).join("");
  if (told.state === "invalid" && told.name)
    return `<div class="card"><h2>The map <code>${esc(told.name)}</code> does not read</h2><p>Its vocabulary, <code>${esc(relative(told.file))}</code>, has these problems. Ask the agent to fix it, or fix it yourself: the map is drawn again as it is saved.</p><ul>${problems}</ul></div>`;
  if (told.state === "none" && told.maps?.length)
    return `<div class="card"><h2>No such map</h2><p>The project has ${told.maps.map((m) => `<code>${esc(m.name)}</code>`).join(", ")}.</p><ul>${problems}</ul></div>`;
  if (told.state === "none")
    return `<div class="card"><h2>The project has no map yet</h2><p>A map is a file of the project, <code>maps/&lt;name&gt;.json</code> in its folder or in one of its repositories', which names its parts and the code each owns; Cox reads the rest from the code. Ask the agent to make one with you: its <code>show_map</code> tool tells it how.</p></div>`;
  return `<div class="card"><h2>The map could not be drawn</h2><ul>${problems}</ul></div>`;
}
const relative = (file) => (file && told.root && file.startsWith(`${told.root}/`) ? file.slice(told.root.length + 1) : (file ?? ""));

// Lays the bars over the drawing: where the drawing starts under them.
function layout() {
  const lines = told.map ? noteLines() : "";
  note.innerHTML = lines;
  note.hidden = !lines;
  const below = 44 + (note.hidden ? 0 : note.offsetHeight);
  document.documentElement.style.setProperty("--top", `${below}px`);
}

/// Shows what coxd told of the map.
function shown(json) {
  const was = drawn;
  told = json;
  told.maps ??= [];
  empty.hidden = Boolean(told.map);
  if (!told.map) {
    for (const pane of ["dt", "fl", "dr"]) document.getElementById(pane).classList.remove("on");
    view.sel = null;
    empty.innerHTML = why();
    crumb.innerHTML = told.project ? `<b>${esc(told.project)}</b>` : "";
    note.hidden = true;
    document.getElementById("dfb").hidden = true;
    drawn = null;
    layout();
    return;
  }
  const map = told.map;
  setMap(map, told.changed);
  setTypes(map, told.root, onStage);
  setDrift(map, relative(told.file), told.state === "ready");
  layout();
  if (was !== told.name) {
    // A map drawn first: where the hash says, if it says.
    drawn = told.name;
    const at = wanted ?? parsed(location.hash).at;
    wanted = null;
    view.stack = ["root"];
    view.sel = null;
    goWhere(at);
    return;
  }
  // The same map drawn again: the page stays where it is, as far as what
  // it shows is still there.
  const stack = ["root"];
  for (const f of view.stack.slice(1)) {
    if (!canOpen(f) || parentOf(f) !== stack.at(-1)) break;
    stack.push(f);
  }
  view.stack = stack;
  if (view.sel && !map.nodes[view.sel]) view.sel = null;
  render();
}

document.addEventListener("click", (e) => {
  const tab = e.target.closest("[data-map]");
  if (tab && tab.dataset.map !== told?.name) {
    const first = told.maps[0]?.name;
    history.replaceState(null, "", tab.dataset.map === first ? "#" : `#${encodeURIComponent(tab.dataset.map)}:`);
    lastText = null;
    load();
  }
  if (e.target.closest("[data-again]")) {
    lastText = null;
    load(true);
  }
});
addEventListener("hashchange", () => {
  const { map, at } = parsed(location.hash);
  if ((map ?? told?.maps?.[0]?.name) !== told?.name) {
    wanted = at;
    lastText = null;
    load();
  } else if (told?.map) {
    showDrift(false);
    goWhere(at);
  }
});
addEventListener("resize", () => told && layout());

async function follow() {
  try {
    if (!document.hidden) await load();
  } catch (error) {
    // What breaks drawing one answer does not stop the page reading the next.
    console.error(error);
  }
  setTimeout(follow, told?.state === "indexing" ? EVERY : EVERY_READY);
}
follow();

