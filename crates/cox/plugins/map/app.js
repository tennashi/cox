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
// card open on its own, as `#cox/stage`.

const API = "cox://api/map";
const EVERY = 2000;
const NS = "http://www.w3.org/2000/svg";
const DEFS =
  '<defs><marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="currentColor"/></marker></defs>';

const app = document.getElementById("app");
let data = null;
let lastText = null;
// What the server says of the files the page shows, by their path, as
// text, to see whether it changed, and read.
const fileText = {};
const fileData = {};
let wanted = [];

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

// --- What the map says, and where things are in it.

const parts = () => (data && data.map && data.map.parts) || {};
const tiers = () => (data && data.map && data.map.tiers) || [];
const edges = () => (data && data.map && data.map.edges) || [];
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

function where() {
  const hash = decodeURIComponent(location.hash.replace(/^#/, ""));
  const [tierId, partId] = hash.split("/");
  const first = tiers()[0];
  return { tier: (tierId && tier(tierId) ? tierId : first && first.id) || null, part: partId || null };
}
function go(tierId, partId) {
  location.hash = partId ? `${tierId}/${partId}` : tierId;
}

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
            gap = `<span class="gap" data-label="${esc(e.says)}" data-dir="${dir}" style="flex-basis:${Math.max(44, e.says.length * 6.5 + 16)}px"></span>`;
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
  const opener = openerOf(tierId);
  const head = opener ? parts()[opener] : null;
  const noframe = !aboveIds.length && !belowIds.length;
  const legend = head ? `<div class="legend"><span class="name">${esc(head.name)}</span>${head.each ? `<span class="each">${esc(head.each)}</span>` : ""}</div>` : "";
  const focus = rows.filter((_, ri) => !lines[ri].side).join("");
  const body = `<div class="drill ${noframe ? "noframe" : ""}" data-wires="${esc(JSON.stringify(wires))}" data-keys="${esc(JSON.stringify(keyOf))}">
    ${aboveIds.length ? rows[0] : ""}
    <div class="focus">${legend}${focus}</div>
    ${belowIds.length ? rows[rows.length - 1] : ""}
  </div>`;
  const chain = [...above(tierId), tierId].map((id) => ({ text: tier(id).name, tier: id }));
  app.innerHTML = `${crumbs(chain)}${sourceLine()}${t.says ? `<div class="says">${esc(t.says)}</div>` : ""}${body}
    <div class="foot"><span>a card opens the tier inside it, or the code it holds</span><span>·</span><span>orange is what changed, as Changes shows it</span></div>`;
  wired();
}

function renderLeaf(tierId, id) {
  const p = parts()[id] || { name: id, says: "" };
  const f = facts(id);
  wanted = f.holds.filter((h) => h.exists && h.lines !== null).map((h) => h.path);
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
    <div class="leaf">${holds || '<div class="says">This box holds no paths.</div>'}</div>
    <div class="foot"><span>a file, or a symbol in it, opens on a tab of the stage, at its line</span><span>·</span><span>the symbols and their uses are as the project's language server says</span></div>`;
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
    return `<div class="sym ${here}"><a href="cox://open${esc(path)}#L${s.line}"><span class="ln">${s.line}</span>${kind ? `<span class="kind">${esc(kind)}</span>` : ""}${label}${counted}${uses}</a>${within}</div>`;
  };
}

function renderEmpty() {
  if (data.error) {
    app.innerHTML = `<div class="says failed">${esc(data.error)}</div><div class="says">The map is <code>${esc(data.file)}</code>. An agent fixes it; ask it to.</div>`;
    return;
  }
  app.innerHTML = `<div class="says empty">This project has no map yet. An agent writes one at <code>.cox/map.json</code>: ask it to map the project, or to show the map, and it learns how.</div>`;
}

function render() {
  if (!data) return;
  wanted = [];
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
    out.push(`<path d="M${x1},${y} L${x2},${y}"${end ? ' marker-end="url(#arr)"' : ""}${start ? ' marker-start="url(#arr)"' : ""}/>`);
    if (gap.dataset.label) texts.push(`<text x="${(x1 + x2) / 2}" y="${y - 6}" text-anchor="middle">${esc(gap.dataset.label)}</text>`);
  }
  svg.innerHTML = DEFS + out.join("");
  labels.innerHTML = texts.join("");
}

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
    drawn.push({ d: rounded(points), end, start, text: w.label, label });
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
    for (const { d: path, end, start, text, label } of drawn) {
      out.push(`<path d="${path}"${end ? ' marker-end="url(#arr)"' : ""}${start ? ' marker-start="url(#arr)"' : ""}/>`);
      if (text && label) texts.push(`<text x="${label[0]}" y="${label[1]}" text-anchor="${label[2]}">${esc(text)}</text>`);
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
  app.querySelectorAll(".drill").forEach(wireDrill);
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

// --- Going places.

app.addEventListener("click", (event) => {
  const t = event.target.closest("[data-goes],[data-tier]");
  if (!t) return;
  if (t.dataset.tier) {
    go(t.dataset.tier, t.dataset.part);
    return;
  }
  const id = t.dataset.id, p = parts()[id];
  const { tier: here } = where();
  if (t.dataset.goes === "tier") go(p.tier);
  else if (t.dataset.goes === "leaf") go(here, id);
  else if (t.dataset.goes === "up") {
    const up = tierOf(id);
    if (up) go(up.id);
  }
});
window.addEventListener("hashchange", render);

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

/// Reads again what the server says of the files the page shows, and
/// draws the page again when it changed.
async function loadFiles() {
  let changed = false;
  await Promise.all(
    wanted.map(async (path) => {
      try {
        const response = await fetch(`cox://api/file${path}`);
        if (!response.ok) return;
        const text = await response.text();
        if (text === fileText[path]) return;
        fileText[path] = text;
        fileData[path] = JSON.parse(text);
        changed = true;
      } catch {
        // Asked again next time.
      }
    }),
  );
  if (changed) render();
}
load();
setInterval(load, EVERY);
