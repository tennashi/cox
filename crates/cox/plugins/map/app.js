// The map of the project, drawn by a page of the stage: the picture of the
// project in tiers that an agent wrote in its `.cox/map.json`, read as
// JSON from `cox://api/map` with what Cox knows laid over it, and read
// again every two seconds, as Cox's own tabs follow what they show. A
// tier is rows of cards, with the cards of the tiers above that stay
// around it as a frame, the user's side above and the code's side below,
// and lines between them for the edges; a card
// opens the tier inside it, or, on its own, the paths of the code it
// holds, each of which opens on a tab of the stage through a
// `cox://open/` address, with what the project's language server says
// of each file, read from `cox://api/file/<path>`: the symbols it
// defines, how many places use each, and which files use them. Where
// the page is open is its hash: a tier's id, and after it the id of a
// card open on its own, as `#cox/stage`, and after `@` the flow
// chosen, drawn as a sequence diagram of the tier's boxes, or, with
// `+map`, numbered on the lines of the tier, or, for a card open on
// its own, of the functions of its code that the flow runs. Drawn
// between the boxes it is written between, a step is named by the
// function it runs, which the page reads from
// `cox://api/call/<path>:<line>` for the place the step is made.

const API = "cox://api/map";
const EVERY = 2000;
const NS = "http://www.w3.org/2000/svg";
const DEFS =
  '<defs><marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="currentColor"/></marker>' +
  '<marker id="arrlit" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="var(--hold)"/></marker>' +
  '<marker id="back" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M1 1 L9 5 L1 9" fill="none" stroke="currentColor" stroke-width="1.4"/></marker>' +
  '<marker id="backlit" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M1 1 L9 5 L1 9" fill="none" stroke="var(--hold)" stroke-width="1.6"/></marker></defs>';

const app = document.getElementById("app");
let data = null;
let lastText = null;
// What the server says of the files the page shows, by their path, as
// text, to see whether it changed, and read.
const fileText = {};
const fileData = {};
let wanted = [];
// What the server says the steps of the flow chosen call, by the place
// each is made, `path:line`, as the page has heard it; the places the
// page shows now, which `loadFiles` reads again.
const callText = {};
const callData = {};
let wantedAt = [];
// The functions the flow chosen runs in the box open on its own, by
// `path:line`, which the files' outlines mark.
let onFlow = new Set();

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

// --- What the map says, and where things are in it.

const parts = () => (data && data.map && data.map.parts) || {};
const tiers = () => (data && data.map && data.map.tiers) || [];
const edges = () => (data && data.map && data.map.edges) || [];
const flows = () => (data && data.map && data.map.flows) || [];
const nameOf = (id) => (parts()[id] && parts()[id].name) || id;
const facts = (id) => (data && data.facts && data.facts[id]) || { holds: [], changed: { files: 0, added: 0, removed: 0 }, stale: [] };
const tier = (id) => tiers().find((t) => t.id === id);
const partsOf = (t) => t.rows.flatMap((row) => row.parts);

/// The tier a card is in: the first that lists it.
function tierOf(id) {
  return tiers().find((t) => partsOf(t).includes(id));
}

/// The card that opens a tier, if one does.
function openerOf(tierId) {
  return Object.keys(parts()).find((id) => parts()[id].tier === tierId);
}

/// The tiers above `tierId`, top first, through the cards that open each.
function above(tierId) {
  const chain = [];
  let current = tierId;
  for (let i = 0; i < 12; i++) {
    const opener = openerOf(current);
    if (!opener) break;
    const up = tierOf(opener);
    if (!up || chain.includes(up.id) || up.id === current) break;
    chain.unshift(up.id);
    current = up.id;
  }
  return chain;
}

// --- Where the page is open.

// The hash is `tier`, or `tier/part` for a card open on its own, and
// after `@` the flow chosen, with `:` and the number of its step chosen,
// and `+map` when the flow is drawn on the map instead of in order.
function where() {
  const hash = decodeURIComponent(location.hash.replace(/^#/, ""));
  const [place, chosen = ""] = hash.split("@");
  const [tierId, partId] = place.split("/");
  const onMap = chosen.endsWith("+map");
  const [flowId, stepNo] = (onMap ? chosen.slice(0, -4) : chosen).split(":");
  const first = tiers()[0];
  const flow = flowId && flows().some((f) => f.id === flowId) ? flowId : null;
  const step = flow && /^\d+$/.test(stepNo || "") ? Number(stepNo) : null;
  return { tier: (tierId && tier(tierId) ? tierId : first && first.id) || null, part: partId || null, flow, step, onMap: Boolean(flow) && onMap };
}
function go(tierId, partId, { flow, step, onMap } = {}) {
  let hash = partId ? `${tierId}/${partId}` : tierId;
  if (flow) hash += `@${flow}${step ? `:${step}` : ""}${onMap ? "+map" : ""}`;
  location.hash = hash;
}

// Where the page has been, as its hashes, and where in them it is, so
// that ‹ and › go back and forward through the places it went to, as a
// code tab's do, after a drill down skipped tiers on the way.
const visited = [];
let visitedAt = -1;
function track() {
  const hash = location.hash;
  if (hash === visited[visitedAt]) return;
  if (hash === visited[visitedAt - 1]) visitedAt--;
  else if (hash === visited[visitedAt + 1]) visitedAt++;
  else {
    visited.splice(visitedAt + 1, visited.length, hash);
    visitedAt = visited.length - 1;
  }
}
function back() {
  if (visitedAt > 0) history.back();
}
function forward() {
  if (visitedAt < visited.length - 1) history.forward();
}

/// A place the page went to, as its hash names it, in words: the tier,
/// or the box open on its own, and the step chosen there.
function placeName(hash) {
  const [place, chosen = ""] = (hash || "").replace(/^#/, "").split("@");
  const [tierId, partId] = place.split("/");
  const t = tier(tierId) || tiers()[0];
  const name = partId && parts()[partId] ? nameOf(partId) : t ? t.name : "the map";
  const step = chosen.replace(/\+map$/, "").split(":")[1];
  return step ? `${name} · step ${step}` : name;
}

/// The ways out of the page open, over what it shows, each naming
/// where it goes: back to where the page was, up a tier, and forward
/// again; nothing when there is nowhere to go.
function waysEl() {
  const ways = [];
  if (visitedAt > 0) ways.push(`<button class="wayto" data-back title="Backspace, Alt-←, ⌘[">‹ back to <b>${esc(placeName(visited[visitedAt - 1]))}</b></button>`);
  const above_ = upFrom();
  if (above_) ways.push(`<button class="wayto" data-up title="↑">↑ up to <b>${esc(placeName(above_.part ? `${above_.tier}/${above_.part}` : above_.tier))}</b></button>`);
  if (visitedAt < visited.length - 1) ways.push(`<button class="wayto" data-forward title="Alt-→, ⌘]">forward to <b>${esc(placeName(visited[visitedAt + 1]))}</b> ›</button>`);
  return ways.length ? `<div class="waybar">${ways.join("")}</div>` : "";
}

/// Where a step up from the page open is: the tier of a box open on its
/// own, or the tier that holds the box the tier open is the inside of;
/// null at the top.
function upFrom() {
  const { tier: here, part } = where();
  if (part) return { tier: here, part: null };
  const opener = openerOf(here);
  const up = opener && tierOf(opener);
  return up ? { tier: up.id, part: null } : null;
}
function up() {
  const to = upFrom();
  const { flow, step, onMap } = where();
  if (to) go(to.tier, to.part, { flow, step, onMap });
}

// --- Flows: the orders in which the boxes talk, rolled up to a tier.

/// The boxes a tier shows: those of its rows and of its frame.
function shownIn(t) {
  const f = t.frame || {};
  return [...partsOf(t), ...(f.above || f.left || []), ...(f.below || f.right || [])];
}

/// The box of tier `t` that `id` is, or is inside of, through the boxes
/// that open the tiers between: one the tier shows, or the box the tier
/// is the inside of; with `far`, else the box of a tier above `t` that
/// holds it, which every box of the map is or is inside of; null when
/// there is none.
function upTo(t, id, far = false) {
  const shown = shownIn(t), own = openerOf(t.id), over = far ? above(t.id) : [];
  let current = id;
  for (let i = 0; i < 12 && current; i++) {
    if (shown.includes(current) || current === own) return current;
    const within = tierOf(current);
    if (within && over.includes(within.id)) return current;
    current = within ? openerOf(within.id) : null;
  }
  return null;
}

/// A flow's steps as tier `t` shows them: each between two of its boxes
/// ("line"), inside one of them ("inside"), between one of them and the
/// box the tier is the inside of ("own"), or elsewhere ("outside"). With
/// `far`, a box the tier does not show counts as the box of a tier above
/// that holds it, so that a step is elsewhere only when a box of it is
/// in no tier.
function stepsIn(t, flow, far = false) {
  const own = openerOf(t.id);
  return flow.steps.map((step, k) => {
    const a = upTo(t, step.from, far), b = upTo(t, step.to, far);
    const kind = !a || !b ? "outside" : a === b ? "inside" : a === own || b === own ? "own" : "line";
    return { n: k + 1, step, a, b, kind };
  });
}

/// Where a step inside box `id` is drawn apart, as deep as the map
/// goes: the first tier under the box whose boxes the step goes
/// between, through the boxes it stays inside of on the way, or, for a
/// box that holds paths, the box on its own, where the step goes
/// between the functions it runs; null for a box that is neither, as a
/// function is, and when it is the page open already.
function drillOf(step, id) {
  const { tier: hereTier, part: herePart } = where();
  let current = id;
  for (let i = 0; i < 12; i++) {
    const p = parts()[current];
    if (!p) return null;
    let found;
    if (p.tier) {
      const t = tier(p.tier);
      const a = upTo(t, step.from, true), b = upTo(t, step.to, true);
      if (a && b && a === b && a !== current) {
        current = a;
        continue;
      }
      found = { tier: t.id, part: null };
    } else if (p.holds && p.holds.length) {
      const t = tierOf(current);
      found = { tier: t ? t.id : hereTier, part: current };
    } else return null;
    return found.tier === hereTier && found.part === (herePart || null) ? null : found;
  }
  return null;
}
const drillHash = (d) => `${d.tier}${d.part ? `/${d.part}` : ""}`;
const drillName = (d) => (d.part ? nameOf(d.part) : (tier(d.tier) || { name: d.tier }).name);

/// Where a step is made, as the page asks the server of it: the file, as
/// an absolute path, and the line; null when `at` names no line.
function atPlace(at) {
  const m = /^(.*):(\d+)$/.exec(at || "");
  return m ? { path: absolute(m[1]), line: Number(m[2]) } : null;
}

/// What the server says of the place a step is made, as heard so far:
/// null until it is asked.
function callOf(step) {
  const place = atPlace(step.at);
  return place ? callData[`${place.path}:${place.line}`] || null : null;
}

/// The function a step runs, to name the step by where the map goes no
/// deeper: of the functions the server says its line calls, in the
/// order they are called on it, the one in the box the step goes to,
/// or else the last in the project, as a chain of calls ends in what it
/// does, or else the first; null when none is known.
function runs(st) {
  const c = callOf(st.step);
  if (!c || !c.calls || !c.calls.length) return null;
  const held = facts(st.b).holds.map((h) => h.path);
  const within = (call) => held.some((p) => absolute(call.path) === p || absolute(call.path).startsWith(`${p}/`));
  const own = c.calls.filter((call) => !call.path.startsWith("/"));
  return c.calls.find(within) || own[own.length - 1] || c.calls[0];
}

/// A function, as a `cox://open` address at its line.
function fnHref(fn) {
  return `cox://open${absolute(fn.path)}#L${fn.line}`;
}

/// The flows the map has, as chips, the flow chosen among them lit,
/// with `views` after them.
function flowBarEl(flow, views = "") {
  if (!flows().length) return "";
  const chips = flows()
    .map((f) => `<button class="flowchip ${flow && f.id === flow.id ? "on" : ""}" data-flow="${esc(f.id)}" title="${esc(f.says)}">${esc(f.name)}</button>`)
    .join("");
  return `<div class="flows"><span class="flowsname">flows</span>${chips}${views}</div>`;
}

/// What a flow is, while none of its steps is chosen.
function flowSaysEl(flow) {
  return `<div class="stepsays faint">${esc(flow.says)} <span class="where">← → go from step to step</span></div>`;
}

/// What the step chosen, `cur`, says: its number, the boxes it goes
/// between, its words, `where_`, where it is made, and what the line
/// calls, as the server says, once it has.
function stepSaysEl(cur, where_) {
  const at = cur.step.at ? `<a class="at" href="${esc(atHref(cur.step.at))}">${esc(cur.step.at)}</a>` : "";
  const back = cur.step.back ? '<span class="where">a step back</span>' : "";
  const c = cur.step.at ? callOf(cur.step) : null;
  let calls = "";
  if (c && c.calls && c.calls.length) {
    const named = c.calls.slice(0, 3).map((fn) => `<a class="at fn" href="${esc(fnHref(fn))}" title="${esc(fn.detail || fn.name)}">${esc(fn.name)}</a>`).join("");
    calls = `<span class="where">calls</span>${named}${c.function ? `<span class="where">in ${esc(c.function.name)}</span>` : ""}`;
  } else if (c && c.pending) calls = `<span class="where faint">${esc(c.said || "asking the language server what the line calls…")}</span>`;
  else if (c && c.error) calls = `<span class="where stale">${esc(c.error)}</span>`;
  else if (c && c.server) calls = `<span class="where stale">${c.function ? "no call found on that line" : "no function at that line"}: an agent is to bring the map up to date</span>`;
  return `<div class="stepsays"><span class="n">${cur.n}</span><span class="who">${esc(nameOf(cur.step.from))} → ${esc(nameOf(cur.step.to))}</span><span class="what">${esc(cur.step.says)}</span>${back}${where_}${at}${calls}</div>`;
}

/// Where a step is made, as a `cox://open` address.
function atHref(at) {
  const m = /^(.*):(\d+)$/.exec(at);
  const path = m ? m[1] : at;
  return `cox://open${path.startsWith("/") ? "" : data.root + "/"}${path}${m ? `#L${m[2]}` : ""}`;
}

/// While a flow is chosen: the steps on each line, by `from>to`, each
/// with its number and whether it is the step chosen.
let flowMarks = {};
let flowOn = false;
/// The pairs `from>to` some line of the tier has an arrow into `to` for.
let flowPointed = new Set();

// --- Drawing.

function crumbs(items) {
  const html = items
    .map((item, i) =>
      i === items.length - 1
        ? `<span class="here">${esc(item.text)}</span>`
        : `<button data-tier="${esc(item.tier)}" ${item.part ? `data-part="${esc(item.part)}"` : ""}>${esc(item.text)}</button><span>›</span>`,
    )
    .join("");
  return `<nav class="crumbs"><span class="depth">${items.length}</span>${html}</nav>`;
}

function sourceLine() {
  const n = Object.keys(parts()).length;
  const since = data.since ? `<span class="since">${esc(data.since)} · ${data.changed_files} file${data.changed_files === 1 ? "" : "s"} changed</span>` : "";
  return `<div class="source"><span>map</span><span class="file">${esc(data.file.replace(data.root + "/", ""))}</span><span>${tiers().length} tier${tiers().length === 1 ? "" : "s"} · ${n} box${n === 1 ? "" : "es"}</span>${since}</div>`;
}

/// A card for the box `id`. `key` places it in its row for the wires; a
/// card of the frame has `outer` instead.
function partEl(id, { key, outer } = {}) {
  const p = parts()[id];
  if (!p) return `<div class="part" ${key ? `data-k="${key}"` : ""}><span class="pname">${esc(id)}</span><span class="psays">not in the map's parts</span></div>`;
  const f = facts(id);
  const lit = f.changed.files > 0;
  const goes = p.tier ? "tier" : p.holds && p.holds.length ? "leaf" : outer ? "up" : null;
  const classes = ["part", goes ? "goes" : "", lit ? "lit" : "", f.stale.length ? "stale" : "", p.many ? "many" : "", outer ? "outer" : ""].join(" ");
  const chips =
    !outer && p.holds && p.holds.length
      ? `<span class="holds">${f.holds
          .map((h) => `<span class="chip ${!h.exists ? "missing" : h.added || h.removed ? "hot" : ""}">${esc(h.said.split("/").pop())}</span>`)
          .join("")}</span>`
      : "";
  const badge = lit ? `<span class="badge">+${f.changed.added} −${f.changed.removed} · ${f.changed.files} file${f.changed.files === 1 ? "" : "s"}</span>` : "";
  const gone = f.stale.length ? `<span class="gone"><b>stale</b><span>${f.stale.map(esc).join(", ")} gone</span></span>` : "";
  const note = p.note && !outer ? `<span class="note"><b>note</b><span>${esc(p.note)}</span></span>` : "";
  const kinds = p.kinds && p.kinds.length && !outer ? `<span class="kinds">${p.kinds.map(esc).join(" · ")}</span>` : "";
  const each = p.each ? `<span class="each">${esc(p.each)}</span>` : "";
  const says = p.says && !outer ? `<span class="psays">${esc(p.says)}</span>` : outer && p.says ? `<span class="psays">${esc(p.says)}</span>` : "";
  const go = goes === "tier" ? '<span class="go">open ›</span>' : goes === "leaf" ? '<span class="go">code ›</span>' : "";
  const tag = goes ? "button" : "div";
  return `<${tag} class="${classes}" data-id="${esc(id)}" ${goes ? `data-goes="${goes}"` : ""} ${key ? `data-k="${key}"` : ""} ${outer ? `data-outer="${esc(id)}"` : ""}>
    <span class="pname">${esc(p.name)}</span>${each}${says}${kinds}${badge}${gone}${note}${chips}${go}</${tag}>`;
}

/// The edge between two boxes, either way, if the map has one.
function edgeBetween(a, b) {
  return edges().find((e) => (e.from === a && e.to === b) || (e.from === b && e.to === a));
}

/// How an edge points, from `a` to `b`: "both", "to" (into b), "from"
/// (into a), or "none".
function pointing(e, a, b) {
  const dir = e.dir || "to";
  if (dir === "both" || dir === "none") return dir;
  return e.from === a && e.to === b ? "to" : "from";
}

function renderTier(tierId) {
  const t = tier(tierId);
  const frame = t.frame || {};
  const aboveIds = frame.above || frame.left || [];
  const belowIds = frame.below || frame.right || [];
  // The lines of cards the tier draws, from the top: the frame's cards of
  // the user's side, the tier's own rows, and the frame's cards of the
  // code's side, so that a tier reads from the top down as the top tier
  // does. Each card is keyed by its place, for the wires.
  const lines = [];
  if (aboveIds.length) lines.push({ parts: aboveIds, side: "above" });
  lines.push(...t.rows);
  if (belowIds.length) lines.push({ parts: belowIds, side: "below" });
  const keyOf = {};
  lines.forEach((row, ri) => row.parts.forEach((id, i) => (keyOf[id] = `r${ri}c${i}`)));
  const placeOf = (id) => (keyOf[id] || "r-1c-1").match(/^r(-?\d+)c(-?\d+)$/).slice(1).map(Number);
  const wires = [];
  const lineEl = (row, ri) =>
    row.parts
      .map((id, i) => {
        let gap = "";
        if (i) {
          const e = edgeBetween(row.parts[i - 1], id);
          if (e) {
            const dir = pointing(e, row.parts[i - 1], id);
            gap = `<span class="gap" data-label="${esc(e.says)}" data-dir="${dir}" data-from="${esc(row.parts[i - 1])}" data-to="${esc(id)}" style="flex-basis:${Math.max(44, e.says.length * 6.5 + 16)}px"></span>`;
          } else gap = '<span class="space"></span>';
        }
        return gap + partEl(id, { key: `r${ri}c${i}`, outer: Boolean(row.side) });
      })
      .join("");
  const rows = lines.map((row, ri) => {
    if (row.side) return `<div class="side ${row.side}"><div class="line">${lineEl(row, ri)}</div></div>`;
    if (row.under && !(row.parts[0] && edgeBetween(row.under, row.parts[0]))) wires.push({ from: row.under, toKey: `r${ri}c0`, tree: true });
    return `<div class="row" ${row.under ? `data-parent="${row.under}"` : ""}>${row.name ? `<span class="rowname">${esc(row.name)}</span>` : ""}<div class="line">${lineEl(row, ri)}</div></div>`;
  });
  // The edges the tier draws past its gaps: between its rows, and between
  // them and the frame. Those among the frame's cards are the tiers
  // above's to draw.
  for (const e of edges()) {
    const [ra, ia] = placeOf(e.from), [rb, ib] = placeOf(e.to);
    if (ra < 0 || rb < 0) continue;
    if (ra === rb && Math.abs(ia - ib) === 1 && edgeBetween(e.from, e.to) === e) continue; // drawn in the gap
    if (lines[ra].side && lines[rb].side) continue;
    wires.push({ from: e.from, to: e.to, label: e.says, dir: e.dir || "to" });
  }
  // The flow chosen: its steps rolled up to this tier's boxes, numbered
  // on the lines between them, a line of the flow's own where the map
  // has no edge.
  const { flow: flowId, step: stepNo, onMap } = where();
  const flow = flowId ? flows().find((f) => f.id === flowId) : null;
  wantedAt = flow ? flow.steps.map((s) => s.at).filter(Boolean) : [];
  const seq = Boolean(flow) && !onMap;
  const shown = flow ? stepsIn(t, flow, seq) : [];
  flowMarks = {};
  flowOn = Boolean(flow);
  flowPointed = new Set();
  for (const e of edges()) {
    const dir = e.dir || "to";
    if (dir === "to" || dir === "both") flowPointed.add(`${e.from}>${e.to}`);
    if (dir === "both") flowPointed.add(`${e.to}>${e.from}`);
  }
  for (const st of shown) {
    if (st.kind !== "line") continue;
    (flowMarks[`${st.a}>${st.b}`] = flowMarks[`${st.a}>${st.b}`] || []).push({ n: st.n, lit: st.n === stepNo });
    const drawn = edgeBetween(st.a, st.b) || wires.some((w) => (w.from === st.a && w.to === st.b) || (w.from === st.b && w.to === st.a));
    if (!drawn) wires.push({ from: st.a, to: st.b, label: "", dir: "to", flow: true });
  }
  const views = flow
    ? `<span class="views"><button class="view ${seq ? "on" : ""}" data-view="seq" title="the steps in order, between the boxes as columns">sequence</button><button class="view ${onMap ? "on" : ""}" data-view="map" title="the steps numbered on the lines of the map">on the map</button></span>`
    : "";
  const flowBar = flowBarEl(flow, views);
  let stepsBar = "";
  if (flow) {
    const chips = seq
      ? ""
      : `<div class="steps">${shown
          .map((st) => `<button class="step ${st.kind} ${st.n === stepNo ? "on" : ""}" data-step="${st.n}" title="${esc(nameOf(st.step.from))} → ${esc(nameOf(st.step.to))}: ${esc(st.step.says)}">${st.n}</button>`)
          .join("")}</div>`;
    const cur = stepNo ? shown[stepNo - 1] : null;
    let says;
    if (cur) {
      const where_ =
        cur.kind === "inside"
          ? drillOf(cur.step, cur.a)
            ? `<span class="where">inside ${esc(nameOf(cur.a))}</span><button class="into" data-drill="${esc(drillHash(drillOf(cur.step, cur.a)))}">in ${esc(drillName(drillOf(cur.step, cur.a)))} ›</button>`
            : `<span class="where">inside ${esc(nameOf(cur.a))}</span>`
          : cur.kind === "own"
            ? `<span class="where">${cur.a === openerOf(tierId) ? "from" : "into"} ${esc(nameOf(openerOf(tierId)))}, the box this tier is the inside of</span>`
            : cur.kind === "outside"
              ? `<span class="where">${seq ? "in no tier of the map" : "elsewhere in the map"}</span>`
              : "";
      says = stepSaysEl(cur, where_);
    } else says = flowSaysEl(flow);
    stepsBar = `${chips}${says}`;
  }
  const opener = openerOf(tierId);
  const head = opener ? parts()[opener] : null;
  const noframe = !aboveIds.length && !belowIds.length;
  const legend = head ? `<div class="legend"><span class="name" data-goes="up" data-id="${esc(opener)}" title="up to ${esc((tierOf(opener) || { name: "the tier above" }).name)} (↑)">${esc(head.name)}<span class="upmark"> ↑</span></span>${head.each ? `<span class="each">${esc(head.each)}</span>` : ""}</div>` : "";
  const focus = rows.filter((_, ri) => !lines[ri].side).join("");
  const body = seq
    ? seqEl(tierView(t), shown, stepNo)
    : `<div class="drill ${noframe ? "noframe" : ""}" data-wires="${esc(JSON.stringify(wires))}" data-keys="${esc(JSON.stringify(keyOf))}">
    ${aboveIds.length ? rows[0] : ""}
    <div class="focus">${legend}${focus}</div>
    ${belowIds.length ? rows[rows.length - 1] : ""}
  </div>`;
  const foot = seq
    ? `<div class="foot"><span>a column opens the tier inside it, or the code it holds</span><span>·</span><span>a row is a step of the flow; a loop is a step inside a box, and opens where the step is drawn apart</span><span>·</span><span>← → go from step to step, ↑ up a tier, Backspace back</span></div>`
    : `<div class="foot"><span>a card opens the tier inside it, or the code it holds</span><span>·</span><span>orange is what changed, as Changes shows it</span><span>·</span><span>↑ goes up a tier, Backspace back</span></div>`;
  const chain = [...above(tierId), tierId].map((id) => ({ text: tier(id).name, tier: id }));
  app.innerHTML = `${crumbs(chain)}${sourceLine()}${t.says ? `<div class="says">${esc(t.says)}</div>` : ""}${flowBar}${stepsBar}${waysEl()}${body}${foot}`;
  if (seq) seqScrolled();
  wired();
}

/// A tier as the sequence diagram draws it: `levels`, the tiers from
/// the top down to it, each with `own`, the box it is the inside of,
/// and `members`, its boxes in the order it draws them; and `near`,
/// the boxes the tier shows, its own, the frame's and the box it is the
/// inside of, as against those of the tiers above, which the diagram
/// draws dim.
function tierView(t) {
  const own = openerOf(t.id) || null;
  const levels = [...above(t.id), t.id].map((tid) => ({ own: openerOf(tid) || null, members: partsOf(tier(tid)) }));
  return { levels, near: new Set([...shownIn(t), ...(own ? [own] : [])]) };
}

/// A flow's steps as box `id`, which holds code, shows them, as deep as
/// the map goes and one step deeper: the other boxes rolled up as tier
/// `t`, which holds the box, shows them with `far`, and the box itself
/// as the functions its steps run, as the server says of the places the
/// steps are made: a step into the box arrives at the function its line
/// calls in the box, a step out of it leaves the function its line is
/// in, and a step inside it goes from the one to the other, or loops on
/// the one when the server names no other. A step whose place the
/// server has not told stays on the box itself. Answers the steps, the
/// view for `seqEl`, and the functions as columns, by `path:line`.
function stepsAt(t, flow, id) {
  const held = facts(id).holds.map((h) => h.path);
  const within = (fn) => held.some((p) => absolute(fn.path) === p || absolute(fn.path).startsWith(`${p}/`));
  const extra = {}, inner = [], places = new Set();
  const column = (fn) => {
    const key = `fn:${absolute(fn.path)}:${fn.line}`;
    if (!extra[key]) {
      extra[key] = { name: fn.name, says: fn.detail || "", each: `${fn.path.split("/").pop()}:${fn.line}`, href: fnHref(fn), fn };
      inner.push(key);
      places.add(`${absolute(fn.path)}:${fn.line}`);
    }
    return key;
  };
  const shown = flow.steps.map((step, k) => {
    let a = upTo(t, step.from, true), b = upTo(t, step.to, true);
    const c = callOf(step);
    if (c) {
      if (a === id && c.function && within(c.function)) a = column(c.function);
      if (b === id) {
        const entry = (c.calls || []).find(within);
        if (entry) b = column(entry);
        else if (a !== id && a && a.startsWith("fn:")) b = a;
      }
    }
    const kind = !a || !b ? "outside" : a === b ? "inside" : "line";
    return { n: k + 1, step, a, b, kind };
  });
  const view = { levels: [...tierView(t).levels, { own: id, members: inner }], near: new Set([...shownIn(t), id, ...inner]), extra };
  return { shown, view, places };
}

/// A flow as a sequence diagram of `view`, a tier's or a box's that
/// holds code: the boxes its steps go between, as `stepsIn` or
/// `stepsAt` rolled them up, as columns, each with a lifeline down the
/// page, in the order of the map: the top tier's boxes as it draws
/// them, each that opens a tier of the way down replaced by that
/// tier's boxes in its order, in a frame named with the box, the box
/// itself first in it, down to the view's own, so that the frames nest
/// down to the tier open, and a drill down opens a box in place, the
/// other columns staying where they were; a box of no tier of the way,
/// as a frame's can be, goes before them all when the flow reaches it
/// before it comes into the view's own box, and after them otherwise.
/// And the steps as rows
/// from the top, an arrow from the box that talks to the one it talks
/// to, with what the step says over it; a step back, the answer to an
/// earlier step, dashed. A bar on a lifeline shows the box at work,
/// from a step into it until its step back, or the last step. A step
/// inside a box is a loop on its lifeline, which opens the box when it
/// opens a tier; a step of a box in no tier is a dim row. The step
/// chosen, `stepNo`, is lit.
function seqEl(view, shown, stepNo) {
  const { levels, near, extra = {} } = view;
  const owns = new Set(levels.map((lv) => lv.own).filter(Boolean));
  const partOf = (id) => parts()[id] || extra[id] || { name: id, says: "" };
  const used = new Set(shown.filter((st) => st.kind !== "outside").flatMap((st) => [st.a, st.b]));
  // Each level's columns, the next level's in the place of the box that
  // opens it.
  const span = levels.map(() => []);
  const expand = (k) => {
    const lv = levels[k], next = levels[k + 1];
    const out = lv.own ? [lv.own] : [];
    let opened = false;
    for (const id of lv.members) {
      if (next && next.own === id) {
        out.push(...expand(k + 1));
        opened = true;
      } else out.push(id);
    }
    if (next && !opened) out.push(...expand(k + 1));
    span[k] = out;
    return out;
  };
  const inOrder = expand(0);
  const placed = new Set(inOrder);
  const inner = new Set(span[levels.length - 1]);
  const arrived = shown.findIndex((st) => st.kind !== "outside" && (inner.has(st.a) || inner.has(st.b)));
  const left = [], right = [];
  shown.forEach((st, i) => {
    if (st.kind === "outside") return;
    for (const id of [st.a, st.b]) {
      if (placed.has(id) || left.includes(id) || right.includes(id)) continue;
      (arrived < 0 || i <= arrived ? left : right).push(id);
    }
  });
  const order = [...left, ...inOrder, ...right];
  const cols = order.filter((id, i) => used.has(id) && order.indexOf(id) === i);
  if (!cols.length) return '<div class="says faint">None of the steps of this flow go between boxes the map has.</div>';
  const col = Object.fromEntries(cols.map((id, i) => [id, i]));
  const textW = (s, size = 11) => String(s).length * size * 0.56;
  const trunc = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
  const HEAD_H = 40, ROW = 30, THIN = 22, LOOP = 22, PAD = 14;
  // Each level's columns, the box it is the inside of among them, are
  // framed as the map boxes the tier, the frames nested from the top
  // tier's box down to the view's own, each a row of its own over the
  // heads, apart from the columns of no level.
  const frames = levels
    .map((lv, k) => {
      const at = span[k].filter((id) => col[id] !== undefined).map((id) => col[id]);
      return lv.own && at.length ? { own: lv.own, first: Math.min(...at), last: Math.max(...at) } : null;
    })
    .filter(Boolean);
  const nF = frames.length;
  const TOP = 6 + 16 * nF;
  // The columns share the width the page has, up to 150 apart, their
  // heads narrower where the page is narrow, and are a step's label
  // apart at least, shared over the columns a long step crosses.
  const avail = (app.clientWidth || 800) - 32;
  const pitch = (avail - 56 - 132 - 2 * PAD * nF) / Math.max(1, cols.length - 1);
  const HEAD_W = Math.round(Math.max(96, Math.min(132, pitch - 10)));
  const LEFT = 40 + HEAD_W / 2 + PAD * Math.max(0, frames.filter((f) => f.first === 0).length - 1);
  const base = Math.max(HEAD_W + 10, Math.min(150, Math.floor(pitch)));
  const gaps = cols.slice(1).map(() => base);
  // Drawn between the boxes it is written between, as deep as the map
  // goes for it, a step is named by the function it runs, once the
  // server says which; rolled up, it says what it is in words.
  const fnOf = {};
  for (const st of shown) if (st.kind !== "outside" && st.a === st.step.from && st.b === st.step.to) fnOf[st.n] = runs(st);
  const labelOf = (st) => (fnOf[st.n] ? fnOf[st.n].name : st.step.says);
  const labelEl = (st, x, y, anchor) => {
    const fn = fnOf[st.n];
    const at = `x="${x}" y="${y}"${anchor ? ` text-anchor="${anchor}"` : ""}`;
    return fn
      ? `<a class="fnlink" href="${esc(fnHref(fn))}"><title>${esc(fn.detail || fn.name)}</title><text class="what fn" ${at}>${esc(fn.name)}</text></a>`
      : `<text class="what" ${at}>${esc(st.step.says)}</text>`;
  };
  for (const st of shown) {
    if (st.kind === "outside" || st.a === st.b) continue;
    const i = Math.min(col[st.a], col[st.b]), j = Math.max(col[st.a], col[st.b]);
    const need = (textW(labelOf(st)) + 28) / (j - i);
    for (let k = i; k < j; k++) gaps[k] = Math.max(gaps[k], need);
  }
  for (const f of frames) {
    if (f.first > 0) gaps[f.first - 1] += 2 * PAD;
    if (f.last < cols.length - 1) gaps[f.last] += 2 * PAD;
  }
  const xs = [LEFT];
  for (const g of gaps) xs.push(xs[xs.length - 1] + g);
  let W = xs[xs.length - 1] + HEAD_W / 2 + 16 + PAD * frames.filter((f) => f.last === cols.length - 1).length;
  let y = TOP + HEAD_H + 24;
  const said = (st) => `${esc(nameOf(st.step.from))} → ${esc(nameOf(st.step.to))}: ${esc(st.step.says)}`;
  const badgeEl = (label, yc) => {
    const w = Math.max(18, textW(label, 9.5) + 10);
    return `<g class="n"><rect x="${22 - w / 2}" y="${yc - 9}" width="${w}" height="18" rx="9"/><text x="22" y="${yc + 3.3}">${label}</text></g>`;
  };
  // Steps elsewhere in the map one after another share a row, which
  // says the one chosen among them, or how many they are.
  // The links out of the loops on a lifeline line up, past its longest
  // label.
  const loopW = {};
  for (const st of shown) if (st.kind !== "outside" && st.a === st.b) loopW[st.a] = Math.max(loopW[st.a] || 0, textW(labelOf(st)));
  const groups = [];
  for (const st of shown) {
    const last = groups[groups.length - 1];
    if (st.kind === "outside" && last && last.outside) last.steps.push(st);
    else groups.push({ outside: st.kind === "outside", steps: [st] });
  }
  for (const group of groups) {
    group.h = group.outside ? THIN : ROW;
    group.yc = y + group.h / 2;
    y += group.h;
  }
  const ycOf = {};
  for (const group of groups) for (const st of group.steps) ycOf[st.n] = group.yc;
  // The bars: a step into a box starts one on its lifeline, beside those
  // it has already, and the box's step back ends the last; the arrows
  // leave and arrive at the bars' edges.
  const BAR = 8;
  const open = {}, bars = [], ends = {};
  const edge = (id, k, side) => xs[col[id]] + (k ? (k - 1) * 4 + (side * BAR) / 2 : 0);
  for (const st of shown) {
    if (st.kind === "outside") continue;
    const oa = open[st.a] || (open[st.a] = []), ob = open[st.b] || (open[st.b] = []);
    if (st.a === st.b) {
      ends[st.n] = { xa: edge(st.a, oa.length, 1) };
      continue;
    }
    const side = xs[col[st.b]] > xs[col[st.a]] ? 1 : -1;
    if (st.step.back) {
      const bar = oa.pop();
      if (bar) {
        bar.y1 = ycOf[st.n];
        bars.push(bar);
      }
      ends[st.n] = { xa: edge(st.a, bar ? bar.k : 0, side), xb: edge(st.b, ob.length, -side) };
    } else {
      const xa = edge(st.a, oa.length, side);
      ob.push({ id: st.b, k: ob.length + 1, y0: ycOf[st.n] });
      ends[st.n] = { xa, xb: edge(st.b, ob.length, -side) };
    }
  }
  const lastY = groups.length ? groups[groups.length - 1].yc : y;
  for (const id in open) for (const bar of open[id]) bars.push({ ...bar, y1: lastY });
  const rows = groups.map((group) => {
    const st = group.steps.find((x) => x.n === stepNo) || group.steps[0];
    const { h, yc } = group;
    const on = group.steps.some((x) => x.n === stepNo);
    const span = group.steps.length > 1 ? `${group.steps[0].n}–${group.steps[group.steps.length - 1].n}` : String(st.n);
    const badge = badgeEl(on || group.steps.length === 1 ? String(st.n) : span, yc);
    let drawn;
    if (group.outside) {
      const text =
        on || group.steps.length === 1
          ? `${said(st)} · a box in no tier of the map`
          : `${group.steps.length} steps of boxes in no tier of the map, ${esc(nameOf(st.step.from))} → … → ${esc(nameOf(group.steps[group.steps.length - 1].step.to))}`;
      drawn = `<text class="what" x="${22 + Math.max(9, textW(span, 9.5) / 2 + 5) + 10}" y="${yc + 3.5}">${text}</text>`;
    } else if (st.a === st.b) {
      const x = ends[st.n].xa;
      const intoAt = x + LOOP + 8 + loopW[st.a] + 10;
      const deeper = drillOf(st.step, st.a);
      const into = deeper ? `<text class="into" x="${intoAt}" y="${yc + 4}">in ${esc(drillName(deeper))} ›</text>` : "";
      W = Math.max(W, intoAt + (deeper ? textW(`in ${drillName(deeper)} ›`) : 0) + 16);
      const loop = `<path class="arrow loop" d="M${x} ${yc - 7} H${x + LOOP} V${yc + 7} H${x + 2}" marker-end="url(#${on ? "arrlit" : "arr"})"/>${labelEl(st, x + LOOP + 8, yc + 4)}${into}`;
      drawn = deeper ? `<g class="drill" data-drill="${esc(drillHash(deeper))}"><title>opens ${esc(drillName(deeper))}, where the step is drawn apart</title>${loop}</g>` : loop;
    } else {
      const { xa, xb } = ends[st.n];
      const end = xb > xa ? xb - 1 : xb + 1;
      const back = st.step.back;
      const marker = back ? (on ? "backlit" : "back") : on ? "arrlit" : "arr";
      drawn = `<path class="arrow ${back ? "back" : ""}" d="M${xa} ${yc} L${end} ${yc}" marker-end="url(#${marker})"/>${labelEl(st, (xa + xb) / 2, yc - 5, "middle")}`;
    }
    const title = group.steps.every((x) => x.step.at && x.kind !== "outside")
      ? ""
      : `<title>${group.steps.map((x) => `${x.n}. ${said(x)}${x.step.at ? ` (${esc(x.step.at)})` : ""}`).join("\n")}</title>`;
    return `<g class="row ${st.kind} ${on ? "on" : ""}" data-step="${st.n}">${title}<rect class="band" x="4" y="${yc - h / 2 + 1}" width="__W__" height="${h - 2}" rx="6"/>${badge}${drawn}</g>`;
  });
  const H = y + 18, HS = H + 6 * Math.max(0, nF - 1);
  const acts = bars
    .map((bar) => `<rect class="act" x="${xs[col[bar.id]] + (bar.k - 1) * 4 - BAR / 2}" y="${bar.y0 - 4}" width="${BAR}" height="${bar.y1 - bar.y0 + 8}" rx="1"/>`)
    .join("");
  // What the page scrolls into view when it is wider than the page: the
  // step chosen, or the box.
  let focus = [0, 0];
  const lit = stepNo ? shown[stepNo - 1] : null;
  if (lit && lit.kind !== "outside") {
    const { xa, xb } = ends[lit.n];
    focus = lit.a === lit.b ? [xa, xa + LOOP + 8 + textW(labelOf(lit))] : [Math.min(xa, xb), Math.max(xa, xb)];
  }
  // The frames, the outermost first, each a little wider and taller
  // than those inside it, and dim but the innermost; the name of each
  // goes up to the tier that shows its box.
  const box = frames
    .map((f, j) => {
      const head = parts()[f.own] || { name: f.own };
      const within = frames.slice(j + 1);
      const x0 = xs[f.first] - HEAD_W / 2 - PAD * (1 + within.filter((g) => g.first === f.first).length);
      const x1 = xs[f.last] + HEAD_W / 2 + PAD * (1 + within.filter((g) => g.last === f.last).length);
      const top = 6 + 16 * (j + 1), bottom = H - 4 + 6 * (nF - 1 - j);
      if (j === nF - 1 && (!lit || lit.kind === "outside")) focus = [x0, x1];
      const each = head.each ? `<text class="each" x="${x0 + 14 + textW(`${head.name} ↑`, 13) + 10}" y="${top - 4}">${esc(head.each)}</text>` : "";
      return `<g class="box ${j < nF - 1 ? "outer" : ""}" data-x0="${Math.round(x0)}" data-x1="${Math.round(x1)}"><rect x="${x0}" y="${top - 8}" width="${x1 - x0}" height="${bottom - (top - 8)}" rx="12"/><text class="name" data-goes="up" data-id="${esc(f.own)}" x="${x0 + 14}" y="${top - 4}"><title>up to ${esc((tierOf(f.own) || { name: "the tier above" }).name)} (↑)</title>${esc(head.name)}<tspan class="upmark"> ↑</tspan></text>${each}</g>`;
    })
    .join("");
  const lives = cols.map((id) => `<line class="life" x1="${xs[col[id]]}" y1="${TOP + HEAD_H}" x2="${xs[col[id]]}" y2="${H - 8}"/>`).join("");
  const heads = cols
    .map((id) => {
      const p = partOf(id);
      const goes = p.tier ? "tier" : p.holds && p.holds.length ? "leaf" : p.href ? "fn" : "";
      const far = !near.has(id);
      const sub = owns.has(id) ? "the box itself" : far ? `in ${(tierOf(id) || { name: "?" }).name}` : p.each || "";
      const fit = Math.floor((HEAD_W - 16) / 7);
      const name = `<text class="name" x="${HEAD_W / 2}" y="${sub ? 17 : 24}" text-anchor="middle">${esc(trunc(p.name, fit))}</text>`;
      const each = sub ? `<text class="each" x="${HEAD_W / 2}" y="31" text-anchor="middle">${esc(trunc(sub, fit + 5))}</text>` : "";
      const g = `<g class="who ${goes ? "goes" : ""} ${owns.has(id) ? "own" : ""} ${far ? "far" : ""} ${p.href ? "fn" : ""}" data-seq="${esc(id)}" transform="translate(${xs[col[id]] - HEAD_W / 2} ${TOP})"><title>${esc(p.name)}${p.says ? ` — ${esc(p.says)}` : ""}</title><rect width="${HEAD_W}" height="${HEAD_H}" rx="7"/>${name}${each}</g>`;
      return p.href ? `<a class="fnlink" href="${esc(p.href)}">${g}</a>` : g;
    })
    .join("");
  const width = Math.round(W);
  const body = rows.map((r) => r.replace("__W__", String(width - 8))).join("");
  return `<div class="seq" data-focus="${focus.map(Math.round).join(",")}"><svg width="${width}" height="${HS}" viewBox="0 0 ${width} ${HS}">${DEFS}${box}${lives}${acts}${body}${heads}</svg></div>`;
}

// A sequence diagram wider than the page keeps where it was scrolled to
// while the page is drawn again at the same place, and scrolls what it
// focuses on into view when the place changes.
let seqAt = "", seqScroll = 0;
function seqScrolled() {
  const el = app.querySelector(".seq");
  if (!el) return;
  const [x0, x1] = (el.dataset.focus || "0,0").split(",").map(Number);
  const width = el.clientWidth;
  const at = String(el.scrollWidth);
  let want = at === seqAt ? seqScroll : 0;
  if (x1 > x0 && (x0 < want + 40 || x1 > want + width - 8)) want = x1 - x0 <= width - 48 ? (x0 + x1) / 2 - width / 2 : x0 - 48;
  el.scrollLeft = Math.max(0, want);
  seqAt = at;
  seqScroll = el.scrollLeft;
  stickFrames(el);
  el.addEventListener("scroll", () => {
    seqScroll = el.scrollLeft;
    stickFrames(el);
  });
}

/// Keeps the names of the frames in sight as the diagram scrolls: each
/// at the left edge of what is in view, once its frame's left edge has
/// gone out of it, until its frame's right edge comes to it; so the
/// names, one over another, say which boxes the columns in view are in.
function stickFrames(el) {
  const left = el.scrollLeft;
  for (const g of el.querySelectorAll(".box")) {
    const x0 = Number(g.dataset.x0), x1 = Number(g.dataset.x1);
    const name = g.querySelector(".name"), each = g.querySelector(".each");
    const w = name.getComputedTextLength ? name.getComputedTextLength() : 0;
    const ew = each && each.getComputedTextLength ? each.getComputedTextLength() + 10 : 0;
    const x = Math.max(x0 + 14, Math.min(left + 14, x1 - 14 - w - ew));
    name.setAttribute("x", String(Math.round(x)));
    if (each) each.setAttribute("x", String(Math.round(x + w + 10)));
  }
}

function renderLeaf(tierId, id) {
  const p = parts()[id] || { name: id, says: "" };
  const f = facts(id);
  wanted = f.holds.filter((h) => h.exists && h.lines !== null).map((h) => h.path);
  // The flow chosen, as a sequence diagram of the functions of the box's
  // files that its steps run, between the other boxes; the functions it
  // runs are marked in the files' outlines below.
  const { flow: flowId, step: stepNo } = where();
  const flow = flowId ? flows().find((x) => x.id === flowId) : null;
  wantedAt = flow ? flow.steps.map((s) => s.at).filter(Boolean) : [];
  onFlow = new Set();
  let flowEls = "";
  if (flow) {
    const { shown, view, places } = stepsAt(tier(tierId), flow, id);
    onFlow = places;
    const cur = stepNo ? shown[stepNo - 1] : null;
    let says;
    if (cur) {
      const fn = (key) => view.extra[key];
      const where_ =
        cur.kind === "inside"
          ? `<span class="where">inside ${esc(fn(cur.a) ? fn(cur.a).name : nameOf(cur.a))}</span>`
          : cur.kind === "outside"
            ? '<span class="where">in no tier of the map</span>'
            : fn(cur.a) || fn(cur.b)
              ? `<span class="where">${fn(cur.a) ? `from ${esc(fn(cur.a).name)}` : ""}${fn(cur.a) && fn(cur.b) ? " " : ""}${fn(cur.b) ? `into ${esc(fn(cur.b).name)}` : ""}</span>`
              : "";
      says = stepSaysEl(cur, where_);
    } else says = flowSaysEl(flow);
    flowEls = `${flowBarEl(flow)}${says}${waysEl()}${seqEl(view, shown, stepNo)}`;
  } else flowEls = `${flowBarEl(null)}${waysEl()}`;
  const holds = f.holds
    .map((h) => {
      const name = h.said.split(":")[0];
      const opens = h.exists && h.lines !== null;
      const href = `cox://open${h.path}${h.line ? `#L${h.line}` : ""}`;
      const changed = h.added || h.removed ? `<span class="badge">+${h.added} −${h.removed}</span>` : "";
      const path = `${esc(name)}${h.line ? `<span class="ln">:${h.line}</span>` : ""}`;
      const head = `<div class="head">${opens ? `<a class="path" href="${esc(href)}">${path}</a>` : `<span class="path">${path}</span>`}${h.lines !== null ? `<span class="n">${h.lines} lines</span>` : h.exists ? '<span class="n">folder</span>' : ""}${changed}${h.exists ? "" : '<span class="missing">gone</span>'}</div>`;
      const classes = `hold ${h.added || h.removed ? "lit" : ""} ${h.exists ? "" : "missing"}`;
      return `<div class="${classes}">${head}${opens ? outlineEl(h.path, h.line) : ""}</div>`;
    })
    .join("");
  const chain = [...above(tierId), tierId].map((tid) => ({ text: tier(tid).name, tier: tid }));
  chain.push({ text: p.name });
  app.innerHTML = `${crumbs(chain)}${sourceLine()}
    <div class="says"><b style="color:var(--foreground);font-weight:500">${esc(p.name)}.</b> ${esc(p.says)}${p.each ? ` <span class="each" style="font-size:10.5px;border:1px solid var(--border);border-radius:999px;padding:0 7px">${esc(p.each)}</span>` : ""}</div>
    ${p.kinds && p.kinds.length ? `<div class="says" style="font-family:var(--mono);font-size:11.5px">${p.kinds.map(esc).join(" · ")}</div>` : ""}
    ${p.note ? `<div class="says" style="color:var(--note)"><b>note</b> ${esc(p.note)}</div>` : ""}
    ${f.stale.length ? `<div class="says" style="color:var(--stale)"><b>stale</b> the project no longer has ${f.stale.map(esc).join(", ")}; an agent is to bring the map up to date</div>` : ""}
    ${flowEls}
    <div class="leaf">${holds || '<div class="says">This box holds no paths.</div>'}</div>
    <div class="foot">${flow ? "<span>a column is a function the flow runs, as the language server says of where each step is made; it opens on a tab</span><span>·</span><span>← → go from step to step</span><span>·</span>" : ""}<span>a file, or a symbol in it, opens on a tab of the stage, at its line</span><span>·</span><span>the symbols and their uses are as the project's language server says</span></div>`;
  if (flow) seqScrolled();
}

/// What the language server says of the file at `path`, an absolute
/// path, as the page has heard it so far: its symbols, each a link to
/// its line, the one at `line` marked, and the files that use them.
function outlineEl(path, line) {
  const f = fileData[path];
  if (!f) return '<div class="outline"><span class="lsp">asking the language server…</span></div>';
  const status = f.pending ? `<span class="lsp">${esc(f.said || (f.server ? `${f.server}: asked again` : "asking the language server…"))}</span>` : "";
  if (!f.pending && !f.server) return `<div class="outline"><span class="lsp faint">no language server knows ${f.language ? esc(f.language) : "this kind of file"}</span></div>`;
  const error = f.error ? `<span class="lsp failed">${esc(f.error)}</span>` : "";
  const symbols = (f.symbols || []).map(symEl(path, line, 0)).join("");
  const used = f.used_by && f.used_by.length
    ? `<div class="usedby"><b>used by</b>${f.used_by.map((u) => `<span><a href="cox://open${esc(absolute(u.path))}">${esc(u.path)}</a><span class="n">×${u.count}</span></span>`).join("")}</div>`
    : f.symbols && !f.pending && !f.error
      ? '<div class="usedby faint">used by no other file</div>'
      : "";
  return `<div class="outline">${status}${error}${symbols ? `<div class="symbols">${symbols}</div>` : f.symbols && !f.error ? '<span class="lsp faint">defines no symbols</span>' : ""}${used}</div>`;
}

/// A path as the server tells it, from the project's folder, as an
/// absolute one.
function absolute(path) {
  return path.startsWith("/") ? path : `${data.root}/${path}`;
}

/// A symbol of the file at `path`, as a line of its outline, with those
/// within it where they are worth showing: the methods of an `impl` or a
/// trait, not the fields of a struct, which are counted instead.
function symEl(path, line, depth) {
  return (s) => {
    const kind = s.kind === "object" && /^impl\b/.test(s.name) ? "" : s.kind === "type parameter" ? "type" : s.kind;
    const kids = s.children || [];
    const opens = ["", "module", "namespace", "class", "interface"].includes(kind);
    const within = opens && kids.length ? `<div class="within">${kids.map(symEl(path, line, depth + 1)).join("")}</div>` : "";
    const counted = !opens && kids.length ? `<span class="n">${kids.length} ${kind === "enum" ? "variant" : kind === "struct" ? "field" : "item"}${kids.length === 1 ? "" : "s"}</span>` : "";
    const uses = s.uses === null || s.uses === undefined ? "" : `<span class="uses ${s.uses ? "" : "zero"}">${s.uses} use${s.uses === 1 ? "" : "s"}</span>`;
    const label = s.detail && s.detail.includes(s.name) ? `<span class="detail">${esc(s.detail)}</span>` : `<span class="name">${esc(s.name)}</span>${s.detail ? `<span class="detail">${esc(s.detail)}</span>` : ""}`;
    // The innermost symbol the line is in is marked.
    const holds = (sym) => line && sym.line <= line && line <= sym.end;
    const here = holds(s) && !kids.some(holds) ? "here" : "";
    const ran = onFlow.has(`${path}:${s.line}`) ? "flow" : "";
    return `<div class="sym ${here} ${ran}"><a href="cox://open${esc(path)}#L${s.line}"><span class="ln">${s.line}</span>${kind ? `<span class="kind">${esc(kind)}</span>` : ""}${label}${counted}${uses}${ran ? '<span class="onflow">on the flow</span>' : ""}</a>${within}</div>`;
  };
}

function renderEmpty() {
  if (data.error) {
    app.innerHTML = `<div class="says failed">${esc(data.error)}</div><div class="says">The map is <code>${esc(data.file)}</code>. An agent fixes it; ask it to.</div>`;
    return;
  }
  app.innerHTML = `<div class="says empty">This project has no map yet. An agent writes one at <code>.cox/map.json</code>: ask it, in the chat, to map the project.</div>`;
}

function render() {
  if (!data) return;
  wanted = [];
  wantedAt = [];
  if (!data.map || !tiers().length) return renderEmpty();
  const { tier: tierId, part } = where();
  if (part && parts()[part]) renderLeaf(tierId, part);
  else renderTier(tierId);
}

// --- The wires, drawn over the cards once they are laid out.

function svgIn(el, which = "wires") {
  let svg = el.querySelector(`:scope > svg.${which}`);
  if (!svg) {
    svg = document.createElementNS(NS, "svg");
    svg.setAttribute("class", which);
    if (which === "wires") el.prepend(svg);
    else el.append(svg);
  }
  const W = el.scrollWidth, H = el.scrollHeight;
  svg.setAttribute("width", W);
  svg.setAttribute("height", H);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  return svg;
}

/// The lines in the gaps of a row.
function wireLine(line) {
  const svg = svgIn(line), labels = svgIn(line, "labels");
  const box = line.getBoundingClientRect();
  const out = [], texts = [];
  for (const gap of line.querySelectorAll(":scope > .gap")) {
    const g = gap.getBoundingClientRect();
    const prev = gap.previousElementSibling, next = gap.nextElementSibling;
    if (!prev || !next) continue;
    const p = prev.getBoundingClientRect(), n = next.getBoundingClientRect();
    const y = Math.min(p.top + Math.min(p.height, n.height) * 0.4, p.bottom - 10) - box.top + line.scrollTop;
    const x1 = p.right - box.left + line.scrollLeft + 2, x2 = n.left - box.left + line.scrollLeft - 2;
    const dir = gap.dataset.dir;
    const end = dir === "to" || dir === "both", start = dir === "from" || dir === "both";
    const { cls, badges } = flowDress(gap.dataset.from, gap.dataset.to, [{ x: x1, y }, { x: x2, y }], false, end, start);
    out.push(`<path class="${cls}" d="M${x1},${y} L${x2},${y}"${markers(end, start, cls)}/>`);
    if (gap.dataset.label) texts.push(`<text class="${cls}" x="${(x1 + x2) / 2}" y="${y - 6}" text-anchor="middle">${esc(gap.dataset.label)}</text>`);
    badges.forEach((b) => texts.push(badgeEl(b)));
  }
  svg.innerHTML = DEFS + out.join("");
  labels.innerHTML = texts.join("");
}

/// The arrow heads of a line, lit with it.
function markers(end, start, cls) {
  const arrow = cls.includes("lit") ? "url(#arrlit)" : "url(#arr)";
  return `${end ? ` marker-end="${arrow}"` : ""}${start ? ` marker-start="${arrow}"` : ""}`;
}

/// How a line from `from` to `to`, along `points`, is drawn while a flow
/// is chosen: dashed when it is the flow's own, with no edge in the map,
/// lit when the step chosen takes it, dimmed when no step does; and the
/// badges of the steps that take it, numbered, each near the end the
/// step goes into. Of several lines between two boxes, a step takes the
/// one whose arrow points its way (`end` into `to`, `start` into
/// `from`), or any when none does.
function flowDress(from, to, points, flowOnly, end = true, start = false) {
  const cls = flowOnly ? ["flow"] : [];
  if (!flowOn) return { cls: cls.join(" "), badges: [] };
  const takes = (a, b, pointed) => (pointed || !flowPointed.has(`${a}>${b}`) ? flowMarks[`${a}>${b}`] || [] : []);
  const forth = takes(from, to, end), back = takes(to, from, start);
  if (!forth.length && !back.length) return { cls: "off", badges: [] };
  if ([...forth, ...back].some((m) => m.lit)) cls.push("lit");
  const badges = [];
  const along = (pts, marks) => {
    const clean = pts.filter((p, i) => i === 0 || Math.hypot(p.x - pts[i - 1].x, p.y - pts[i - 1].y) > 0.5);
    if (clean.length < 2 || !marks.length) return;
    const E = clean[clean.length - 1], P = clean[clean.length - 2];
    const len = Math.hypot(P.x - E.x, P.y - E.y) || 1, ux = (P.x - E.x) / len, uy = (P.y - E.y) / len;
    marks.forEach((m, k) => badges.push({ x: E.x + ux * (17 + 16 * k), y: E.y + uy * (17 + 16 * k), n: m.n, lit: m.lit }));
  };
  along(points, forth);
  along([...points].reverse(), back);
  return { cls: cls.join(" "), badges };
}
const badgeEl = (b) =>
  `<g class="badge ${b.lit ? "lit" : ""}"><circle cx="${b.x}" cy="${b.y}" r="7.5"/><text x="${b.x}" y="${b.y}" dy="3.4" text-anchor="middle">${b.n}</text></g>`;

/// How far apart the lines run in a channel between two lines of cards,
/// how far the first runs from the line above, and the last from the
/// line below.
const LANE = 16, LANE_FIRST = 14, LANE_LAST = 10;
/// How close a line comes to a card it passes beside.
const CLEAR = 10;

/// A path along `points`, its corners rounded where the segments allow.
function rounded(points) {
  const pts = points.filter((q, i) => i === 0 || Math.abs(q.x - points[i - 1].x) > 0.5 || Math.abs(q.y - points[i - 1].y) > 0.5);
  if (pts.length < 2) return "";
  let d = `M${pts[0].x},${pts[0].y}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const a = pts[i - 1], c = pts[i], b = pts[i + 1];
    const r = Math.min(6, Math.hypot(c.x - a.x, c.y - a.y) / 2, Math.hypot(b.x - c.x, b.y - c.y) / 2);
    const ua = { x: Math.sign(a.x - c.x), y: Math.sign(a.y - c.y) }, ub = { x: Math.sign(b.x - c.x), y: Math.sign(b.y - c.y) };
    d += ` L${c.x + ua.x * r},${c.y + ua.y * r} Q${c.x},${c.y} ${c.x + ub.x * r},${c.y + ub.y * r}`;
  }
  const last = pts[pts.length - 1];
  return d + ` L${last.x},${last.y}`;
}

/// Where a line's label goes: over the widest stretch of the run that no
/// other line crosses, at `crossings`, when it is wide enough for the
/// label, or else beside the line where it turns down at the run's end.
function labelAt(run, text, crossings = [], width = 0) {
  const need = (text || "").length * 6.3 + 12;
  const cuts = [run.x1, ...crossings.filter((x) => x > run.x1 && x < run.x2).sort((a, b) => a - b), run.x2];
  let best = null;
  for (let i = 1; i < cuts.length; i++) {
    const width = cuts[i] - cuts[i - 1];
    if (width >= need && (!best || width > best[1] - best[0])) best = [cuts[i - 1], cuts[i]];
  }
  if (best) return [(best[0] + best[1]) / 2, run.y - 4, "middle"];
  // Beside the line where it turns, past the run's end or before its
  // start, whichever fewer lines cross and the page has room for.
  const all = [...crossings, run.xs, run.xe];
  const choices = [
    [run.xe + 7, "start", [run.xe + 7, run.xe + 7 + need]],
    [run.xs - 7, "end", [run.xs - 7 - need, run.xs - 7]],
  ].filter(([, , [x1, x2]]) => x1 >= 4 && (!width || x2 <= width - 4));
  const crossed = ([, , [x1, x2]]) => all.filter((x) => x > x1 && x < x2).length;
  const pick = choices.sort((a, b) => crossed(a) - crossed(b))[0] || [run.xe + 7, "start"];
  return [pick[0], run.y - 4, pick[1]];
}

/// The lines of a tier, past the gaps of its lines of cards: the frame's
/// cards above, its rows, and the frame's cards below. Each runs from the
/// bottom of a card into the channel under its line, along it, and down
/// into the top of a lower card, up from the lower card the same way, or
/// back up into the bottom of another card of the line; one that skips
/// lines runs down beside the cards of the lines between, where none is,
/// or the nearest free way. The channel under the frame's cards above
/// lies over the focus, and a line into the frame's cards below runs
/// along a channel under the focus, so that the focus holds only the
/// lines among its own rows, which keep clear of its legend. The lines of
/// one channel each take a lane of their own, so their labels, on the
/// lanes, never meet, and what is under the channel moves down to make
/// room for the lanes.
function route(wires, ctx) {
  const { rows, rowOf, card, width, focus, legend, first, last } = ctx;
  // The frame's line below the focus, if it has one.
  const below = last + 1 < rows.length ? last + 1 : -1;
  // What a line passing a line of cards keeps clear of: its cards, and
  // the legend over the focus's first row.
  const blocks = (ri) => (ri === first && legend ? [...rows[ri].cards, legend] : rows[ri].cards);
  // Each card's ports, each sorted by where its line goes.
  const ports = {};
  const port = (id, which, toward) => {
    const key = `${id}/${which}`;
    (ports[key] = ports[key] || []).push(toward);
    return key;
  };
  const jobs = [];
  for (const w of wires) {
    const A = card(w.from), B = card(w.to);
    if (!A || !B) continue;
    const ra = rowOf(w.from), rb = rowOf(w.to);
    if (ra === rb) {
      jobs.push({ w, kind: "loop", A, B, row: ra, pa: port(w.from, "bottom", B.l + B.w / 2), pb: port(w.to, "bottom", A.l + A.w / 2) });
      continue;
    }
    const upward = rb < ra;
    const U = upward ? B : A, L = upward ? A : B, rU = Math.min(ra, rb), rL = Math.max(ra, rb);
    const want = L.l + L.w / 2;
    const skips = rL - rU > 1;
    const between = [];
    for (let ri = rU + 1; ri < rL; ri++) between.push(...blocks(ri));
    const via = skips ? freeWay(want, between, width) : { x: want };
    jobs.push({ w, kind: "down", upward, U, L, rU, rL, skips, via: via.x, interval: via.interval, pu: port(upward ? w.to : w.from, "bottom", skips ? via.x : want), pl: port(upward ? w.from : w.to, "top", U.l + U.w / 2) });
  }
  // Lines through one narrow way, a gap with a line and a label of its
  // own in the middle, keep to its two ends, the first at the left.
  const ways = {};
  for (const q of jobs) if (q.kind === "down" && q.interval) (ways[q.interval.join()] = ways[q.interval.join()] || []).push(q);
  for (const group of Object.values(ways)) {
    const [x1, x2] = group[0].interval;
    group.sort((a, b) => a.via - b.via).forEach((q, k) => (q.via = k % 2 ? x2 - Math.floor(k / 2) * 12 : x1 + Math.floor(k / 2) * 12));
  }
  // A port's place: the card's edge shared out among its lines, in the
  // order of where they go, so that they do not cross as they leave. A
  // line into the top of the focus's first row keeps clear of the legend.
  const portAt = {};
  for (const [key, towards] of Object.entries(ports)) {
    const [id, which] = key.split("/");
    const c = card(id);
    const order = towards.map((x, i) => [x, i]).sort((a, b) => a[0] - b[0]);
    const at = {};
    order.forEach(([, i], k) => (at[i] = c.l + (c.w * (k + 1)) / (order.length + 1)));
    if (which === "top" && legend && rowOf(id) === first) {
      for (const i of Object.keys(at)) if (at[i] > legend.l - 8 && at[i] < legend.l + legend.w + 8) at[i] = Math.min(legend.l + legend.w + 10, c.l + c.w - 8);
    }
    portAt[key] = towards.map((_, i) => at[i]);
  }
  const used = {};
  const at = (key) => {
    const n = used[key] || 0;
    used[key] = n + 1;
    return portAt[key][n];
  };
  // The channels: one under each line of cards, from the bottom of its
  // cards, and one under the focus, for the lines into the frame below.
  const channels = rows.map((r) => ({ runs: [], top: r.bottom }));
  if (below >= 0) channels.push({ runs: [], top: focus.t + focus.h });
  const under = (rU, rL) => (rL === below ? channels.length - 1 : rU);
  for (const q of jobs) {
    if (q.kind === "loop") {
      const xa = at(q.pa), xb = at(q.pb);
      q.runs = [{ q, loop: true, x1: Math.min(xa, xb), x2: Math.max(xa, xb), xs: xa, xe: xb, label: true }];
      channels[q.row].runs.push(q.runs[0]);
    } else {
      const xu = at(q.pu), xl = at(q.pl);
      if (!q.skips) {
        q.runs = [{ q, xs: xu, xe: xl, x1: Math.min(xu, xl), x2: Math.max(xu, xl), label: true }];
        channels[under(q.rU, q.rL)].runs.push(q.runs[0]);
      } else {
        const first = { q, xs: xu, xe: q.via, x1: Math.min(xu, q.via), x2: Math.max(xu, q.via), label: true };
        const last = { q, xs: q.via, xe: xl, x1: Math.min(q.via, xl), x2: Math.max(q.via, xl), label: false };
        if (last.x2 - last.x1 > first.x2 - first.x1 + 40) [first.label, last.label] = [false, true];
        channels[q.rU].runs.push(first);
        channels[under(q.rL - 1, q.rL)].runs.push(last);
        q.runs = [first, last];
      }
    }
  }
  // The lanes of each channel: loops first, the shorter the higher, then
  // the runs, those going right from further right first and those going
  // left from further left first, which keeps them from crossing.
  const needed = channels.map(() => 0);
  channels.forEach(({ runs, top }, c) => {
    runs.sort((a, b) => {
      if (Boolean(a.loop) !== Boolean(b.loop)) return a.loop ? -1 : 1;
      if (a.loop) return a.x2 - a.x1 - (b.x2 - b.x1);
      const ar = a.xe >= a.xs, br = b.xe >= b.xs;
      if (ar !== br) return ar ? -1 : 1;
      return ar ? b.xs - a.xs : a.xs - b.xs;
    });
    runs.forEach((run, k) => (run.y = top + LANE_FIRST + k * LANE));
    needed[c] = runs.length ? LANE_FIRST + (runs.length - 1) * LANE + LANE_LAST : 0;
    // Where the other lines of the channel cross each lane: a line drops
    // from above to its lane, and from its lane down, so a loop's two
    // ends and a run's start cross the lanes above it, and a run's end
    // the lanes below.
    for (const run of runs) {
      run.crossings = [];
      for (const other of runs) {
        if (other === run) continue;
        if (other.y > run.y) run.crossings.push(other.xs, ...(other.loop ? [other.xe] : []));
        else if (!other.loop) run.crossings.push(other.xe);
      }
    }
  });
  const drawn = [];
  for (const q of jobs) {
    const w = q.w;
    let points, label = null;
    const bottom = (c) => c.t + c.h + (c.many ? 8 : 0);
    if (q.kind === "loop") {
      const [run] = q.runs;
      points = [{ x: run.xs, y: bottom(q.A) }, { x: run.xs, y: run.y }, { x: run.xe, y: run.y }, { x: run.xe, y: bottom(q.B) }];
      label = labelAt({ ...run, xe: run.x2 }, w.label, run.crossings, width);
    } else {
      const [first, last] = q.runs;
      points = [{ x: first.xs, y: bottom(q.U) }, { x: first.xs, y: first.y }, { x: first.xe, y: first.y }];
      if (last) points.push({ x: last.xs, y: last.y }, { x: last.xe, y: last.y });
      points.push({ x: (last || first).xe, y: q.L.t });
      if (q.upward) points.reverse();
      const on = first.label ? first : last;
      label = labelAt(on, w.label, on.crossings, width);
    }
    const end = w.dir === "to" || w.dir === "both", start = w.dir === "from" || w.dir === "both";
    drawn.push({ d: rounded(points), end, start, text: w.label, label, from: w.from, to: w.to, points, flow: Boolean(w.flow) });
  }
  return { drawn, needed };
}

/// Where a line passes the lines of cards between, by `want` where nothing
/// is in the way among `cards`, or else the nearest free way: not a narrow
/// one, which is a gap with a line of its own in it, unless it is much
/// nearer.
function freeWay(want, cards, width) {
  const blocked = cards.some((c) => c.l - CLEAR < want && want < c.l + c.w + CLEAR);
  if (!blocked) return { x: want };
  const sorted = [...cards].sort((a, b) => a.l - b.l);
  const free = [];
  let edge = 8;
  for (const c of sorted) {
    if (c.l - CLEAR > edge) free.push([edge, c.l - CLEAR]);
    edge = Math.max(edge, c.l + c.w + CLEAR);
  }
  free.push([edge, Math.max(edge, width - 8)]);
  let best = null, cost = Infinity;
  for (const [x1, x2] of free) {
    const x = Math.min(Math.max(want, x1), x2);
    const narrow = x2 - x1 < 60;
    const c = Math.abs(x - want) + (narrow ? 80 : 0);
    if (c < cost) [best, cost] = [{ x, interval: narrow ? [x1, x2] : null }, c];
  }
  return best;
}

/// The lines of a tier past the gaps: guides from a card to the rows under
/// it, and the lines between its lines of cards, the frame's included.
function wireDrill(d) {
  const keys = JSON.parse(d.dataset.keys || "{}");
  // A row under a card starts under that card, wherever the card is in
  // its row.
  for (const row of d.querySelectorAll(".row[data-parent]")) {
    const parent = d.querySelector(`[data-k="${keys[row.dataset.parent]}"]`);
    if (!parent) continue;
    const want = Math.round(parent.getBoundingClientRect().left - row.getBoundingClientRect().left + 26);
    if (parseInt(row.style.paddingLeft, 10) !== want) row.style.paddingLeft = want + "px";
  }
  const svg = svgIn(d), labels = svgIn(d, "labels");
  const box = d.getBoundingClientRect();
  const rel = (el) => {
    const r = el.getBoundingClientRect();
    return { l: r.left - box.left, t: r.top - box.top, w: r.width, h: r.height, many: el.classList.contains("many") };
  };
  const out = [], texts = [];
  // The lines of cards, from the top: the frame's above, the focus's rows,
  // and the frame's below.
  const rows = [...d.querySelectorAll(":scope > .side, :scope > .focus > .row")].map((row) => {
    const cs = [...row.querySelectorAll(":scope > .line > .part")].map(rel);
    const name = row.querySelector(":scope > .rowname");
    return { el: row, side: row.classList.contains("side"), cards: cs, bottom: Math.max(...cs.map((c) => c.t + c.h + (c.many ? 8 : 0)), 0), nameHeight: name ? name.getBoundingClientRect().height + 6 : 0 };
  });
  const first = rows.findIndex((r) => !r.side);
  const last = rows.length - 1 - [...rows].reverse().findIndex((r) => !r.side);
  const rowOf = (id) => Number((keys[id] || "r-1").match(/^r(-?\d+)/)[1]);
  const measured = {};
  const card = (id) => {
    if (!(id in measured)) {
      const el = keys[id] ? d.querySelector(`[data-k="${keys[id]}"]`) : null;
      measured[id] = el ? rel(el) : null;
    }
    return measured[id];
  };
  const focusEl = d.querySelector(".focus");
  const focus = rel(focusEl);
  const legendEl = d.classList.contains("noframe") ? null : focusEl.querySelector(":scope > .legend");
  const legend = legendEl ? rel(legendEl) : null;
  const all = JSON.parse(d.dataset.wires || "[]");
  for (const w of all) {
    if (!w.tree) continue;
    const A = card(w.from), b = d.querySelector(`[data-k="${w.toKey}"]`);
    if (!A || !b) continue;
    const B = rel(b);
    const x = A.l + 12, y1 = A.t + A.h + (A.many ? 8 : 0), y2 = B.t + B.h / 2;
    out.push(`<path d="M${x},${y1} L${x},${y2 - 8} Q${x},${y2} ${x + 8},${y2} L${B.l},${y2}" marker-end="url(#arr)"/>`);
  }
  if (first >= 0) {
    const { drawn, needed } = route(all.filter((w) => !w.tree), { rows, rowOf, card, width: box.width, focus, legend, first, last });
    for (const { d: path, end, start, text, label, from, to, points, flow } of drawn) {
      const { cls, badges } = flowDress(from, to, points, flow, end, start);
      out.push(`<path class="${cls}" d="${path}"${markers(end, start, cls)}/>`);
      if (text && label) texts.push(`<text class="${cls}" x="${label[0]}" y="${label[1]}" text-anchor="${label[2]}">${esc(text)}</text>`);
      badges.forEach((b) => texts.push(badgeEl(b)));
    }
    // What is under a channel moves down to make room for its lanes: the
    // next line of cards, the focus as a whole under the frame's cards
    // above, and the frame's cards below under the channel under the
    // focus; the focus grows under its last row for the lanes there.
    const move = (el, natural, need) => {
      const margin = parseFloat(el.style.marginTop) || 0;
      const want = Math.max(0, Math.round(need - (natural - margin)));
      if (Math.abs(want - margin) > 1) el.style.marginTop = want ? want + "px" : "";
    };
    const topOf = (row) => Math.min(...row.cards.map((c) => c.t), Infinity);
    rows.forEach((row, i) => {
      const next = rows[i + 1];
      if (!next) return;
      if (i === last) {
        const want = needed[i] ? Math.round(needed[i] + 4) : 0;
        const has = parseFloat(focusEl.style.paddingBottom) || 0;
        if (Math.abs(want - has) > 1) focusEl.style.paddingBottom = want ? want + "px" : "";
        const need = needed[needed.length - 1];
        move(next.el, topOf(next) - (focus.t + focus.h), need ? need + 6 : 0);
        return;
      }
      if (i + 1 === first) move(focusEl, focus.t - row.bottom, needed[i] ? needed[i] + 6 : 0);
      else move(next.el, topOf(next) - row.bottom, needed[i] + 6 + (needed[i] ? next.nameHeight : 0));
    });
  }
  svg.innerHTML = DEFS + out.join("");
  labels.innerHTML = texts.join("");
}

function wireAll() {
  app.querySelectorAll(".line").forEach(wireLine);
  app.querySelectorAll(".drill[data-wires]").forEach(wireDrill);
}
const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => wireAll());
function wired() {
  requestAnimationFrame(() => {
    wireAll();
    if (ro) {
      ro.disconnect();
      ro.observe(app);
      app.querySelectorAll(".line, .drill").forEach((el) => ro.observe(el));
    }
  });
}
if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => wireAll());

// --- The code a step is made at, under the mouse.

// The lines around each place the mouse rested on, as the page last read
// them, by `path:line`.
const lineData = {};
let peekEl = null, peekAt = null, peekTimer = null, peekGoing = null;

/// Takes the peek down.
function peekHide() {
  clearTimeout(peekTimer);
  peekTimer = null;
  peekStay();
  if (peekEl) peekEl.remove();
  peekEl = null;
  peekAt = null;
}
/// Takes the peek down in a moment, unless the mouse comes onto it, or
/// back onto its step, first: so the mouse crosses from the one to the
/// other, to click a line of the code.
function peekHideSoon() {
  clearTimeout(peekGoing);
  peekGoing = setTimeout(peekHide, 220);
}
function peekStay() {
  clearTimeout(peekGoing);
  peekGoing = null;
}

/// Shows, by `row`, a row of the sequence diagram, the code its step is
/// made at: the step, the file and line, the function the line is in
/// and what it calls, as the server has said, and the lines around it,
/// the line itself marked, each a link that opens the file there on a
/// tab. The lines are read when the mouse comes, and shown as last read
/// until they come.
async function peekShow(row, st) {
  const place = atPlace(st.step.at);
  if (!place) return;
  const key = `${place.path}:${place.line}`;
  peekAt = key;
  const draw = () => {
    if (peekAt !== key || !row.isConnected) return peekHide();
    const got = lineData[key];
    const c = callOf(st.step);
    const said = c && c.function ? `in <b>${esc(c.function.name)}</b>${c.calls && c.calls.length ? `, calls ${c.calls.map((x) => `<b>${esc(x.name)}</b>`).join(", ")}` : ""}` : c && c.error ? esc(c.error) : "";
    const lines = got
      ? got.lines.length
        ? got.lines.map((line, i) => `<a class="ln ${got.from + i === place.line ? "at" : ""}" href="${esc(`cox://open${place.path}#L${got.from + i}`)}" title="opens ${esc(got.name)}:${got.from + i} on a tab"><span class="no">${got.from + i}</span>${line.map(([text, what]) => (what ? `<span class="hl-${what.replace(/\./g, "-")}">${esc(text)}</span>` : esc(text))).join("") || " "}</a>`).join("")
        : `<div class="ln faint">${got.exists ? "the file has no such line" : "the file is gone"}</div>`
      : '<div class="ln faint">reading…</div>';
    if (!peekEl) {
      peekEl = document.createElement("div");
      peekEl.className = "peek";
      peekEl.addEventListener("mouseenter", peekStay);
      peekEl.addEventListener("mouseleave", peekHideSoon);
      peekEl.addEventListener("click", (event) => {
        if (event.target.closest("a")) setTimeout(peekHide, 0);
      });
      document.body.appendChild(peekEl);
    }
    peekEl.innerHTML = `<div class="head"><span class="n">${st.n}</span> ${esc(nameOf(st.step.from))} → ${esc(nameOf(st.step.to))}: ${esc(st.step.says)}</div><div class="where"><a class="at" href="${esc(`cox://open${place.path}#L${place.line}`)}" title="opens the file there on a tab">${esc(st.step.at)}</a>${said ? ` · ${said}` : ""}</div><pre>${lines}</pre>`;
    const r = row.getBoundingClientRect();
    const seq = row.closest(".seq");
    const s = seq ? seq.getBoundingClientRect() : r;
    const w = peekEl.offsetWidth, h = peekEl.offsetHeight;
    let x = Math.max(8, Math.min(Math.max(s.left + 44, r.left), window.innerWidth - w - 8));
    let y = r.bottom + 6;
    if (y + h > window.innerHeight - 8) y = Math.max(8, r.top - h - 6);
    peekEl.style.left = `${Math.round(x)}px`;
    peekEl.style.top = `${Math.round(y)}px`;
  };
  draw();
  try {
    const response = await fetch(`cox://api/lines${key}`);
    if (response.ok) lineData[key] = await response.json();
  } catch {
    // Shown as last read.
  }
  draw();
}

/// The step a row of the sequence diagram shows, as the page shows the
/// flow chosen now.
function stepOfRow(row) {
  const { flow: flowId } = where();
  const flow = flowId ? flows().find((f) => f.id === flowId) : null;
  const n = Number(row.dataset.step);
  const step = flow && flow.steps[n - 1];
  return step ? { n, step } : null;
}

app.addEventListener("mouseover", (event) => {
  const row = event.target.closest(".seq .row[data-step]");
  if (!row) return;
  if (row.classList.contains("outside")) return;
  const st = stepOfRow(row);
  const place = st && atPlace(st.step.at);
  if (!place) return;
  const key = `${place.path}:${place.line}`;
  if (peekAt === key && peekEl) return peekStay();
  peekHide();
  peekTimer = setTimeout(() => peekShow(row, st), 180);
});
app.addEventListener("mouseout", (event) => {
  const row = event.target.closest(".seq .row[data-step]");
  if (!row) return;
  if (event.relatedTarget && (row.contains(event.relatedTarget) || (peekEl && peekEl.contains(event.relatedTarget)))) return;
  if (peekEl) peekHideSoon();
  else peekHide();
});
document.addEventListener("scroll", peekHide, true);

// --- Going places.

app.addEventListener("click", (event) => {
  const t = event.target.closest("[data-goes],[data-tier],[data-flow],[data-step],[data-drill],[data-view],[data-seq],[data-back],[data-forward],[data-up]");
  if (!t) return;
  if ("back" in t.dataset) return back();
  if ("forward" in t.dataset) return forward();
  if ("up" in t.dataset) return up();
  const { tier: here, part, flow, step, onMap } = where();
  const kept = { flow, step, onMap };
  if (t.dataset.tier) {
    go(t.dataset.tier, t.dataset.part, kept);
    return;
  }
  if ("flow" in t.dataset) {
    go(here, part, { flow: t.dataset.flow === flow ? null : t.dataset.flow, onMap });
    return;
  }
  if (t.dataset.drill) {
    const [tierId, partId] = t.dataset.drill.split("/");
    const row = t.closest("[data-step]");
    go(tierId, partId || null, { flow, step: row ? Number(row.dataset.step) : step, onMap });
    return;
  }
  if (t.dataset.step) {
    go(here, part, { flow, step: Number(t.dataset.step) === step ? null : Number(t.dataset.step), onMap });
    return;
  }
  if (t.dataset.view) {
    go(here, null, { flow, step, onMap: t.dataset.view === "map" });
    return;
  }
  if (t.dataset.seq) {
    const p = parts()[t.dataset.seq];
    if (p && p.tier) go(p.tier, null, kept);
    else if (p && p.holds && p.holds.length) go(here, t.dataset.seq, kept);
    return;
  }
  const id = t.dataset.id, p = parts()[id];
  if (t.dataset.goes === "tier") go(p.tier, null, kept);
  else if (t.dataset.goes === "leaf") go(here, id, kept);
  else if (t.dataset.goes === "up") {
    const up = tierOf(id);
    if (up) go(up.id, null, kept);
  }
});
// The keys: Backspace, Alt-← and ⌘[ go back to where the page was, and
// Alt-→ and ⌘] forward again; ↑ goes up a tier; and, with a flow
// chosen, ← and → go from step to step.
window.addEventListener("keydown", (event) => {
  const alt = event.altKey && !event.metaKey && !event.ctrlKey;
  const cmd = (event.metaKey || event.ctrlKey) && !event.altKey;
  const plain = !event.altKey && !event.metaKey && !event.ctrlKey;
  if ((plain && event.key === "Backspace") || (alt && event.key === "ArrowLeft") || (cmd && event.key === "[")) back();
  else if ((alt && event.key === "ArrowRight") || (cmd && event.key === "]")) forward();
  else if (plain && event.key === "ArrowUp") up();
  else if (plain && (event.key === "ArrowRight" || event.key === "ArrowLeft")) {
    const { tier: here, part, flow, step, onMap } = where();
    const f = flow && flows().find((x) => x.id === flow);
    if (!f || !f.steps.length) return;
    if (event.key === "ArrowRight") go(here, part, { flow, step: Math.min(f.steps.length, (step || 0) + 1), onMap });
    else go(here, part, { flow, step: Math.max(1, (step || f.steps.length + 1) - 1), onMap });
  } else return;
  event.preventDefault();
});
window.addEventListener("hashchange", () => {
  track();
  render();
});
track();
// The sequence diagram shares the page's width among its columns, so it
// is drawn again when the width changes.
let seqWidth = 0;
window.addEventListener("resize", () => {
  const { flow, onMap } = where();
  if (!flow || onMap || app.clientWidth === seqWidth) return;
  seqWidth = app.clientWidth;
  render();
});

// --- Following what Cox knows.

async function load() {
  try {
    const response = await fetch(API);
    if (!response.ok) throw new Error(`${API} answered ${response.status}`);
    const text = await response.text();
    if (text !== lastText) {
      lastText = text;
      data = JSON.parse(text);
      render();
    }
  } catch (failed) {
    if (!data) app.innerHTML = `<div class="says failed">${esc(String(failed))}</div>`;
  }
  await loadFiles();
}

/// Reads again what the server says of the files the page shows, and of
/// the places the steps of the flow shown are made, and draws the page
/// again when it changed.
async function loadFiles() {
  let changed = false;
  const read = async (url, texts, read, key) => {
    try {
      const response = await fetch(url);
      if (!response.ok) return;
      const text = await response.text();
      if (text === texts[key]) return;
      texts[key] = text;
      read[key] = JSON.parse(text);
      changed = true;
    } catch {
      // Asked again next time.
    }
  };
  await Promise.all([
    ...wanted.map((path) => read(`cox://api/file${path}`, fileText, fileData, path)),
    ...wantedAt.map((at) => {
      const place = atPlace(at);
      if (!place) return null;
      const key = `${place.path}:${place.line}`;
      return read(`cox://api/call${key}`, callText, callData, key);
    }),
  ]);
  if (changed) render();
}
load();
setInterval(load, EVERY);
