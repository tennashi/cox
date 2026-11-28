// A type of the code, chosen in the level inside the box that owns it:
// on the right, where it is used and its functions, each of which opens
// on where it is called from, what it calls and the fields it uses; on
// the left, its fields, each opening on where it is used. A function
// opened lights the fields it uses on the left, the others folding into
// one row, the uses of each by that function, and on the map takes what
// it relates to in the open box to an area at its top. A function's name,
// wherever it is, goes to it. In a map with drivers, the type's head
// names the drivers that reach it, whichever one's slice is drawn. On a
// narrow screen, all of it is in a drawer from below.
//
// The map comes without its types' details, megabytes of a large
// project's: the detail of a type is read as the type is chosen, and
// kept while the map is the same.

import { ICON } from "./icons.js";
import { afterRender, draw, esc, fitAbove, fitAll, focus, goTo, moving, node, placed, render, view, whenStill } from "./canvas.js";

const dt = document.getElementById("dt");
const fl = document.getElementById("fl");

let types = {};
// The details read, by their type's id: each with the tag of the map it
// was asked of, its type, and what the fields of other types its
// functions use name, by the field's key.
const details = new Map();
// Those being read, by their type's id, and those that did not read,
// which are read again once their type is chosen again.
const reading = new Map();
const failed = new Set();
// The tag of the map, and what reads a type's detail of it.
let tag = null;
let read = async () => null;
// The names of the drivers whose slices show each type, in the map's
// order: empty for a map with no drivers.
let reachedFrom = {};
// Where a file opens on a tab of the stage, where there is one.
let opens = null;

/// Reads the types of `map`, tagged `of`, whose details `reader` reads,
/// each by its type's id, with the files of the project in `root`
/// opening on the stage when `link` says they can.
export function setTypes(map, of, reader, root, link) {
  types = map.types;
  read = reader;
  if (of !== tag) {
    // Another map's details are not this one's: the one shown stays
    // until it is read again.
    tag = of;
    reading.clear();
    failed.clear();
    for (const id of [...details.keys()]) if (id !== shownFor) details.delete(id);
  }
  reachedFrom = {};
  for (const slice of map.slices ?? []) {
    const name = map.nodes[slice.driver].name;
    for (const level of Object.values(slice.levels)) for (const id in level.parts) if (id in types) (reachedFrom[id] ??= []).push(name);
  }
  opens = link ? (file, line) => `cox://open${root}/${file}#L${line}` : null;
  stale = true;
}

/// The type whose member `key` is: a member's key is its type's id, `::`
/// or `.`, and its name.
export function ownerOf(key) {
  for (let at = key.length; at > 0; ) {
    at = Math.max(key.lastIndexOf("::", at - 1), key.lastIndexOf(".", at - 1));
    if (at > 0 && types[key.slice(0, at)]) return key.slice(0, at);
  }
  return null;
}

// The detail of the type `id` as it was read last, if it was; read again
// when it is of a map the page has no more.
function detailOf(id) {
  const detail = details.get(id);
  if (detail?.tag !== tag && !failed.has(id)) want(id);
  return detail ?? null;
}

// Reads the detail of the type `id` of the map as it is now, once however
// often it is asked for meanwhile, and draws it if its type is chosen
// still. Answers it, or nothing when it could not be read.
function want(id) {
  if (details.get(id)?.tag === tag) return Promise.resolve(details.get(id));
  if (reading.has(id)) return reading.get(id);
  const of = tag;
  const asked = read(id).then((detail) => {
    // Read of a map the page has left meanwhile, it is no longer asked for.
    if (of !== tag) return null;
    reading.delete(id);
    if (detail) details.set(id, { tag: of, type: detail.type, to: detail.to ?? {}, text: JSON.stringify([detail.type, detail.to]) });
    else failed.add(id);
    whenStill(() => {
      if (view.sel !== id) return;
      stale = true;
      render();
    });
    return detail ? details.get(id) : null;
  });
  reading.set(id, asked);
  return asked;
}

// The function keyed `key` and its type, once its type's detail is read.
function functionOf(key) {
  const detail = details.get(ownerOf(key));
  const f = detail?.type.functions.find((one) => one.key === key);
  return f ? { detail, t: detail.type, f } : null;
}

const narrow = () => matchMedia("(max-width:700px)").matches;
// How much of the drawing the left card and the right panel cover.
const LEFT = 324;
const RIGHT = 420;

// A file and a line, short, opening on the stage where it can.
function place(file, line) {
  const short = file.replace(/^crates\//, "").replace(/^([^/]+\/)src\//, "$1").replace(/^src\//, "");
  const text = `${esc(short)}:${line}`;
  return opens ? `<a href="${esc(opens(file, line))}" title="${esc(file)}">${text}</a>` : `<span title="${esc(file)}">${text}</span>`;
}

// A function's name, which goes to it when it is one of the map's.
const fnSpan = (key, shown) =>
  key && ownerOf(key) ? `<span class="f jump" data-jump="${esc(key)}">${esc(shown)}</span>` : `<span class="f">${esc(shown)}</span>`;

const chip = (nd, self) => (nd && nd !== self ? `<span class="chip">${esc(node(nd).name)}</span>` : "");

// Where something is used, a row for each function, however often that
// uses it, and the tests apart.
function usesHTML(uses, self) {
  const real = uses.filter((u) => !u.test);
  const tests = uses.length - real.length;
  const by = new Map();
  for (const u of real) {
    const k = u.fn ?? `${u.module}|${u.in}`;
    if (!by.has(k)) by.set(k, { ...u, n: 0, writes: false });
    const x = by.get(k);
    x.n++;
    x.writes ||= Boolean(u.writes);
  }
  const rows = [...by.values()]
    .map(
      (u) =>
        `<div class="u" data-in="${esc(u.fn ?? u.in)}" data-node="${esc(u.node ?? "")}">${fnSpan(u.fn, u.in)}${u.writes ? '<span class="tag w">writes</span>' : ""}${u.n > 1 ? `<span class="tag">${u.n}</span>` : ""}<span class="at">${place(u.file, u.line)}</span>${chip(u.node, self)}</div>`,
    )
    .join("");
  return (
    (rows || '<div class="none">Not used outside tests</div>') +
    (tests ? `<div class="none">${tests} place${tests === 1 ? "" : "s"} in tests</div>` : "")
  );
}

// How many functions use something, outside tests: in the accent when
// some are another node's.
function count(uses, self) {
  const real = uses.filter((u) => !u.test);
  const fns = new Set(real.map((u) => u.fn ?? u.module + u.in));
  const out = real.some((u) => u.node && u.node !== self);
  return `<span class="n${out ? " out" : ""}" title="Used in ${fns.size} function${fns.size === 1 ? "" : "s"} outside tests">${fns.size} fn</span>`;
}

// What a function opened calls, and the fields it uses, by their type.
function callsHTML(calls, self) {
  const fns = calls.filter((c) => c.kind === "function");
  const flds = calls.filter((c) => c.kind === "field");
  const by = new Map();
  for (const c of flds) {
    const dot = c.name.lastIndexOf(".");
    const owner = dot < 0 ? c.name : c.name.slice(0, dot);
    const name = dot < 0 ? c.name : c.name.slice(dot + 1);
    if (!by.has(owner)) by.set(owner, { node: c.node, fs: [] });
    by.get(owner).fs.push({ name, writes: c.writes, key: c.key });
  }
  const fr = fns
    .map(
      (c) =>
        `<div class="u" data-node="${esc(c.node ?? "")}">${fnSpan(c.key, c.name)}${c.count > 1 ? `<span class="tag">${c.count}</span>` : ""}<span class="at">:${c.line}</span>${chip(c.node, self)}</div>`,
    )
    .join("");
  const dr = [...by]
    .map(
      ([owner, g]) =>
        `<div class="u wraps" data-node="${esc(g.node ?? "")}"><span class="f">${esc(owner)}</span>${chip(g.node, self)}<span class="break"></span>${g.fs
          .map(
            (x) =>
              `<span class="fc${x.writes ? " w" : ""}${x.key ? "" : " other"}"${x.key ? ` data-go="${esc(x.key)}"` : ""}>${esc(x.name)}</span>`,
          )
          .join("")}</div>`,
    )
    .join("");
  return `<div class="sub">Calls ${fns.length}</div>${fr || '<div class="none">None</div>'}<div class="sub">Uses fields ${flds.length}</div>${dr || '<div class="none">None</div>'}`;
}

// A member's signature, but for a field's name, which its row shows.
const signature = (m, isFunction) => (!isFunction && m.sig.startsWith(`${m.name}: `) ? m.sig.slice(m.name.length + 2) : m.sig);

// The members of `t`, of `list`, its fields or its functions, under `title`.
function members(t, title, list, isFunction) {
  if (!list.length) return "";
  return (
    `<h4>${title}</h4>` +
    list
      .map(
        (m, i) =>
          `<div class="m ${isFunction ? "fn" : "fld"}" data-m="${isFunction ? "fn" : "fld"}${i}" ${isFunction ? `data-fn="${esc(m.key)}"` : `data-f="${esc(m.key)}"`}><div class="row"><span class="nm">${esc(m.name)}</span>${isFunction ? "" : '<span class="tag w hw">writes</span>'}<span class="sig">${esc(signature(m, isFunction))}</span>${count(m.uses, t.id)}</div><div class="us">${
            isFunction
              ? `<div class="sub">Called from</div>${usesHTML(m.uses, t.id)}${callsHTML(m.calls, t.id)}`
              : usesHTML(m.uses, t.id)
          }</div></div>`,
      )
      .join("")
  );
}

// The type the detail is of, what it was drawn from, and whether the map
// was read again since.
let shownFor = null;
let shownFrom = null;
let stale = false;
let actFn = null;
let focusMark = new Set();

/// The function opened in the detail, if one is.
export const chosenFunction = () => actFn;

// The detail of the type chosen, if one is, drawn once for each.
function panel() {
  const id = view.sel && types[view.sel] && !moving() ? view.sel : null;
  dt.classList.toggle("on", Boolean(id));
  fl.classList.toggle("on", Boolean(id));
  if (id === shownFor && !stale) return;
  // The map read again with this type as it was: the detail stays as the
  // human left it.
  const reached = (id && reachedFrom[id]) || [];
  if (id !== shownFor) failed.delete(id);
  const detail = id ? detailOf(id) : null;
  const from = id ? `${narrow()}|${reached.join()}|${JSON.stringify(types[id])}|${detail?.text ?? failed.has(id)}` : null;
  if (id === shownFor && from === shownFrom) {
    stale = false;
    return;
  }
  const was = shownFor;
  const again = stale && id === was ? actFn : null;
  const kept = id && id === was ? keep() : null;
  shownFrom = from;
  const lit = view.mark.size > 0 || view.focus;
  shownFor = id;
  stale = false;
  view.mark = new Set();
  actFn = null;
  focus(null);
  focusMark = new Set();
  // What the function that was open lit goes dark.
  if (lit) draw();
  dt.classList.remove("focus");
  fl.classList.remove("focus");
  // The panels cover the drawing's sides: the map moves aside, and back.
  if (Boolean(id) !== Boolean(was)) {
    if (narrow()) {
      if (id) fitAbove(id);
      else fitAll();
    } else if (id) fitAbove(id, LEFT, RIGHT);
    else fitAll();
    if (!id) dt.classList.remove("full");
  }
  if (!id) return;
  const head = (t) =>
    `<div class="grip"></div><div class="hd"><div class="ttl"><span class="ic"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="1.6" stroke-linecap="round">${ICON.braces}</svg></span><b>${esc(t.name)}</b><span class="k">${esc(t.kind)}</span></div>
  <div class="k where">${esc(t.id)} · ${place(t.file, t.line)}</div>${reached.length ? `<div class="reached">Reached from: ${esc(reached.join(", "))}</div>` : ""}${t.doc ? `<div class="doc">${esc(t.doc)}</div>` : ""}</div>`;
  if (!detail) {
    // Its detail is being read: what the map says of it meanwhile.
    const says = failed.has(id) ? "Where it is used could not be read. Choose it again to read it." : "Reading where it is used…";
    fl.innerHTML = "";
    dt.innerHTML = `${head(types[id])}<div class="m open"><div class="us"><div class="none">${says}</div></div></div>`;
    return;
  }
  const t = detail.type;
  const fieldsTitle = t.kind === "enum" ? "Variants" : "Fields";
  const fieldsHTML = members(t, fieldsTitle, t.fields, false);
  fl.innerHTML = narrow()
    ? ""
    : `<div class="fh"><b>${esc(t.name)}</b><span>${fieldsTitle} ${t.fields.length}</span></div>${members(t, "", t.fields, false).replace(/^<h4><\/h4>/, "")}<div class="rest" data-rest></div>`;
  dt.innerHTML = `${head(t)}
  <h4>Where it is used</h4><div class="m open"><div class="us">${usesHTML(t.uses, id)}</div></div>
  ${narrow() ? fieldsHTML : ""}
  ${members(t, "Functions", t.functions, true)}`;
  // Drawn again, the function that was open stays open, and so do the
  // rows that were, where the panels were scrolled to.
  if (again && functionOf(again)?.t.id === id) {
    const row = [...dt.querySelectorAll(".m.fn")].find((x) => x.dataset.fn === again);
    row?.classList.add("open");
    focusFn(again);
  }
  if (kept) restore(kept);
}

// A row of a member, by the pane it is in and the member's key.
const rowKey = (row) => `${row.closest(".pane").id}|${row.dataset.f ?? row.dataset.fn}`;

// How the panels stand: the rows open, the unused fields unfolded, and
// where each is scrolled to.
function keep() {
  return {
    open: new Set([...document.querySelectorAll(".pane .m.open[data-m]")].map(rowKey)),
    rest: fl.classList.contains("rest"),
    dt: dt.scrollTop,
    fl: fl.scrollTop,
  };
}

function restore(kept) {
  document.querySelectorAll(".pane .m[data-m]").forEach((row) => {
    if (kept.open.has(rowKey(row))) row.classList.add("open");
  });
  if (kept.rest && fl.classList.contains("focus")) {
    fl.classList.add("rest");
    const rest = fl.querySelector("[data-rest]");
    if (rest) rest.textContent = `Fold the ${rest.dataset.n} field${rest.dataset.n === "1" ? "" : "s"} it does not use`;
  }
  dt.scrollTop = kept.dt;
  fl.scrollTop = kept.fl;
}

// What a function relates to, each as a link from a node to a node: the
// nodes that call it, outside tests, then, in the order it does, the
// nodes it calls and whose fields it uses, and the nodes those fields'
// types name. `nodes` are the other ends, lit as its type is.
function reach(fn) {
  const { detail, t: own, f } = functionOf(fn);
  // What a field names: the type's own tell it themselves.
  const named = (key) => own.fields.find((field) => field.key === key)?.to ?? detail.to[key] ?? [];
  const links = [];
  const nodes = new Set();
  const add = (from, to, as) => {
    if (!from || !to || from === to) return;
    links.push([from, to, as]);
    nodes.add(as === "caller" ? from : to);
  };
  for (const u of f.uses) if (!u.test) add(u.node, own.id, "caller");
  for (const c of f.calls) {
    add(own.id, c.node, c.kind === "field" ? "field" : "call");
    if (c.kind === "field" && c.key) for (const n of named(c.key)) add(c.node, n, "target");
  }
  return { own: own.id, name: `${own.name}::${f.name}`, links, nodes };
}
const fieldsOf = (fn) => functionOf(fn).f.calls.filter((c) => c.kind === "field" && c.key).map((c) => ({ key: c.key, writes: c.writes }));

// Opens the function `fn` in the detail, or none: the fields it uses
// light on the left, and what it relates to is laid out on the map.
function focusFn(fn) {
  if (fn && !functionOf(fn)) fn = null;
  actFn = fn;
  const reached = fn ? reach(fn) : null;
  focus(reached && { own: reached.own, name: reached.name, links: reached.links }, ...(narrow() ? [] : [LEFT, RIGHT]));
  focusMark = reached ? reached.nodes : new Set();
  view.mark = new Set(focusMark);
  const hit = new Map((fn ? fieldsOf(fn) : []).map((x) => [x.key, x]));
  dt.classList.toggle("focus", Boolean(fn));
  fl.classList.toggle("focus", Boolean(fn));
  dt.querySelectorAll(".m.fn").forEach((r) => r.classList.toggle("act", r.dataset.fn === fn));
  document.querySelectorAll(".pane .m.fld").forEach((r) => r.classList.toggle("hit", hit.has(r.dataset.f)));
  document.querySelectorAll(".pane .m.fld").forEach((r) => {
    const h = hit.get(r.dataset.f);
    r.classList.toggle("w", Boolean(h && h.writes));
  });
  fl.classList.remove("rest");
  const rest = fl.querySelector("[data-rest]");
  if (rest) {
    const n = fl.querySelectorAll(".m.fld").length - fl.querySelectorAll(".m.fld.hit").length;
    rest.dataset.n = n;
    rest.textContent = `${n} field${n === 1 ? "" : "s"} it does not use`;
    rest.classList.toggle("nothing", !n);
  }
  fl.querySelectorAll(".u").forEach((u) => u.classList.toggle("me", Boolean(fn) && u.dataset.in === fn));
  render();
}

/// Goes to the function `fn`: shows its type, in the level of the box
/// that owns it, and opens it there.
export function jump(fn) {
  whenStill(() => jumpNow(fn));
}

function jumpNow(fn) {
  const tid = ownerOf(fn);
  if (!tid) return;
  if (view.sel !== tid) showTypeNow(tid);
  if (details.get(tid)?.tag !== tag) {
    // Once its type's detail is read, if the page is still at the type.
    want(tid).then((detail) => detail && whenStill(() => view.sel === tid && jumpNow(fn)));
    return;
  }
  dt.querySelectorAll(".m.fn.open").forEach((r) => r.classList.remove("open"));
  const row = [...dt.querySelectorAll(".m.fn")].find((x) => x.dataset.fn === fn);
  if (!row) return;
  row.classList.add("open");
  focusFn(fn);
  row.scrollIntoView({ block: "start", behavior: "smooth" });
  row.classList.add("flash");
  setTimeout(() => row.classList.remove("flash"), 900);
}

/// Shows the type `id`, in the level of the box that owns it.
export function showType(id) {
  whenStill(() => showTypeNow(id));
}

function showTypeNow(id) {
  const t = types[id];
  if (!t) return;
  if (view.stack.at(-1) !== t.box || !placed(id)) goTo(t.box, id);
  else {
    view.sel = id;
    render();
  }
  if (narrow()) fitAbove(id);
  else fitAbove(id, LEFT, RIGHT);
}

function onClick(e) {
  const rest = e.target.closest("[data-rest]");
  if (rest) {
    fl.classList.toggle("rest");
    const n = Number(rest.dataset.n);
    rest.textContent = fl.classList.contains("rest")
      ? `Fold the ${n} field${n === 1 ? "" : "s"} it does not use`
      : `${n} field${n === 1 ? "" : "s"} it does not use`;
    return;
  }
  const j = e.target.closest("[data-jump]");
  if (j) {
    jump(j.dataset.jump);
    return;
  }
  const go = e.target.closest("[data-go]");
  if (go) {
    const row = [...document.querySelectorAll(".pane .m.fld")].find((x) => x.dataset.f === go.dataset.go);
    if (row) row.scrollIntoView({ block: "center", behavior: "smooth" });
    else if (ownerOf(go.dataset.go)) showType(ownerOf(go.dataset.go));
    return;
  }
  const m = e.target.closest(".m[data-m]");
  if (!m || e.target.closest(".us")) return;
  m.classList.toggle("open");
  if (m.dataset.fn) focusFn(m.classList.contains("open") ? m.dataset.fn : actFn === m.dataset.fn ? null : actFn);
}
// A row pointed at lights the nodes that use it on the map.
function onOver(e) {
  const m = e.target.closest(".m");
  const marked = new Set(focusMark);
  if (m) m.querySelectorAll("[data-node]").forEach((u) => u.dataset.node && marked.add(u.dataset.node));
  if ([...marked].join() !== [...view.mark].join()) {
    view.mark = marked;
    draw();
  }
}
function onLeave() {
  view.mark = new Set(focusMark);
  draw();
}
for (const el of [dt, fl]) {
  el.addEventListener("click", onClick);
  el.addEventListener("pointerover", onOver);
  el.addEventListener("pointerleave", onLeave);
}
afterRender(panel);

// The drawer's grip, or its head, pulled: up for all of it, down to
// half, and from half down to close it.
let pull = null;
dt.addEventListener("pointerdown", (e) => {
  if (!narrow() || !e.target.closest(".grip,.hd")) return;
  pull = { y: e.clientY, base: dt.getBoundingClientRect().top };
  dt.classList.add("drag");
  dt.setPointerCapture(e.pointerId);
});
dt.addEventListener("pointermove", (e) => {
  if (!pull) return;
  const at = Math.max(innerHeight * 0.12, pull.base + e.clientY - pull.y);
  dt.style.transform = `translateY(${at - innerHeight * 0.12}px)`;
});
dt.addEventListener("pointerup", (e) => {
  if (!pull) return;
  const d = e.clientY - pull.y;
  const full = dt.classList.contains("full");
  pull = null;
  dt.classList.remove("drag");
  dt.style.transform = "";
  if (Math.abs(d) < 6) {
    dt.classList.toggle("full");
    return;
  }
  if (d < 0) dt.classList.add("full");
  else if (full) dt.classList.remove("full");
  else {
    view.sel = null;
    render();
  }
});

