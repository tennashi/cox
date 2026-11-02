// The parts a chat runs with, drawn by a page of the stage instead of a
// tab Cox draws: the same picture as the Parts tab, from the same facts,
// read as JSON from `cox://api/parts` and read again every two seconds,
// as Cox's own tabs follow what they show. A click on a part goes to a
// `cox://open/` address, which the stage takes for the code that starts
// the part, and opens on a tab of its own.

import { h, render } from "preact";
import { useEffect, useState } from "preact/hooks";
import htm from "htm";

const html = htm.bind(h);

const API = "cox://api/parts";
const EVERY = 2000;

/// What Cox says the chat runs with now, and the error if it said nothing.
function useParts() {
  const [parts, setParts] = useState(null);
  const [error, setError] = useState(null);
  useEffect(() => {
    let live = true;
    const load = async () => {
      try {
        const response = await fetch(API);
        if (!response.ok) throw new Error(`${API} answered ${response.status}`);
        const read = await response.json();
        if (live) {
          setParts(read);
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
  }, []);
  return [parts, error];
}

/// The address that opens the code starting `part`, when the project has it.
function opens(part) {
  if (!part.file) return null;
  return `cox://open${part.file}` + (part.line ? `#L${part.line}` : "");
}

function Part({ part }) {
  const href = opens(part);
  const file = part.file ? part.file.split("/").pop() : null;
  const inside = html`
    <div class="name">${part.name}</div>
    <div class="says">${part.says}</div>
    <div class="started">${part.started}</div>
    ${part.detail && html`<div class="detail">${part.detail}</div>`}
    ${file &&
    html`<div class="code">${file}${part.line ? `:${part.line}` : ""}</div>`}
  `;
  return href
    ? html`<a class="part" href=${href}>${inside}</a>`
    : html`<div class="part">${inside}</div>`;
}

function Between({ between }) {
  const left = between.way !== "right";
  const right = between.way !== "left";
  return html`
    <div class="between">
      <div class="arrow">
        <span>${left ? "◀" : ""}</span>
        <span class="line"></span>
        <span>${right ? "▶" : ""}</span>
      </div>
      <div>${between.says}</div>
    </div>
  `;
}

function Row({ row }) {
  const across = [];
  row.parts.forEach((part, index) => {
    if (index > 0 && row.between[index - 1]) {
      across.push(html`<${Between} between=${row.between[index - 1]} />`);
    }
    across.push(html`<${Part} part=${part} />`);
  });
  return html`
    <div class="row">
      ${row.when && html`<div class="when">${row.when}</div>`}
      <div class="across">${across}</div>
    </div>
  `;
}

function App() {
  const [parts, error] = useParts();
  let heading;
  if (error) {
    heading = html`<span class="error">${error}</span>`;
  } else if (!parts) {
    heading = "Asking Cox what the chat runs with…";
  } else if (parts.running) {
    heading =
      "The parts the chat runs with, as Cox started them. " +
      "A click on a part opens the code that starts it.";
  } else {
    heading =
      "The parts a chat runs with, once its agent runs; this chat's does not now. " +
      "A click on a part opens the code that starts it.";
  }
  return html`
    <div class="heading">${heading}</div>
    ${parts && parts.rows.map((row) => html`<${Row} row=${row} />`)}
  `;
}

render(html`<${App} />`, document.getElementById("app"));
