// The map, drawn a level at a time: the boxes inside the node opened, and
// around them the nodes of the levels above it that their lines reach,
// those the open box's lines do not tie to faded. Opening a box grows it
// into the edge of the level inside it, and then the level's boxes come
// out of it; going up plays that back. A level a person placed has its
// boxes where the map's vocabulary put them; the level inside a box that
// owns code, its types, laid out by the generator, grows out of where the
// box sits in the level above.
//
// The lines between boxes whose code meets are as thick as it meets
// often; a box's two heaviest are always drawn, the rest, weak, only
// while one of its ends is chosen or pointed at from a list.
//
// A function chosen splits the level: the cards of what it relates to
// leave their places for an area above the box, laid out as the calls
// flow, from the left to the right, and the rest stays under it, faint.

import { ICON } from "./icons.js";

// Sizes: an outside node's card, a part's or a type's card, whatever
// its name is long, the gap between cards in
// a column of the top, the top's center, how far the top's columns stand
// from it, and how far the outside nodes stand from a level's edge.
const PW = 248;
const PH = 60;
const IW = 216;
const IH = 56;
const GAP = 18;
const CW = 260;
const CH = 132;
const GX = 360;
const OUT = 150;
// How faint what the open box's lines do not reach is drawn.
const FAR = 0.42;
const SIDE = { left: "l", right: "r", top: "t", bottom: "b" };
// The area a function chosen is laid out in: how far its columns and the
// cards of one stand apart, the room over a column for what it is, the
// gap between the rows it wraps into, the space around it inside its
// edge and out, how far the lines that pass a column stand apart, how
// faint the rest of the level is under it, and its lines, and those of
// them that end at a card of the area, how small it may be drawn, and how
// many seconds the cards take to go there.
const AREA = { across: 72, down: 14, label: 24, wrap: 30, pad: 26, out: 44, lane: 6, rest: 0.36, lines: 0.4, ends: 0.2, readable: 0.62, takes: 0.4 };
// The columns of the area, from the left, each with what is written over
// it: who calls the function, its own type, what it calls, the types
// whose fields it uses, and what those fields point to; at the far sides,
// what stands in the level for code it does not draw.
const COLUMNS = [
  ["from", "Called from elsewhere"],
  ["caller", "Called from"],
  ["own", ""],
  ["call", "Calls"],
  ["field", "Uses fields of"],
  ["target", "Fields point to"],
  ["to", "Reaches elsewhere"],
];
// Which column a card that is in several ways related goes to.
const FIRST = ["call", "caller", "field", "target"];

// What is drawn, and how: the map's nodes, its top and levels as the
// engine reads them, and where the page is in it.
export const view = {
  /// The levels opened, from the top: "root", then the node each opens.
  stack: ["root"],
  /// The node chosen, whose lines light up.
  sel: null,
  /// The nodes pointed at from a list, which light up as the chosen one.
  mark: new Set(),
  /// The function chosen, as `focus` was given it, and the `level` it
  /// was chosen in.
  focus: null,
  /// Drift drawn as lines, between two nodes each, while the drift is
  /// shown, and the one pointed at, as "a|b".
  ghost: [],
  ghostOn: null,
  /// How many items of drift each node has, open.
  drift: {},
  /// What changed in the code of each node.
  changed: {},
};

let nodes = {};
let top = { center: "", left: [], right: [], wires: [] };
let levels = {};
let parents = {};
let weights = {};
let heaviest = 10;

/// Reads `map`, the map as generated, for drawing, with what `changed`
/// in its nodes' code.
export function setMap(map, changed) {
  nodes = map.nodes;
  const wire = (line) => [
    line.from,
    line.to,
    line.both ? 1 : 0,
    line.sides ? line.sides.map((side) => SIDE[side]) : undefined,
    line.weak ? 1 : 0,
  ];
  top = { center: map.top.center, left: map.top.left, right: map.top.right, wires: map.top.lines.map(wire) };
  levels = {};
  parents = {};
  for (const [id, level] of Object.entries(map.levels)) {
    levels[id] = {
      box: level.box,
      size: level.size,
      outer: Object.entries(level.outer ?? {}).map(([node, [side, along]]) => [node, SIDE[side], along]),
      at: level.at ?? {},
      frames: level.frames ?? {},
      parts: Object.entries(level.parts).map(([node, [x, y]]) => [node, x, y]),
      wires: level.lines.map(wire),
    };
    for (const part of Object.keys(level.parts)) parents[part] ??= id;
  }
  weights = Object.fromEntries(map.edges.map((edge) => [pairKey(edge.a, edge.b), edge.count]));
  heaviest = Math.max(10, ...map.edges.map((edge) => edge.count));
  view.changed = changed ?? {};
}

const pairKey = (a, b) => (a < b ? `${a}|${b}` : `${b}|${a}`);
export const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const lerp = (a, b, t) => a + (b - a) * t;
const ease = (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
const clamp = (t) => Math.max(0, Math.min(1, t));
const R = (x, y, w, h) => ({ x, y, w, h });
const lerpR = (a, b, t) => R(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.w, b.w, t), lerp(a.h, b.h, t));
const textWidth = (text) => [...text].reduce((w, c) => w + (/[\x20-\x7e]/.test(c) ? 0.6 : 1) * 14.5, 0);

/// The node `id`, as drawn.
export const node = (id) => nodes[id] ?? { name: id, kind: "part" };
const inside = (id) => node(id).kind !== "outside";
const iconOf = (id) => node(id).icon ?? (node(id).kind === "type" ? "braces" : "box");
/// The node of the project, in the middle of the top.
export const center = () => top.center;
export const canOpen = (id) => Boolean(levels[id]);
/// The node that opens the level `id` is in, or "root" at the top.
export const parentOf = (id) => parents[id] ?? "root";
/// The nodes to open, from the top down, to come to the level inside
/// `f`, `f` last.
export function chain(f) {
  const way = [];
  for (let at = f; at !== "root" && !way.includes(at) && way.length < 32; at = parentOf(at)) way.unshift(at);
  return way;
}

const size = (id) => (inside(id) ? [IW, IH] : [PW, PH]);

// The card that stands for the node `id` among those at `r` in the level
// inside `f`: its own, or that of the nearest box it is inside of which
// is drawn closed there.
const standsFor = (id, r, f) =>
  chain(id)
    .reverse()
    .find((at) => at !== f && r[at]);
// The cards lit as the chosen one is: those of the nodes pointed at.
const marked = (r, f) => new Set([...view.mark].map((id) => standsFor(id, r, f)).filter(Boolean));

// Opening a box replaces the lines to it with those of the level inside
// it; the other lines stay. A weak line shows only while one of its ends
// is chosen or pointed at.
const showWeak = (mark) => (e) => !e[4] || [e[0], e[1]].some((n) => n === view.sel || mark.has(n));
// A level a person placed shows, of the levels above, only what it
// places around its box, so of their lines only those between what
// `shown` there, by id.
function wiresOf(f, shown, mark) {
  let wires = top.wires;
  for (const g of chain(f)) wires = wires.filter(([x, y]) => x !== g && y !== g).concat(levels[g].wires);
  return wires.filter(([x, y]) => x in shown && y in shown).filter(showWeak(mark));
}
// A level laid out, with its lines and the cards lit in it, and the
// nodes it places at the `edge` of its box.
function laid(f, r, box, frames, parts, edge = []) {
  const mark = marked(r, f);
  return { id: f, r, box, frames, mark, wires: wiresOf(f, { ...r, ...frames }, mark), parts, edge };
}

/// The level inside `f`, or the top for "root", laid out: where each node
/// is, the box opened, the edges of those above it, its lines and parts.
export function geo(f) {
  const r = {};
  if (f === "root") {
    const column = (list, x) => {
      const h = list.length * PH + (list.length - 1) * GAP;
      list.forEach((id, i) => (r[id] = R(x, -h / 2 + i * (PH + GAP), PW, PH)));
    };
    column(top.left, -GX - PW);
    column(top.right, GX);
    r[top.center] = R(-CW / 2, -CH / 2, CW, CH);
    const mark = marked(r, f);
    return { id: f, r, box: null, frames: {}, mark, wires: top.wires.filter(showWeak(mark)), parts: [] };
  }
  const d = levels[f];
  // A level gone as the map was read again draws as the top until the
  // page goes somewhere it still has.
  if (!d) return geo("root");
  if (d.size) {
    // Grows out of where the box sits in the level above, which moves
    // aside to make room.
    const above = geo(parentOf(f));
    const o0 = above.r[f];
    const [W, H] = d.size;
    const gx = (W - o0.w) / 2;
    const gy = (H - o0.h) / 2;
    const box = R(o0.x - gx, o0.y - gy, W, H);
    const moved = (q) =>
      R(
        q.x + (q.x >= o0.x + o0.w ? gx : q.x + q.w <= o0.x ? -gx : 0),
        q.y + (q.y >= o0.y + o0.h ? gy : q.y + q.h <= o0.y ? -gy : 0),
        q.w,
        q.h,
      );
    const grown = (q) => R(q.x - gx, q.y - gy, q.w + 2 * gx, q.h + 2 * gy);
    for (const id in above.r) if (id !== f && !(id in above.frames) && id !== parentOf(f)) r[id] = moved(above.r[id]);
    const frames = {};
    for (const k in above.frames) frames[k] = grown(above.frames[k]);
    frames[parentOf(f)] = grown(above.box);
    for (const [id, x, y] of d.parts) r[id] = R(box.x + x, box.y + y, ...size(id));
    r[f] = box;
    return laid(
      f,
      r,
      box,
      frames,
      d.parts.map((p) => p[0]),
    );
  }
  const [bx, by, bw, bh] = d.box;
  const box = R(bx, by, bw, bh);
  for (const [id, [x, y]] of Object.entries(d.at)) r[id] = R(x, y, ...size(id));
  for (const [id, side, along] of d.outer) {
    const [w, h] = size(id);
    r[id] =
      side === "l"
        ? R(bx - OUT - w, along, w, h)
        : side === "r"
          ? R(bx + bw + OUT, along, w, h)
          : side === "t"
            ? R(along, by - OUT * 0.6 - h, w, h)
            : R(along, by + bh + OUT * 0.6, w, h);
  }
  for (const [id, x, y] of d.parts) r[id] = R(x, y, ...size(id));
  r[f] = box;
  const frames = {};
  for (const [k, v] of Object.entries(d.frames)) frames[k] = R(...v);
  return laid(
    f,
    r,
    box,
    frames,
    d.parts.map((p) => p[0]),
    d.outer.map((o) => o[0]),
  );
}

const outline = (rects) => ({
  x0: Math.min(...rects.map((r) => r.x)) - 30,
  x1: Math.max(...rects.map((r) => r.x + r.w)) + 30,
  y0: Math.min(...rects.map((r) => r.y)) - 40,
  y1: Math.max(...rects.map((r) => r.y + r.h)) + 30,
});

// The parts of a level laid out, and the box opened, whose edge the
// lines it had closed that reach none of its parts go to.
const within = (g) => new Set(g.box ? [...g.parts, g.id] : g.parts);

// All of it, when it fits readably; when it does not, the open box and
// what its lines reach, no farther from the box than a card's width or
// what the level places at its edge: a level that keeps all of the level
// above around its box has lines to the far ends of it, and fitting
// those leaves the box small. The rest is drawn past the edges still.
function bounds(g) {
  const all = outline([...Object.values(g.r), ...Object.values(g.frames)]);
  const k = Math.min(cv.clientWidth / (all.x1 - all.x0), cv.clientHeight / (all.y1 - all.y0));
  if (k >= 0.55 || !g.box) return all;
  const parts = within(g);
  const near = new Set();
  g.wires.forEach(([x, y]) => {
    if (parts.has(x)) near.add(y);
    if (parts.has(y)) near.add(x);
  });
  const reached = outline([g.box, ...[...near].filter((id) => g.r[id]).map((id) => g.r[id])]);
  const grown = R(g.box.x - IW, g.box.y - IW, g.box.w + 2 * IW, g.box.h + 2 * IW);
  const close = outline([grown, ...g.edge.map((id) => g.r[id])]);
  return {
    x0: Math.max(reached.x0, close.x0),
    x1: Math.min(reached.x1, close.x1),
    y0: Math.max(reached.y0, close.y0),
    y1: Math.min(reached.y1, close.y1),
  };
}

const middle = (r) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });
function side(r, o) {
  const a = middle(r);
  const b = middle(o);
  const dx = (b.x - a.x) / ((r.w + o.w) / 2);
  const dy = (b.y - a.y) / ((r.h + o.h) / 2);
  if (Math.abs(dx) >= Math.abs(dy)) return dx > 0 ? "r" : "l";
  return dy > 0 ? "b" : "t";
}
// Where each line leaves and meets its ends: the side toward the other
// end, unless the line says, the lines on one side spread along it in the
// order of where they go.
function anchors(list, rect) {
  const ends = {};
  const out = list.map(() => ({}));
  list.forEach((e, i) => {
    for (const [k, me, other] of [
      ["a", e[0], e[1]],
      ["b", e[1], e[0]],
    ]) {
      const sd = e[3]?.[k === "a" ? 0 : 1] ?? side(rect(me), rect(other));
      (ends[me + sd] ??= []).push({ i, k, sd, me, other });
    }
  });
  for (const arr of Object.values(ends)) {
    const { sd, me } = arr[0];
    const r = rect(me);
    const across = sd === "l" || sd === "r";
    arr.sort((u, v) =>
      across ? middle(rect(u.other)).y - middle(rect(v.other)).y : middle(rect(u.other)).x - middle(rect(v.other)).x,
    );
    arr.forEach((u, j) => {
      const f = (j + 1) / (arr.length + 1);
      const n = { l: [-1, 0], r: [1, 0], t: [0, -1], b: [0, 1] }[sd];
      out[u.i][u.k] = {
        x: across ? r.x + (sd === "r" ? r.w + 4 : -4) : r.x + r.w * f,
        y: across ? r.y + r.h * f : r.y + (sd === "b" ? r.h + 4 : -4),
        n,
      };
    });
  }
  return out;
}
// A line from `a` to `b` by the `way` past the columns between them.
function pathBy(a, b, way) {
  const k = (Math.sign(way.to - way.from) * AREA.across) / 2;
  return `M${a.x} ${a.y}C${a.x + k} ${a.y},${way.from - k} ${way.y},${way.from} ${way.y}L${way.to} ${way.y}C${way.to + k} ${way.y},${b.x - k} ${b.y},${b.x} ${b.y}`;
}
function path(a, b) {
  const d = Math.hypot(b.x - a.x, b.y - a.y);
  const k = Math.min(130, Math.max(36, d * 0.42));
  return `M${a.x} ${a.y}C${a.x + a.n[0] * k} ${a.y + a.n[1] * k},${b.x + b.n[0] * k} ${b.y + b.n[1] * k},${b.x} ${b.y}`;
}

// A card's text is measured as the page draws it, to fit the card.
const pen = document.createElement("canvas").getContext("2d");
const fits = new Map();
// `text` in the `font`, on `most` lines of `width` at most: broken
// between words where it can be, and cut short, with an ellipsis, where
// it is longer than that.
function wrapped(text, font, width, most) {
  const key = `${font}|${width}|${most}|${text}`;
  if (fits.has(key)) return fits.get(key);
  pen.font = `${font} ${getComputedStyle(W).fontFamily}`;
  const wide = (part) => pen.measureText(part).width;
  const lines = [];
  let rest = String(text ?? "").trim();
  while (rest && lines.length < most) {
    if (wide(rest) <= width) {
      lines.push(rest);
      break;
    }
    const last = lines.length === most - 1;
    const room = last ? width - wide("…") : width;
    let end = rest.length - 1;
    while (end > 1 && wide(rest.slice(0, end).trimEnd()) > room) end--;
    if (last) {
      lines.push(`${rest.slice(0, end).trimEnd()}…`);
      break;
    }
    const space = rest.slice(0, end + 1).lastIndexOf(" ");
    const cut = space > 0 ? space : end;
    lines.push(rest.slice(0, cut));
    rest = rest.slice(cut).trim();
  }
  fits.set(key, lines);
  return lines;
}
const NAME = "600 14.5px";
const CODE = "400 11.5px";
// A card's name over what it is in the code, on three lines at most
// between them, in the `room` right of `x`, around the middle of `h`;
// where some was cut, all of it shows as the card is pointed at.
function label(p, x, room, h) {
  const name = wrapped(p.name, NAME, room, 2);
  const code = p.code ? wrapped(p.code, CODE, room, 3 - name.length) : [];
  const from = (h - 17 * name.length - 14 * code.length - (code.length ? 2 : 0)) / 2;
  const cut = [...name, ...code].some((line) => line.endsWith("…"));
  return (
    (cut ? `<title>${esc(p.name)}${p.code ? `\n${esc(p.code)}` : ""}</title>` : "") +
    name
      .map(
        (line, i) =>
          `<text x="${x}" y="${from + 17 * i + 13.5}" font-size="14.5" font-weight="600" fill="var(--foreground)">${esc(line)}</text>`,
      )
      .join("") +
    code
      .map(
        (line, i) =>
          `<text x="${x}" y="${from + 17 * name.length + 14 * i + 12.5}" font-size="11.5" fill="var(--dim)">${esc(line)}</text>`,
      )
      .join("")
  );
}

// What changed in a node's code, as a tab over its top left edge.
function changedTab(id) {
  const changed = view.changed[id];
  if (!changed) return "";
  const text = `+${changed.added} −${changed.removed}`;
  const w = Math.ceil(textWidth(text) * 0.72) + 14;
  return `<g class="ch" transform="translate(12,-9)"><title>${changed.files} file${changed.files === 1 ? "" : "s"} changed</title><rect width="${w}" height="17" rx="8.5" fill="var(--surface)" stroke="var(--border-strong)"/><text x="7" y="12.5" font-size="10.5" font-weight="600"><tspan fill="var(--added-word)">+${changed.added}</tspan><tspan dx="4" fill="var(--removed-word)">−${changed.removed}</tspan></text></g>`;
}
const driftBadge = (id, w) =>
  view.drift[id] && !anim
    ? `<g class="dn" transform="translate(${w - 2},2)"><circle r="9" fill="var(--drift)"/><text y="4" text-anchor="middle" font-size="10.5" font-weight="700" fill="#1b1408">${view.drift[id]}</text></g>`
    : "";

function card(id, r, op = 1, appearing = 0) {
  const p = node(id);
  const inn = inside(id);
  const w = r.w;
  const h = r.h;
  const sc = appearing ? lerp(0.85, 1, op) : 1;
  const on = id === view.sel || lit.has(id);
  const tf = `translate(${r.x + w / 2},${r.y + h / 2}) scale(${sc}) translate(${-w / 2},${-h / 2})`;
  const stack = p.many
    ? [8, 4]
        .map(
          (d) =>
            `<rect x="${d}" y="${-d}" width="${w}" height="${h}" rx="10" fill="var(--surface)" stroke="${inn ? "rgba(111,211,197,.3)" : "var(--border)"}"/>`,
        )
        .join("")
    : "";
  const open =
    canOpen(id) && !anim
      ? `<g class="open" data-open="${esc(id)}" transform="translate(${w - 26},6)"><title>Open</title><rect width="20" height="20" rx="5" fill="transparent"/><g transform="translate(3,3) scale(.6)" fill="none" stroke="var(--dim)" stroke-width="2" stroke-linecap="round">${ICON.drill}</g></g>`
      : "";
  const text = label(p, 58, w - 58 - (canOpen(id) ? 30 : 12), h);
  return `<g class="obj${inn ? " in" : ""}" data-id="${esc(id)}" opacity="${op}" transform="${tf}">${stack}<rect class="body" width="${w}" height="${h}" rx="10" fill="var(--raised)" stroke="${on ? "var(--accent)" : inn ? "rgba(111,211,197,.45)" : "var(--border-strong)"}" stroke-width="${on ? 1.8 : 1}"/>
  <rect x="11" y="${(h - 36) / 2}" width="36" height="36" rx="8" fill="${inn ? "var(--accent-ground)" : "var(--button)"}"/>
  <g transform="translate(17,${(h - 36) / 2 + 6})" fill="none" stroke="${inn ? "var(--accent)" : "var(--dim)"}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${ICON[iconOf(id)] ?? ICON.box}</g>
  ${text}${open}${anim ? "" : changedTab(id)}${driftBadge(id, w)}</g>`;
}

// The node of the project, in the middle of the top.
function centerCard(id, r, op) {
  const on = view.sel === id;
  const name = wrapped(node(id).name, "700 22px", r.w - 64, 2);
  return `<g class="obj" data-id="${esc(id)}" opacity="${op}"><rect class="body" x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}" rx="16" fill="var(--accent-ground)" stroke="var(--accent)" stroke-width="${on ? 2.4 : 1.6}"/>
  ${name.map((line, i) => `<text x="${r.x + r.w / 2}" y="${r.y + r.h / 2 + 8 + 28 * (i - (name.length - 1) / 2)}" font-size="22" font-weight="700" fill="var(--foreground)" text-anchor="middle">${esc(line)}</text>`).join("")}
  ${canOpen(id) && !anim ? `<g class="open" data-open="${esc(id)}" transform="translate(${r.x + r.w - 30},${r.y + 8})"><title>Open</title><rect width="22" height="22" rx="5" fill="transparent"/><g transform="translate(3,3) scale(.7)" fill="none" stroke="var(--dim)" stroke-width="2" stroke-linecap="round">${ICON.drill}</g></g>` : ""}
  ${anim ? "" : `<g transform="translate(${r.x},${r.y})">${changedTab(id)}${driftBadge(id, r.w)}</g>`}</g>`;
}

// The edge of a box opened, with its name, around the level inside it.
function boundary(id, r, op) {
  const n = node(id);
  return `<g opacity="${op}"><rect x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}" rx="20" fill="rgba(21,44,42,.28)" stroke="var(--accent)" stroke-width="1.6" stroke-dasharray="7 5"/>
  <text x="${r.x + 18}" y="${r.y + 28}" font-size="14" font-weight="700" fill="var(--accent)">${esc(n.name)}${n.code ? `<tspan class="mono" dx="10" font-size="11.5" font-weight="400" fill="var(--dim)">${esc(n.code)}</tspan>` : ""}</text></g>`;
}

const cv = document.getElementById("cv");
const W = document.getElementById("w");

// What the panels leave of the drawing, as `focus` was told: its width
// and height, and where it starts from the left.
let clear = { left: 0, right: 0 };
function room() {
  const beside = clear.left || clear.right;
  return {
    left: clear.left,
    w: Math.max(1, cv.clientWidth - clear.left - clear.right),
    h: Math.max(1, beside ? cv.clientHeight : innerHeight * 0.5 - cv.getBoundingClientRect().top),
  };
}

// The level `g` as the function chosen arranges it: where each card and
// each edge is, and, when the function relates to cards of the level,
// those `taken` to its area, the area's `edge`, what is written in it,
// the places the cards left, and the lines between them, by their ends:
// each from a card to a card, whether it has a head at both ends, the
// sides it leaves and meets them at, and the way it takes past the
// columns between them, if there are some.
// With no function chosen, or one that relates to nothing drawn here,
// the level as it is.
function arrangement(g) {
  const still = { id: g.id, r: g.r, frames: g.frames, taken: null, lines: null };
  const chosen = view.focus;
  if (!chosen || chosen.level !== g.id || !g.box) return still;
  const cardOf = (id) => standsFor(id, g.r, g.id);
  const own = cardOf(chosen.own);
  if (!own) return still;
  const parts = new Set(g.parts);
  const column = new Map([[own, "own"]]);
  const lines = new Map();
  for (const role of FIRST)
    for (const [from, to, as] of chosen.links) {
      const [x, y] = [cardOf(from), cardOf(to)];
      if (as !== role || !x || !y || x === y) continue;
      // Called both ways, the line has a head at both ends.
      const key = pairKey(x, y);
      const had = lines.get(key);
      lines.set(key, had ? [had[0], had[1], had[2] || had[0] !== x] : [x, y, false]);
      const other = role === "caller" ? x : y;
      if (!column.has(other)) column.set(other, parts.has(other) ? role : role === "caller" ? "from" : "to");
    }
  if (column.size < 2) return still;

  // Each column as stacks of a few cards, and the stacks in rows, as many
  // as are wide enough to read in what the panels leave.
  const { w, h } = room();
  const most = Math.max(2, Math.min(5, Math.floor((h * 0.45) / AREA.readable / (IH + AREA.down))));
  const stacks = [];
  for (const [key, says] of COLUMNS) {
    const ids = [...column].filter(([, at]) => at === key).map(([id]) => id);
    for (let i = 0; i < ids.length; i += most) {
      const of = ids.slice(i, i + most);
      stacks.push({
        says: key === "own" ? chosen.name : says,
        more: i > 0,
        own: key === "own",
        ids: of,
        w: Math.max(...of.map((id) => g.r[id].w)),
        h: of.reduce((sum, id) => sum + g.r[id].h, 0) + (of.length - 1) * AREA.down,
      });
    }
  }
  const widest = w / AREA.readable - 2 * AREA.pad;
  const rows = [];
  for (const stack of stacks) {
    const row = rows.at(-1);
    if (row && row.w + AREA.across + stack.w <= widest) {
      row.stacks.push(stack);
      row.w += AREA.across + stack.w;
      row.h = Math.max(row.h, stack.h);
    } else rows.push({ stacks: [stack], w: stack.w, h: stack.h });
  }
  const wide = Math.max(...rows.map((row) => row.w)) + 2 * AREA.pad;
  const tall = rows.reduce((sum, row) => sum + AREA.label + row.h, 0) + (rows.length - 1) * AREA.wrap + 2 * AREA.pad;

  // The area stands over the box and what is beside it; what is above
  // them moves up to make room, and the edges around them grow.
  const above = (q) => q.y + q.h <= g.box.y;
  const rest = Object.keys(g.r).filter((id) => !column.has(id));
  const floor = Math.min(...rest.filter((id) => !above(g.r[id])).map((id) => g.r[id].y));
  const edge = R(middle(g.box).x - wide / 2, floor - AREA.out - tall, wide, tall);
  const up = g.box.y - edge.y + AREA.out;
  const moved = (q) => (above(q) ? R(q.x, q.y - up, q.w, q.h) : q);
  const r = {};
  for (const id of rest) r[id] = moved(g.r[id]);
  const frames = {};
  for (const [id, q] of Object.entries(g.frames)) frames[id] = R(q.x, q.y - up, q.w, q.h + up);
  const left = [...column.keys()].map((id) => moved(g.r[id]));
  const labels = [];
  const where = new Map();
  let y = edge.y + AREA.pad;
  for (const row of rows) {
    let x = edge.x + AREA.pad;
    row.stacks.forEach((stack, n) => {
      let at = y + AREA.label + (row.h - stack.h) / 2;
      Object.assign(stack, { x, top: at, bottom: at + stack.h });
      // A column says what it is once in a row.
      if (!stack.more || !n) labels.push({ x, y: at - 9, says: stack.says, own: stack.own });
      for (const id of stack.ids) {
        r[id] = R(x + (stack.w - g.r[id].w) / 2, at, g.r[id].w, g.r[id].h);
        where.set(id, { row, n });
        at += g.r[id].h + AREA.down;
      }
      x += stack.w + AREA.across;
    });
    row.middle = y + AREA.label + row.h / 2;
    y += AREA.label + row.h + AREA.wrap;
  }
  // In a row, a line goes from side to side, as the calls flow; past the
  // columns between its ends, over them or under, whichever its far end
  // is nearer, the lines of one card together and those of another apart.
  // To another row, it comes in from the side, not through the cards
  // stacked over its end.
  const lanes = new Map();
  for (const line of lines.values()) {
    const [a, b] = [where.get(line[0]), where.get(line[1])];
    if (a.row !== b.row)
      line[3] = [rows.indexOf(a.row) < rows.indexOf(b.row) ? "b" : "t", middle(r[line[0]]).x <= middle(r[line[1]]).x ? "l" : "r"];
    if (a.row !== b.row || a.n === b.n) continue;
    const on = a.n < b.n;
    line[3] = on ? ["r", "l"] : ["l", "r"];
    const passed = a.row.stacks.slice(Math.min(a.n, b.n) + 1, Math.max(a.n, b.n));
    if (!passed.length) continue;
    const over = middle(r[line[1]]).y < a.row.middle;
    const key = `${rows.indexOf(a.row)}|${over}`;
    const taken = lanes.get(key) ?? lanes.set(key, new Map()).get(key);
    const lane = taken.get(line[0]) ?? taken.set(line[0], taken.size).get(line[0]);
    const [x0, x1] = [passed[0].x, passed.at(-1).x + passed.at(-1).w];
    line[4] = {
      y: over
        ? Math.min(...passed.map((stack) => stack.top)) - AREA.label - 4 - AREA.lane * lane
        : Math.max(...passed.map((stack) => stack.bottom)) + 10 + AREA.lane * lane,
      from: on ? x0 : x1,
      to: on ? x1 : x0,
    };
  }
  return { id: g.id, r, frames, taken: new Set(column.keys()), lines, edge, labels, left, box: g.box };
}

// The area of an arrangement, under its cards: its edge, what is written
// over its columns, and the places its cards left.
function areaUnder(p, op) {
  if (!p.taken || op <= 0) return "";
  const { edge } = p;
  return `<g opacity="${op}"><rect x="${edge.x}" y="${edge.y}" width="${edge.w}" height="${edge.h}" rx="16" fill="var(--surface)" fill-opacity=".55" stroke="var(--border-strong)"/>${p.left
    .map(
      (q) =>
        `<rect x="${q.x}" y="${q.y}" width="${q.w}" height="${q.h}" rx="10" fill="none" stroke="var(--border-strong)" stroke-dasharray="4 5" opacity=".6"/>`,
    )
    .join("")}${p.labels
    .map((l) =>
      l.own
        ? `<text class="mono" x="${l.x}" y="${l.y}" font-size="12.5" font-weight="600" fill="var(--accent)">${esc(l.says)}</text>`
        : `<text x="${l.x}" y="${l.y}" font-size="11.5" fill="var(--dim)">${esc(l.says)}</text>`,
    )
    .join("")}</g>`;
}

// The cards on their way to where the function chosen has them, from
// where they were as it was chosen, or let go.
let shift = null;
// What the drawing showed before a function was chosen, to show again.
let home = null;

// The level `g` as it is drawn now: as the function chosen arranges it,
// or part of the way there from how it was.
function arranged(g) {
  const to = arrangement(g);
  const from = shift && shift.from.id === g.id ? shift.from : null;
  const e = from ? ease(shift.t) : 1;
  const mix = (a, b) => Object.fromEntries(Object.entries(b).map(([id, q]) => [id, a[id] && e < 1 ? lerpR(a[id], q, e) : q]));
  const r = from ? mix(from.r, to.r) : to.r;
  const frames = from ? mix(from.frames, to.frames) : to.frames;
  // How faint each arrangement has a card, and a line by its ends.
  const faint = (p, id) => (!p.taken || p.taken.has(id) ? 1 : AREA.rest);
  const faintLine = (p, x, y) => (!p.taken ? 1 : p.taken.has(x) || p.taken.has(y) ? AREA.ends : AREA.lines);
  // The level draws no line of its own where the area draws one.
  const wires = to.lines ? g.wires.filter(([x, y]) => !to.lines.has(pairKey(x, y))) : g.wires;
  return {
    g: { ...g, r, frames, wires },
    to,
    e,
    now: { ...to, r, frames },
    rest: (id) => lerp(from ? faint(from, id) : 1, faint(to, id), e),
    restLine: (x, y) => lerp(from ? faintLine(from, x, y) : 1, faintLine(to, x, y), e),
    under: (from ? areaUnder(from, 1 - e) : "") + areaUnder(to, e),
  };
}

// Where the drawing shows the area of `p`, and under it the box, or as
// much of it as fits while the area still reads.
function areaShown(p) {
  const { left, w, h } = room();
  const all = outline([p.edge, p.box]);
  let k = Math.min(1, w / (all.x1 - all.x0), h / (all.y1 - all.y0));
  if (k >= AREA.readable) return { k, x: left + w / 2 - (k * (all.x0 + all.x1)) / 2, y: h / 2 - (k * (all.y0 + all.y1)) / 2 };
  const peek = Math.min(p.box.h, 2 * IH) + AREA.out;
  k = Math.max(AREA.readable, Math.min(1, w / (p.edge.w + 40), h / (p.edge.h + peek + 40)));
  return { k, x: left + w / 2 - k * (p.edge.x + p.edge.w / 2), y: 20 - k * p.edge.y };
}

/// Chooses the function `next`: its `own` type, its `name`, and `links`,
/// each from a node to a node as "caller", "call", "field" or "target";
/// or none. The cards of what it relates to go to an area above the level,
/// and back as it is let go, shown in what panels `left` and `right` of
/// the drawing leave, or, with neither, above a drawer from below.
export function focus(next, left = 0, right = 0) {
  const level = view.stack.at(-1);
  const steady = !pair && !anim;
  const was = steady ? arranged(geo(level)) : null;
  const heading = shift?.cam1;
  view.focus = next && { ...next, level };
  if (next) clear = { left, right };
  shift = null;
  const to = steady ? arrangement(geo(level)) : null;
  if (!was || (!was.to.taken && !was.under && !to.taken)) {
    home = null;
    return;
  }
  if (to.taken && !was.to.taken) home = heading ?? { ...cam };
  const here = { from: was.now, t: 0, cam0: { ...cam }, cam1: to.taken ? areaShown(to) : (home ?? { ...cam }) };
  if (!to.taken) home = null;
  // A page out of sight gets no frames to animate in: it goes there.
  if (document.hidden) {
    cam = here.cam1;
    apply();
    return;
  }
  shift = here;
  let last = performance.now();
  const step = (now) => {
    // Overtaken, by another function chosen or a level opened.
    if (shift !== here) return;
    if (pair || view.stack.at(-1) !== level) {
      shift = null;
      return;
    }
    here.t = Math.min(1, here.t + Math.min(0.05, (now - last) / 1000) / AREA.takes);
    last = now;
    const e = ease(here.t);
    cam = { k: lerp(here.cam0.k, here.cam1.k, e), x: lerp(here.cam0.x, here.cam1.x, e), y: lerp(here.cam0.y, here.cam1.y, e) };
    apply();
    if (here.t === 1) shift = null;
    safely(draw);
    if (shift === here) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

// Between the level A, above, and the level B, inside its box f: s = 0
// draws A and s = 2 draws B. Up to 1, f grows into the edge of B; past
// it, B's boxes and lines come out.
let pair = null;
let s = 0;
let anim = false;
// The cards lit in what is being drawn, as the chosen one is.
let lit = new Set();
let cam = { x: 0, y: 0, k: 1 };
let world = { x0: -100, x1: 100, y0: -100, y1: 100 };
const hooks = [];

/// Whether a level is being opened or left.
export const moving = () => Boolean(pair);
// The levels between which the drawing goes, unless one is gone, as when
// the map was read again mid-way.
const going = () => (pair && pair.every((f) => f === "root" || canOpen(f)) ? pair : null);
/// Has `hook` run after the map is drawn anew, as the chosen node changes.
export const afterRender = (hook) => hooks.push(hook);

/// Draws the map as `view` says, without the panels beside it.
export function draw() {
  if (!top.center) return;
  const between = going();
  const [A, B] = between ?? [parentOf(view.stack.at(-1)), view.stack.at(-1)];
  const resting = !between;
  const sv = resting ? (B === "root" ? 0 : 2) : s;
  const a = resting && B === "root" ? "root" : A;
  const b = resting && B === "root" ? null : B;
  const t1 = b ? ease(clamp(sv)) : 0;
  const t2 = b ? ease(clamp(sv - 1)) : 0;
  let gA = geo(a);
  let gB = b ? geo(b) : null;
  // A function chosen moves the cards of the level shown, at rest.
  const act = resting ? arranged(gB ?? gA) : null;
  if (act && gB) gB = act.g;
  else if (act) gA = act.g;
  const g = {};
  const o = [];
  for (const k in gA.r)
    g[k] = gB && gB.r[k] && k !== a ? (k === b ? lerpR(gA.r[k], gB.box, t1) : lerpR(gA.r[k], gB.r[k], t1)) : gA.r[k];
  if (gB) for (const k in gB.r) g[k] ??= gB.r[k];
  const onlyA = (id) => gB && !(id in gB.r);
  const onlyB = (id) => !(id in gA.r);
  const rect = (id) => g[id];
  const here = gB ?? gA;
  lit = here.mark;
  // A function chosen lights the lines of its area alone.
  const flow = act?.to.lines;
  const isLit = (x, y) => (view.focus ? Boolean(flow?.has(pairKey(x, y))) : view.sel && (x === view.sel || y === view.sel));
  const edgesOf = (gg, f) => (gg ? { ...gg.frames, ...(gg.box ? { [f]: gg.box } : {}) } : {});
  const bA = edgesOf(gA, a);
  const bB = edgesOf(gB, b);
  // A line to a box opened above, which it kept as it opened, goes to its
  // edge, as that edge moves.
  for (const id of new Set([...Object.keys(bA), ...Object.keys(bB)]))
    if (id !== b) g[id] = bA[id] && bB[id] ? lerpR(bA[id], bB[id], t1) : (g[id] ?? bA[id] ?? bB[id]);
  for (const id in bA)
    if (id !== b) o.push(boundary(id, bB[id] ? lerpR(bA[id], bB[id], t1) : bA[id], bB[id] || !gB ? 1 : 1 - t1));
  if (act) o.push(act.under);
  // What the open box's lines do not reach is faint.
  const near = (gg) => {
    if (!gg || !gg.parts.length) return null;
    const P = within(gg);
    const n = new Set(P);
    gg.wires.forEach(([x, y]) => {
      if (P.has(x)) n.add(y);
      if (P.has(y)) n.add(x);
    });
    return n;
  };
  const nA = near(gA);
  const nB = near(gB);
  const wires = here.wires;
  const ofChosen = view.sel && !flow ? wires.filter(([x, y]) => x === view.sel || y === view.sel).flat() : [];
  const litNodes = new Set([...lit, ...(view.sel ? [view.sel, ...ofChosen] : [])]);
  const dim = (id) =>
    litNodes.has(id) ? 1 : lerp(nA && !nA.has(id) ? FAR : 1, nB && !nB.has(id) ? FAR : 1, t1) * (act ? act.rest(id) : 1);
  const inA = within(gA);
  const inB = gB && within(gB);
  const wireDim = (x, y) => {
    if (isLit(x, y)) return 1;
    const f = (gg, P) => (gg && gg.parts.length && !P.has(x) && !P.has(y) ? FAR : 1);
    return lerp(f(gA, inA), f(gB, inB), t1) * (act ? act.restLine(x, y) : 1);
  };
  // The box opened: its card fades as its edge comes.
  if (b) {
    const r = g[b];
    o.push(boundary(b, r, t1));
    if (t1 < 1) o.push(b === center() ? centerCard(b, r, 1 - t1) : card(b, r, 1 - t1));
  }
  const wire = (d, x, y, back, op = 1) => {
    const lit = isLit(x, y);
    const n = weights[pairKey(x, y)];
    const width = n ? (0.8 + (1.8 * Math.log10(n)) / Math.log10(heaviest)).toFixed(2) : 1.4;
    const faint = n && n < 10 && !lit ? 0.35 : 1;
    o.push(
      `<path class="${lit ? "lit" : ""}" d="${d}" fill="none" stroke="var(--focused-edge)" stroke-width="${width}" opacity="${op * wireDim(x, y) * faint}" marker-end="url(#${lit ? "hl" : "h"})" ${back ? `marker-start="url(#${lit ? "hl" : "h"})"` : ""}/>`,
    );
  };
  if (t2 === 0) {
    anchors(gA.wires, rect).forEach((P, i) => {
      const [x, y, k] = gA.wires[i];
      wire(path(P.a, P.b), x, y, k, onlyA(x) || onlyA(y) ? 1 - t1 : 1);
    });
  } else {
    // The lines out of the box slide from its edge to the parts inside.
    const inB = new Set(gB.parts);
    const Wb = gB.wires;
    const ext = Wb.filter(([x, y]) => inB.has(x) !== inB.has(y));
    const int = Wb.filter(([x, y]) => inB.has(x) && inB.has(y));
    const oth = Wb.filter(([x, y]) => !inB.has(x) && !inB.has(y));
    const A0 = anchors([...ext.map(([x, y, k]) => [inB.has(x) ? b : x, inB.has(y) ? b : y, k]), ...oth], rect);
    const A1 = anchors([...ext, ...int, ...oth], rect);
    const E = ext.length;
    const I = int.length;
    const mix = (p, q) => ({ x: lerp(p.x, q.x, t2), y: lerp(p.y, q.y, t2), n: t2 < 0.5 ? p.n : q.n });
    ext.forEach((e, i) => wire(path(mix(A0[i].a, A1[i].a), mix(A0[i].b, A1[i].b)), e[0], e[1], e[2], onlyB(inB.has(e[0]) ? e[1] : e[0]) ? t2 : 1));
    oth.forEach((e, j) =>
      wire(path(mix(A0[E + j].a, A1[E + I + j].a), mix(A0[E + j].b, A1[E + I + j].b)), e[0], e[1], e[2], onlyB(e[0]) || onlyB(e[1]) ? t2 : 1),
    );
    int.forEach((e, i) => wire(path(A1[E + i].a, A1[E + i].b), e[0], e[1], e[2], t2));
  }
  // What a function chosen calls and uses, and who calls it: a line for
  // each, whether the level draws one between the two or not.
  if (flow) {
    const lines = [...flow.values()];
    // They come as the cards come to rest, not across their way there.
    const op = clamp((act.e - 0.6) / 0.4);
    anchors(lines, rect).forEach((P, i) => {
      const [x, y, both, , way] = lines[i];
      if (op > 0) wire(way ? pathBy(P.a, P.b, way) : path(P.a, P.b), x, y, both, op);
    });
  }
  if (!between)
    for (const [x, y] of view.ghost) {
      if (!g[x] || !g[y] || x === y) continue;
      const A = anchors([[x, y]], rect)[0];
      const hot = view.ghostOn === `${x}|${y}`;
      o.push(
        `<path d="${path(A.a, A.b)}" fill="none" stroke="var(--drift)" stroke-width="${hot ? 2.4 : 1.4}" stroke-dasharray="5 5" opacity="${view.ghostOn && !hot ? 0.25 : 0.85}"/>`,
      );
    }
  for (const id in gA.r) {
    if (id === a || id === b) continue;
    const op = (onlyA(id) ? 1 - t1 : 1) * dim(id);
    if (op > 0) o.push(id === center() ? centerCard(id, g[id], op) : card(id, g[id], op));
  }
  if (gB)
    for (const id in gB.r) {
      if (id === b || !onlyB(id) || t2 === 0) continue;
      o.push(card(id, g[id], t2 * dim(id), gB.parts.includes(id)));
    }
  W.innerHTML = o.join("");
  const b0 = bounds(gA);
  const b1 = gB ? bounds(gB) : b0;
  world = { x0: lerp(b0.x0, b1.x0, t1), x1: lerp(b0.x1, b1.x1, t1), y0: lerp(b0.y0, b1.y0, t1), y1: lerp(b0.y1, b1.y1, t1) };
  if (anim) fitAll();
}

/// Draws the map, and the panels beside it.
export function render() {
  draw();
  for (const hook of hooks) hook();
}

// What is asked while a level opens or closes, as a link followed then:
// it is done once the drawing stands still.
const waiting = [];
/// Does `action` now, or once the level opening or closing is open or
/// closed.
export function whenStill(action) {
  if (anim) waiting.push(action);
  else action();
}

const safely = (action) => {
  try {
    action();
    return true;
  } catch (error) {
    console.error(error);
    return false;
  }
};

function play(from, to, done) {
  // However it ends, it ends: the drawing stands still, and what waited
  // for it is done.
  const end = () => {
    anim = false;
    s = to;
    safely(done);
    if (!anim) {
      pair = null;
      safely(render);
      while (!anim && waiting.length) safely(waiting.shift());
    }
  };
  // A page out of sight gets no frames to animate in: it goes there.
  if (document.hidden) {
    end();
    return;
  }
  anim = true;
  s = from;
  let last = performance.now();
  const step = (now) => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    s = to > s ? Math.min(to, s + dt / 0.55) : Math.max(to, s - dt / 0.55);
    if (!going() || !safely(render)) s = to;
    if (s !== to) requestAnimationFrame(step);
    else end();
  };
  requestAnimationFrame(step);
}

/// Opens the box `id`, inside the level shown.
export function open(id) {
  if (anim || !canOpen(id)) return;
  view.sel = null;
  pair = [view.stack.at(-1), id];
  play(0, 2, () => {
    if (canOpen(id)) view.stack.push(id);
  });
}

/// Goes up to the `n`th level opened, the top being 0.
export function up(n) {
  if (anim || view.stack.length - 1 <= n) return;
  view.sel = null;
  const f = view.stack.at(-1);
  view.stack.pop();
  pair = [view.stack.at(-1), f];
  play(2, 0, () => {
    if (view.stack.length - 1 > n) up(n);
  });
}

/// Shows the level inside `f` at once, as a link to it does, with
/// `chosen` chosen there.
export function goTo(f, chosen = null) {
  if (anim) {
    whenStill(() => goTo(f, chosen));
    return;
  }
  view.stack = ["root", ...chain(f)];
  view.sel = chosen;
  pair = null;
  draw();
  fitAll();
  for (const hook of hooks) hook();
}

function apply() {
  W.setAttribute("transform", `translate(${cam.x},${cam.y}) scale(${cam.k})`);
}
// Shows the drawing as `next` says: at once, or, while the cards go to a
// function's area or back, as they come to rest.
function aim(next) {
  if (shift && !pair && shift.from.id === view.stack.at(-1)) shift.cam1 = next;
  else {
    cam = next;
    apply();
  }
}
/// Fits what is drawn to the drawing's area.
export function fitAll() {
  const w = cv.clientWidth;
  const h = cv.clientHeight;
  if (!w || !h) return;
  const k = Math.min(1.25, Math.min(w / (world.x1 - world.x0), h / (world.y1 - world.y0)) * 0.94);
  aim({ k, x: w / 2 - (k * (world.x0 + world.x1)) / 2, y: h / 2 - (k * (world.y0 + world.y1)) / 2 });
}
/// Fits the node `id`, and what its lines reach, to what the panels
/// leave of the drawing: `left` and `right` of it on a wide screen, or,
/// on a narrow one, above the drawer from below.
export function fitAbove(id, left = 0, right = 0) {
  const g = geo(view.stack.at(-1));
  const ids = new Set([id]);
  g.wires.forEach(([x, y]) => {
    if (x === id) ids.add(y);
    if (y === id) ids.add(x);
  });
  const v = [...ids].map((k) => g.r[k]).filter(Boolean);
  if (!v.length) return;
  const x0 = Math.min(...v.map((r) => r.x)) - 20;
  const x1 = Math.max(...v.map((r) => r.x + r.w)) + 20;
  const y0 = Math.min(...v.map((r) => r.y)) - 20;
  const y1 = Math.max(...v.map((r) => r.y + r.h)) + 20;
  const w = cv.clientWidth - left - right;
  const h = left || right ? cv.clientHeight : innerHeight * 0.5 - cv.getBoundingClientRect().top;
  const k = Math.min(1, w / (x1 - x0), h / (y1 - y0));
  if ((left || right) && k < 0.8) {
    const r = g.r[id];
    const kk = 0.8;
    return aim({ k: kk, x: left + w / 2 - kk * (r.x + r.w / 2), y: h / 2 - kk * (r.y + r.h / 2) });
  }
  aim({ k, x: left + w / 2 - (k * (x0 + x1)) / 2, y: h / 2 - (k * (y0 + y1)) / 2 });
}
function zoomAt(f, px, py) {
  const k = Math.max(0.3, Math.min(3, cam.k * f));
  cam.x = px - ((px - cam.x) * k) / cam.k;
  cam.y = py - ((py - cam.y) * k) / cam.k;
  cam.k = k;
  apply();
}

cv.addEventListener(
  "wheel",
  (e) => {
    e.preventDefault();
    const r = cv.getBoundingClientRect();
    if (e.ctrlKey || e.metaKey) zoomAt(Math.exp(-e.deltaY * 0.01), e.clientX - r.left, e.clientY - r.top);
    else {
      cam.x -= e.deltaX;
      cam.y -= e.deltaY;
      apply();
    }
  },
  { passive: false },
);
// A press chooses a box, which lights its lines; its corner's mark, or a
// second press, opens it. A drag moves the drawing; two fingers, as on a
// phone, move it and pinch it larger or smaller, and choose nothing.
let drag = null;
const fingers = new Map();
let pinch = null;
// Where two fingers are: how far apart, and the point between them.
function spread() {
  const [a, b] = [...fingers.values()];
  return { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}
cv.addEventListener("pointerdown", (e) => {
  if (e.pointerType === "touch") fingers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  if (fingers.size >= 2) {
    pinch = spread();
    drag = null;
    cv.classList.remove("drag");
    return;
  }
  drag = {
    x: e.clientX,
    y: e.clientY,
    cx: cam.x,
    cy: cam.y,
    moved: 0,
    on: e.target.closest(".obj")?.dataset.id,
    open: e.target.closest("[data-open]")?.dataset.open,
  };
});
addEventListener("pointermove", (e) => {
  if (fingers.has(e.pointerId)) fingers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  if (pinch && fingers.size >= 2) {
    const now = spread();
    const r = cv.getBoundingClientRect();
    cam.x += now.x - pinch.x;
    cam.y += now.y - pinch.y;
    zoomAt(now.d / pinch.d, now.x - r.left, now.y - r.top);
    pinch = now;
    return;
  }
  if (!drag) return;
  const dx = e.clientX - drag.x;
  const dy = e.clientY - drag.y;
  drag.moved = Math.max(drag.moved, Math.hypot(dx, dy));
  if (drag.moved > 4) {
    cv.classList.add("drag");
    cam.x = drag.cx + dx;
    cam.y = drag.cy + dy;
    apply();
  }
});
// A finger lifted, or taken by the browser: a pinch ends with either.
function lift(e) {
  fingers.delete(e.pointerId);
  if (fingers.size < 2) pinch = null;
}
addEventListener("pointercancel", (e) => {
  lift(e);
  drag = null;
  cv.classList.remove("drag");
});
addEventListener("pointerup", (e) => {
  lift(e);
  if (drag && drag.moved <= 4 && !anim) {
    if (drag.open) open(drag.open);
    else {
      view.sel = drag.on && drag.on !== view.sel ? drag.on : null;
      render();
    }
  }
  drag = null;
  cv.classList.remove("drag");
});
cv.addEventListener("dblclick", (e) => {
  const id = e.target.closest(".obj")?.dataset.id;
  if (id && canOpen(id)) open(id);
  else if (!id && view.stack.length > 1) up(view.stack.length - 2);
});
addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (view.sel) {
    view.sel = null;
    render();
  } else if (view.stack.length > 1) up(view.stack.length - 2);
});
document.addEventListener("click", (e) => {
  const u = e.target.closest("[data-up]");
  if (u) up(Number(u.dataset.up));
  const z = e.target.closest("[data-z]");
  if (z) {
    const f = Number(z.dataset.z);
    if (!f) fitAll();
    else zoomAt(f > 0 ? 1.25 : 0.8, cv.clientWidth / 2, cv.clientHeight / 2);
  }
});
addEventListener("resize", fitAll);
