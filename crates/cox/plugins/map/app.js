// The map of the project, drawn by a page of the stage: the picture of the
// project in tiers that an agent wrote in its `.cox/map.json`, read as
// JSON from `cox://api/map` with what Cox knows laid over it, and read
// again every two seconds, as Cox's own tabs follow what they show. A
// tier is rows of cards, with the cards of the tiers above that stay
// beside it as a frame, and lines between them for the edges; a card
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
  const inRows = partsOf(t);
  const frame = t.frame || { left: [], right: [] };
  const frameIds = [...(frame.left || []), ...(frame.right || [])];
  const keyOf = {};
  const wires = [];
  const rows = t.rows
    .map((row, ri) => {
      row.parts.forEach((id, i) => (keyOf[id] = `r${ri}c${i}`));
      if (row.under && !(row.parts[0] && edgeBetween(row.under, row.parts[0]))) wires.push({ from: row.under, toKey: `r${ri}c0`, tree: true });
      const line = row.parts
        .map((id, i) => {
          let gap = "";
          if (i) {
            const e = edgeBetween(row.parts[i - 1], id);
            if (e) {
              const dir = pointing(e, row.parts[i - 1], id);
              gap = `<span class="gap" data-label="${esc(e.says)}" data-dir="${dir}" style="flex-basis:${Math.max(44, e.says.length * 6.5 + 16)}px"></span>`;
            } else gap = '<span class="space"></span>';
          }
          return gap + partEl(id, { key: `r${ri}c${i}` });
        })
        .join("");
      return `<div class="row" ${row.under ? `data-parent="${row.under}"` : ""}>${row.name ? `<span class="rowname">${esc(row.name)}</span>` : ""}<div class="line">${line}</div></div>`;
    })
    .join("");
  // The edges the tier draws past its gaps: between rows, to the frame,
  // and along the frame.
  for (const e of edges()) {
    const aIn = inRows.includes(e.from), bIn = inRows.includes(e.to);
    const aFrame = frameIds.includes(e.from), bFrame = frameIds.includes(e.to);
    if (aIn && bIn) {
      const [ra, ia] = place(t, e.from), [rb, ib] = place(t, e.to);
      if (ra === rb && Math.abs(ia - ib) === 1) continue; // drawn in the gap
      wires.push({ from: e.from, to: e.to, inner: true, label: e.says, dir: e.dir || "to" });
    } else if ((aIn && bFrame) || (aFrame && bIn)) {
      const inner = aIn ? e.from : e.to, outer = aIn ? e.to : e.from;
      const [ri, ci] = place(t, inner);
      const last = ci === t.rows[ri].parts.length - 1;
      const side = (frame.right || []).includes(outer) ? "right" : "left";
      wires.push({ from: inner, outer, label: e.says, dir: pointing(e, inner, outer), under: side === "right" && !last, side });
    } else if (aFrame && bFrame) {
      const side = [frame.left || [], frame.right || []].find((ids) => ids.includes(e.from) && ids.includes(e.to));
      if (side && Math.abs(side.indexOf(e.from) - side.indexOf(e.to)) === 1) wires.push({ outerFrom: e.from, outerTo: e.to, label: e.says, dir: e.dir || "to" });
    }
  }
  const side = (ids) => (ids || []).map((id) => partEl(id, { outer: true })).join("");
  const opener = openerOf(tierId);
  const head = opener ? parts()[opener] : null;
  const noframe = !frameIds.length;
  const body = `<div class="drill ${noframe ? "noframe" : ""}" data-wires="${esc(JSON.stringify(wires))}" data-keys="${esc(JSON.stringify(keyOf))}">
    ${noframe ? "" : `<div class="side left">${side(frame.left)}</div>`}
    <div class="focus">${head ? `<div class="focushead"><span class="name">${esc(head.name)}</span>${head.each ? `<span class="each">${esc(head.each)}</span>` : ""}<span class="open">open</span></div>` : ""}${rows}</div>
    ${noframe ? "" : `<div class="side right">${side(frame.right)}</div>`}
  </div>`;
  const chain = [...above(tierId), tierId].map((id) => ({ text: tier(id).name, tier: id }));
  app.innerHTML = `${crumbs(chain)}${sourceLine()}${t.says ? `<div class="says">${esc(t.says)}</div>` : ""}${body}
    <div class="foot"><span>a card opens the tier inside it, or the code it holds</span><span>·</span><span>orange is what changed, as Changes shows it</span></div>`;
  wired();
}

/// The row and the place in it of a box in a tier.
function place(t, id) {
  for (let ri = 0; ri < t.rows.length; ri++) {
    const ci = t.rows[ri].parts.indexOf(id);
    if (ci >= 0) return [ri, ci];
  }
  return [-1, -1];
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

/// A curve from card `A` to card `B`, side to side, or down when `B` is
/// under `A`; and where its label goes.
function curve(A, B) {
  const vertical = B.t >= A.t + A.h - 4 && Math.min(A.l + A.w, B.l + B.w) - Math.max(A.l, B.l) > 40;
  if (vertical) {
    const x = Math.max(A.l, B.l) + 44;
    const y1 = A.t + A.h + (A.many ? 8 : 0) + 2, y2 = B.t - 2;
    return { d: `M${x},${y1} L${x},${y2}`, tx: x + 8, ty: (y1 + y2) / 2 + 4, anchor: "start" };
  }
  const leftward = B.l + B.w / 2 < A.l + A.w / 2;
  const ax = leftward ? A.l : A.l + A.w, bx = leftward ? B.l + B.w : B.l;
  const ay = A.t + Math.min(A.h, 72) / 2 + (A.h > 72 ? 10 : 0), by = B.t + Math.min(B.h, 72) / 2 + (B.h > 72 ? 10 : 0);
  const dx = Math.max(24, Math.abs(bx - ax) * 0.45) * (leftward ? -1 : 1);
  return { d: `M${ax},${ay} C${ax + dx},${ay} ${bx - dx},${by} ${bx},${by}`, tx: (ax + bx) / 2, ty: (ay + by) / 2 - 6, anchor: "middle" };
}

/// The lines of a tier past the gaps: guides from a card to the rows under
/// it, lines between rows, to the frame, and along the frame.
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
  const inner = (id) => d.querySelector(`.focus [data-k="${keys[id]}"]`);
  const outer = (id) => d.querySelector(`.side [data-outer="${CSS.escape(id)}"]`);
  const out = [], texts = [];
  const underCount = {};
  const cards = [...d.querySelectorAll(".focus .part")].map(rel);
  for (let w of JSON.parse(d.dataset.wires || "[]")) {
    let a, b;
    if (w.tree) [a, b] = [inner(w.from), d.querySelector(`[data-k="${w.toKey}"]`)];
    else if (w.inner) [a, b] = [inner(w.from), inner(w.to)];
    else if (w.outer) [a, b] = [inner(w.from), outer(w.outer)];
    else [a, b] = [outer(w.outerFrom), outer(w.outerTo)];
    if (!a || !b) continue;
    const A = rel(a), B = rel(b);
    let { d: path, tx, ty, anchor } = curve(A, B);
    let end = w.dir === "to" || w.dir === "both", start = w.dir === "from" || w.dir === "both";
    // A line to the frame that would cross cards of the rows below goes
    // under its own card first, across between the rows, and on.
    if (w.outer && w.side === "right" && !w.under) {
      const right = A.l + A.w, ay = A.t + A.h / 2, by = B.t + B.h / 2;
      const blocked = cards.some((c) => c !== A && c.l < B.l && c.l + c.w > right + 4 && c.t < Math.max(ay, by) && c.t + c.h > Math.min(ay, by) && c.t > A.t);
      if (blocked) w = { ...w, under: true };
    }
    if (w.tree) {
      const x = A.l + 12, y1 = A.t + A.h + (A.many ? 8 : 0), y2 = B.t + B.h / 2;
      path = `M${x},${y1} L${x},${y2 - 8} Q${x},${y2} ${x + 8},${y2} L${B.l},${y2}`;
      tx = null;
      end = true;
      start = false;
    } else if (w.inner && Math.abs(A.t - B.t) < 8) {
      // Along one row, past the cards between: down, across under them, and up.
      const k = (underCount[A.t] = (underCount[A.t] || 0) + 1);
      const ax = A.l + A.w * 0.6, bx = B.l + B.w * 0.4, ay = A.t + A.h + (A.many ? 8 : 0), yy = Math.max(A.t + A.h, B.t + B.h) + 10 + k * 14;
      path = `M${ax},${ay} L${ax},${yy - 8} Q${ax},${yy} ${ax + 8},${yy} L${bx - 8},${yy} Q${bx},${yy} ${bx},${yy - 8} L${bx},${B.t + B.h + (B.many ? 8 : 0)}`;
      tx = (ax + bx) / 2;
      ty = yy - 5;
      anchor = "middle";
    } else if (w.under) {
      // From a card that is not the last of its row, under the rest of
      // the row and on to the frame.
      const k = (underCount[w.from] = (underCount[w.from] || 0) + 1) - 1;
      const ax = A.l + A.w * 0.6, ay = A.t + A.h + (A.many ? 8 : 0), yy = ay + 16 + k * 14, bx = B.l, by = B.t + B.h / 2;
      path = `M${ax},${ay} L${ax},${yy - 10} Q${ax},${yy} ${ax + 10},${yy} L${bx - 34},${yy} C${bx - 12},${yy} ${bx - 12},${by} ${bx},${by}`;
      tx = ax + 16;
      ty = yy - 5;
      anchor = "start";
    }
    out.push(`<path d="${path}"${end ? ' marker-end="url(#arr)"' : ""}${start ? ' marker-start="url(#arr)"' : ""}/>`);
    if (w.label && tx !== null) texts.push(`<text x="${tx}" y="${ty}" text-anchor="${anchor}">${esc(w.label)}</text>`);
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
