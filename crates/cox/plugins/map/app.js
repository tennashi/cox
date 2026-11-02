// The map of the chat's project, drawn by a page of the stage instead of
// a tab Cox draws: the same picture as the Map tab, from the same facts,
// read as JSON from `cox://api/map/<crate>/<module>…` and read again
// every two seconds, as Cox's own tabs follow what they show. Where the
// map is open is the page's hash, `#cox/stage`, so that the browser's
// back and forward go between the places it was open at. Each box is a
// card: a crate, a file, or a module, with what it says of itself, its
// counts, and what changed in it. A click on a crate or a module with
// modules of its own opens the map at it, and a click on a file goes to
// a `cox://open/` address, which the stage takes for that file, and opens
// on a tab of its own. The mouse resting on a box colors the boxes it
// uses and those that use it, and the heading names them.

import { h, render } from "preact";
import { useEffect, useState } from "preact/hooks";
import htm from "htm";

const html = htm.bind(h);

const API = "cox://api/map";
const EVERY = 2000;

const KINDS = {
  crate: "var(--crate)",
  file: "var(--file)",
  module: "var(--module)",
};

/// The names the map is open at, from the page's hash.
function atFromHash() {
  return location.hash
    .replace(/^#\/?/, "")
    .split("/")
    .filter(Boolean)
    .map(decodeURIComponent);
}

/// The hash that opens the map at `at`.
function hashOf(at) {
  return "#" + at.map(encodeURIComponent).join("/");
}

/// The names the map is open at, following the hash as it changes.
function useAt() {
  const [at, setAt] = useState(atFromHash);
  useEffect(() => {
    const changed = () => setAt(atFromHash());
    addEventListener("hashchange", changed);
    return () => removeEventListener("hashchange", changed);
  }, []);
  return at;
}

/// What Cox says the map shows at `at`, read again now and then, and the
/// error if it said nothing.
function useMap(at) {
  const [map, setMap] = useState(null);
  const [error, setError] = useState(null);
  const key = at.map(encodeURIComponent).join("/");
  useEffect(() => {
    let live = true;
    const load = async () => {
      try {
        const response = await fetch(`${API}/${key}`);
        if (!response.ok) throw new Error(`${API} answered ${response.status}`);
        const read = await response.json();
        if (live) {
          setMap(read);
          setError(null);
        }
      } catch (failed) {
        if (live) setError(String(failed));
      }
    };
    load();
    const timer = setInterval(load, EVERY);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [key]);
  return [map, error];
}

function plural(count, one) {
  return `${count} ${one}${count === 1 ? "" : "s"}`;
}

/// What a box has to do with the box at `hovered`: "uses", "used-by",
/// "both", "hovered", or nothing.
function relation(map, index, hovered) {
  if (hovered === null) return "";
  if (index === hovered) return "hovered";
  const uses = map.boxes[hovered].uses.includes(index);
  const usedBy = map.boxes[index].uses.includes(hovered);
  if (uses && usedBy) return "both";
  if (uses) return "uses";
  if (usedBy) return "used-by";
  return "";
}

function Box({ map, box, index, relation, onHover }) {
  const opens = box.opens;
  const href = opens ? `cox://open${box.path}` : hashOf([...map.at, box.name]);
  const counts =
    box.modules === null || box.modules === undefined
      ? plural(box.lines, "line")
      : `${plural(box.modules, "module")}, ${plural(box.lines, "line")}`;
  const file = box.path.split("/").pop();
  const classes = ["box", relation, opens ? "opens" : "", box.changed ? "changed" : ""]
    .filter(Boolean)
    .join(" ");
  return html`
    <a
      class=${classes}
      style=${`--kind: ${KINDS[box.kind] || KINDS.module}`}
      href=${href}
      title=${opens ? `Open ${file}` : `Open the map at ${box.name}`}
      onMouseEnter=${() => onHover(index)}
      onMouseLeave=${() => onHover(null)}
    >
      <span class="kind">${box.kind}</span>
      <span class="name">${box.name}</span>
      <span class="counts">${counts}</span>
      ${box.said && html`<span class="says">${box.said}</span>`}
      ${box.crates.length > 0 && html`<span class="note">uses ${box.crates.join(", ")}</span>`}
      ${box.changed &&
      html`<span class="diff">
        +${box.changed.added} −${box.changed.removed} in ${plural(box.changed.files, "file")}
      </span>`}
      ${!box.changed && opens && html`<span class="foot">${file} ›</span>`}
      ${!box.changed && !opens && html`<span class="foot">open ›</span>`}
    </a>
  `;
}

/// The line above the boxes: ‹ and ›, the path to where the map is open,
/// each step a link back up, and what the box the mouse rests on has to
/// do with the others, or else what changed since.
function Heading({ map, at, hovered }) {
  const crumbs = [];
  const project = map ? map.project : "…";
  const names = [project, ...(map ? map.at : at)];
  names.forEach((name, index) => {
    if (index > 0) crumbs.push(html`<span class="sep">›</span>`);
    const last = index === names.length - 1;
    crumbs.push(
      last
        ? html`<span>${name}</span>`
        : html`<a href=${hashOf(names.slice(1, index + 1))}>${name}</a>`,
    );
  });
  let aside = null;
  if (map && hovered !== null && map.boxes[hovered]) {
    const box = map.boxes[hovered];
    const related = (...wanted) =>
      map.boxes.filter((_, index) => wanted.includes(relation(map, index, hovered)));
    const uses = related("uses", "both");
    const usedBy = related("used-by", "both");
    aside = html`<span class="name">${box.name}</span>${" "}
      ${uses.length === 0 && usedBy.length === 0 && "uses none of these"}
      ${uses.length > 0 && html`uses <span class="uses">${uses.map((b) => b.name).join(", ")}</span>`}
      ${uses.length > 0 && usedBy.length > 0 && html`<span class="dot">·</span>`}
      ${usedBy.length > 0 && html`used by <span class="used-by">${usedBy.map((b) => b.name).join(", ")}</span>`}`;
  } else if (map && map.missing) {
    aside = html`<span style="color: var(--failed)">there is no ${map.missing} here</span>`;
  } else if (map && map.since) {
    aside =
      map.changed_files > 0
        ? `${plural(map.changed_files, "file")} changed ${map.since}`
        : `nothing changed ${map.since}`;
  }
  return html`
    <div class="heading">
      <div class="steps">
        <span title="Back" onClick=${() => history.back()}>‹</span>
        <span title="Forward" onClick=${() => history.forward()}>›</span>
      </div>
      <div class="crumbs">${crumbs}</div>
      <div class="aside">${aside}</div>
    </div>
  `;
}

function App() {
  const at = useAt();
  const [map, error] = useMap(at);
  const [hovered, setHovered] = useState(null);
  useEffect(() => setHovered(null), [at.join("/")]);
  let said = null;
  if (error) {
    said = html`<div class="said failed">${error}</div>`;
  } else if (!map) {
    said = html`<div class="said">Reading the project…</div>`;
  } else if (map.error) {
    said = html`<div class="said failed">${map.error}</div>`;
  }
  return html`
    <${Heading} map=${map} at=${at} hovered=${hovered} />
    ${said}
    ${map &&
    html`<div class="boxes">
      ${map.boxes.map(
        (box, index) =>
          html`<${Box}
            key=${box.path}
            map=${map}
            box=${box}
            index=${index}
            relation=${relation(map, index, hovered)}
            onHover=${setHovered}
          />`,
      )}
    </div>`}
  `;
}

render(html`<${App} />`, document.getElementById("app"));
