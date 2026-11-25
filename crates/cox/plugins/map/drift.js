// The drift: where the map's vocabulary and the code, as indexed,
// disagree, to talk through one by one, and settle by fixing the
// vocabulary, changing the code, or agreeing to leave it. A count on each
// box with drift open; the list behind the button in the bar, whose rows
// light the boxes they are on as they are pointed at, and go to them as
// they are pressed; and, while it is open, a dashed line where the code
// of two boxes meets that no level draws a line between.

import { canOpen, center, chain, draw, esc, fitAbove, fitAll, goTo, parentOf, render, view } from "./canvas.js";
import { showType } from "./detail.js";

const KINDS = [
  ["undrawn", "Code that meets where no level draws a line"],
  ["unowned", "Code no box claims"],
  ["claim-gone", "Claims the code no longer has"],
  ["no-types", "Boxes that own no types"],
  ["rule-empty", "Lines whose rule picks no type"],
  ["agreed-stale", "Agreements nothing matches"],
  ["agreed", "Agreed"],
];

const button = document.getElementById("dfb");
const dr = document.getElementById("dr");
const narrow = () => matchMedia("(max-width:700px)").matches;
const RIGHT = 420;

let drift = [];
let open = [];
let byId = {};
let types = {};

/// Reads the drift of `map`, from its vocabulary in `file`, with whether
/// it is told at all: not while some of the code is not indexed.
export function setDrift(map, file, told) {
  drift = map.drift;
  open = drift.filter((x) => !x.agreed);
  byId = Object.fromEntries(drift.map((x) => [x.id, x]));
  types = map.types;
  view.drift = {};
  for (const x of open) for (const n of x.nodes) view.drift[n] = (view.drift[n] ?? 0) + 1;
  button.hidden = !told;
  button.textContent = `Drift ${open.length}`;
  button.title = open.length ? "Where the vocabulary and the code disagree" : "The vocabulary and the code agree";
  const sites = (x) =>
    (x.at ?? [])
      .map((at) => `<div class="at">${esc(at.file)}:${at.line}${at.in ? ` in ${esc(at.in)}` : ""}</div>`)
      .join("");
  dr.innerHTML =
    `<div class="hd2"><b>Drift ${open.length}</b><div>Where the vocabulary, ${esc(file)}, and the code as indexed disagree: fix the vocabulary, change the code, or agree to leave it in its <code>agreed</code>. Agreed ${drift.length - open.length}.</div></div>` +
    KINDS.map(([kind, label]) => {
      const xs = drift.filter((x) => (kind === "agreed" ? Boolean(x.agreed) : !x.agreed && x.kind === kind));
      return xs.length
        ? `<div class="dk">${label} ${xs.length}</div>` +
            xs
              .map(
                (x) =>
                  `<div class="di${x.agreed ? " ok" : ""}" data-d="${esc(x.id)}"><div class="tx">${esc(x.text)}</div>${x.agreed ? `<div class="why">Agreed: ${esc(x.agreed)}</div>` : ""}${sites(x)}<div class="id">${esc(x.id)}</div></div>`,
              )
              .join("")
        : "";
    }).join("");
  if (!told && isOpen()) show(false);
  else if (isOpen()) view.ghost = undrawn();
}

const isOpen = () => dr.classList.contains("on");
const undrawn = () => open.filter((x) => x.kind === "undrawn").map((x) => x.nodes);
const fitHere = (n) => (narrow() ? fitAbove(n) : fitAbove(n, 0, RIGHT));

// The level the most drift is in, of those placed: where the list opens
// the map when it is at the top.
function busiest() {
  const count = {};
  for (const x of open) for (const n of x.nodes) if (!types[n]) count[parentOf(n)] = (count[parentOf(n)] ?? 0) + 1;
  const best = Object.entries(count).sort((a, b) => b[1] - a[1])[0];
  return best && best[0] !== "root" ? best[0] : canOpen(center()) ? center() : "root";
}

/// Shows the list, or hides it.
export function show(on) {
  dr.classList.toggle("on", on);
  button.classList.toggle("on", on);
  view.ghost = on ? undrawn() : [];
  view.ghostOn = null;
  if (on) {
    view.sel = null;
    if (view.stack.length < 2) goTo(busiest());
  }
  render();
  if (!on) fitAll();
  else {
    const first = open.find((x) => x.nodes.length)?.nodes[0];
    if (first && chain(first).includes(view.stack.at(-1))) fitHere(first);
  }
}

button.addEventListener("click", () => show(!isOpen()));
dr.addEventListener("pointerover", (e) => {
  const d = e.target.closest("[data-d]");
  const x = d && byId[d.dataset.d];
  view.mark = new Set(x ? x.nodes : []);
  view.ghostOn = x && x.kind === "undrawn" ? x.nodes.join("|") : null;
  draw();
});
dr.addEventListener("pointerleave", () => {
  view.mark = new Set();
  view.ghostOn = null;
  draw();
});
// A row pressed goes to where its drift is: a type, in the level of the
// box that owns it, chosen; a box, in the level it is in, in the middle.
dr.addEventListener("click", (e) => {
  const d = e.target.closest("[data-d]");
  const x = d && byId[d.dataset.d];
  if (!x || !x.nodes.length) return;
  const n = x.nodes[0];
  if (types[n]) {
    show(false);
    showType(n);
    return;
  }
  const level = parentOf(n);
  if (view.stack.at(-1) !== level) goTo(level);
  view.mark = new Set(x.nodes);
  draw();
  fitHere(n);
});
