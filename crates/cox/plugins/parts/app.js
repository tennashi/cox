// The parts a chat runs with, drawn by a page of the stage instead of a
// tab Cox draws: the same picture as the Parts tab, from the same facts,
// read as JSON from `cox://api/parts` and read again every two seconds,
// as Cox's own tabs follow what they show. Each part is a card, colored
// by what kind of thing it is (a process, a thread of Cox's, or part of
// the window), with what it is, who starts it, how it was started while
// the agent runs, and the code that starts it. A click on a card goes to
// a `cox://open/` address, which the stage takes for that code, and
// opens on a tab of its own.

import { h, render } from "preact";
import { useEffect, useState } from "preact/hooks";
import htm from "htm";

const html = htm.bind(h);

const API = "cox://api/parts";
const EVERY = 2000;

const KINDS = {
  process: { name: "a process of its own", color: "var(--process)" },
  thread: { name: "a thread of Cox's", color: "var(--thread)" },
  window: { name: "part of the window", color: "var(--window)" },
};

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
  const kind = KINDS[part.kind] || KINDS.process;
  const file = part.file ? part.file.split("/").pop() : null;
  const style = `--kind: ${kind.color}`;
  const inside = html`
    <div class="head">
      <span class="kind">${part.kind}</span>
      <span class="name">${part.name}</span>
    </div>
    <div class="says">${part.says}</div>
    <div class="started">${part.started}</div>
    ${part.detail && html`<div class="detail">${part.detail}</div>`}
    ${file &&
    html`<div class="code">
      ${file}${part.line ? `:${part.line}` : ""} ${href ? "›" : ""}
    </div>`}
  `;
  return href
    ? html`<a class="part" style=${style} href=${href} title=${`Open ${file}:${part.line}`}>${inside}</a>`
    : html`<div class="part" style=${style}>${inside}</div>`;
}

/// An arrow between two parts, pointing the way what goes between them
/// goes, with what it is above.
function Between({ between }) {
  const left = between.way !== "right";
  const right = between.way !== "left";
  const w = 100;
  return html`
    <div class="between">
      <div class="label">${between.says}</div>
      <svg viewBox="0 0 ${w} 12" preserveAspectRatio="none">
        <line x1=${left ? 8 : 0} y1="6" x2=${right ? w - 8 : w} y2="6" />
        ${left && html`<polygon points="0,6 9,1.5 9,10.5" />`}
        ${right && html`<polygon points="${w},6 ${w - 9},1.5 ${w - 9},10.5" />`}
      </svg>
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
    <section class="row">
      <div class="row-head">
        <span class="name">${row.name}</span>
        ${row.when && html`<span class="when">${row.when}</span>`}
      </div>
      <div class="across">${across}</div>
    </section>
  `;
}

function Legend() {
  return html`
    <div class="legend">
      ${Object.entries(KINDS).map(
        ([key, kind]) => html`<span style=${`--kind: ${kind.color}`}>${kind.name}</span>`,
      )}
    </div>
  `;
}

function App() {
  const [parts, error] = useParts();
  const running = Boolean(parts && parts.running);
  let lead;
  if (error) {
    lead = html`<span class="error">${error}</span>`;
  } else if (!parts) {
    lead = "Asking Cox what the chat runs with…";
  } else if (running) {
    lead = "What the chat runs with, as Cox started it. A click on a part opens the code that starts it.";
  } else {
    lead = "What a chat runs with once its agent runs; this chat's does not now. A click on a part opens the code that starts it.";
  }
  return html`
    <div class="top">
      <h1>Parts</h1>
      ${parts &&
      html`<span class=${`status ${running ? "running" : ""}`}>
        <span class="dot"></span>${running ? "agent running" : "agent not running"}
      </span>`}
      <${Legend} />
    </div>
    <p class="lead">${lead}</p>
    ${parts && parts.rows.map((row) => html`<${Row} row=${row} />`)}
  `;
}

render(html`<${App} />`, document.getElementById("app"));
