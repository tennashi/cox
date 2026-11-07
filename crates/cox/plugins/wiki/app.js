// What the agents learned and read, drawn by a page of the stage, on
// three shelves the chips at its top choose between: the human's wiki,
// the pages the agents of every project keep of what they learned, read
// as JSON from `cox://api/wiki`; the project's own wiki, in `.cox/wiki`
// in its folder, from `cox://api/project-wiki`, which the home has none
// of; and the sources the agents kept of what they read, from
// `cox://api/raw`. Each is read again every two seconds, as Cox's own
// tabs follow what they show.
//
// With no page in the hash, the page lists the shelf's pages, which the
// line at its top narrows to those whose names, titles or first lines
// have every word typed, and, on a wiki, the chips to those Cox marks:
// with links to no page, linked from no page, with no title, or saying
// nowhere where they were used. A page kept in one project, for its
// chats alone, says which. The sources are listed the last kept first,
// with where each was read.
//
// The hash names the shelf and the page: `#rust/errors` a page of the
// human's wiki, `#project:` the project's wiki and `#project:people` a
// page of it, `#raw:` the sources and `#raw:rust-news/2026-11-06-...` a
// source. A page shows its text, its links opening the pages they name
// on its shelf, and, on a wiki, the pages that link to it and where it
// was used; a source shows where it was read, and when and by which
// chat. When the human reads in a language of their own, a page is
// shown in it, as far as Cox has translated it, the rest as it is
// written, with chips over it to read it as it is written instead.

const EVERY = 2000;

/// The shelves, by the prefix of the hash that names them.
const SHELVES = {
  wiki: { prefix: "", api: "cox://api/wiki", label: "Your wiki" },
  project: { prefix: "project:", api: "cox://api/project-wiki", label: "This project's wiki" },
  raw: { prefix: "raw:", api: "cox://api/raw", label: "Sources" },
};

/// What each mark of Cox's says, as a chip and on a page.
const MARKS = {
  broken: ["links to no page", "Links to a page there is not"],
  orphan: ["linked from no page", "No page links here"],
  untitled: ["no title", "Has no # title"],
  unused: ["no Used in", "Says nowhere where it was used"],
};

const app = document.getElementById("app");
// Each shelf's list, as it was last read, and its text, to tell whether
// it changed.
const lists = {};
const listTexts = {};
let page = null;
let pageText = null;
let pageOf = null;
// The hash the page was drawn for last, so that a shelf chosen is drawn
// though nothing it reads changed.
let drawn = null;
// What the line at the top holds, and the mark chosen, as the human
// left them, across what the page reads again.
let typed = "";
let marked = null;
// Whether the human reads the pages as they are written, in place of
// their language, from page to page.
let written = false;

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

/// The shelf and the page the hash names.
function shown() {
  const hash = decodeURIComponent(location.hash.replace(/^#/, ""));
  for (const [shelf, { prefix }] of Object.entries(SHELVES)) {
    if (prefix && hash.startsWith(prefix)) return { shelf, name: hash.slice(prefix.length) };
  }
  return { shelf: "wiki", name: hash };
}
const encoded = (name) => name.split("/").map(encodeURIComponent).join("/");
const href = (shelf, name) => `#${SHELVES[shelf].prefix}${encoded(name)}`;
const only = (p) => (p.only ? `<span class="only" title="Only the chats of ${esc(p.only)} find this page">only in ${esc(p.only)}</span>` : "");
const marks = (p) => (p.marks ?? []).map((m) => `<span class="mark ${m}" title="${esc(MARKS[m]?.[1] ?? m)}">${esc(MARKS[m]?.[0] ?? m)}</span>`).join("");
const entriesOf = (shelf) => (shelf === "raw" ? lists.raw?.sources : lists[shelf]?.pages) ?? [];

async function read(url) {
  try {
    const answer = await fetch(url);
    return answer.ok ? await answer.text() : null;
  } catch {
    return null;
  }
}

async function load() {
  let changed = false;
  for (const [shelf, { api }] of Object.entries(SHELVES)) {
    const text = await read(api);
    if (text !== listTexts[shelf]) {
      listTexts[shelf] = text;
      lists[shelf] = text ? JSON.parse(text) : null;
      changed = true;
    }
  }
  const { shelf, name } = shown();
  if (name) {
    const text = await read(`${SHELVES[shelf].api}/${encoded(name)}`);
    const of = `${shelf}:${name}`;
    if (text !== pageText || pageOf !== of) {
      pageText = text;
      pageOf = of;
      page = text ? JSON.parse(text) : null;
      changed = true;
    }
  } else if (pageOf !== null) {
    page = null;
    pageText = null;
    pageOf = null;
    changed = true;
  }
  if (changed || location.hash !== drawn) draw();
}

function draw() {
  drawn = location.hash;
  const { shelf, name } = shown();
  if (name) drawPage(shelf, name);
  else drawList(shelf);
}

/// The chips that choose the shelf, each with how many it holds; the
/// project's wiki is not one in the home.
function shelves(chosen) {
  return (
    `<div class="shelves">` +
    Object.entries(SHELVES)
      .filter(([shelf]) => shelf !== "project" || (lists.project && !lists.project.home) || chosen === "project")
      .map(([shelf, { label }]) => {
        const count = entriesOf(shelf).length;
        return `<a class="shelf ${shelf === chosen ? "on" : ""}" href="${href(shelf, "")}" data-shelf="${shelf}">${esc(label)}<b>${count}</b></a>`;
      })
      .join("") +
    `</div>`
  );
}

function drawList(shelf) {
  const list = lists[shelf];
  if (!list) {
    app.innerHTML = `${shelves(shelf)}<div class="empty">Cox cannot tell where the ${shelf === "raw" ? "sources are" : "wiki is"}.</div>`;
    return;
  }
  const entries = entriesOf(shelf);
  const counts =
    shelf === "raw"
      ? []
      : Object.keys(MARKS)
          .map((m) => [m, entries.filter((p) => p.marks.includes(m)).length])
          .filter(([, n]) => n);
  if (marked && !counts.some(([m]) => m === marked)) marked = null;
  const focused = document.activeElement?.id === "find";
  app.innerHTML =
    shelves(shelf) +
    `<div class="crumbs"><span>${entries.length} ${shelf === "raw" ? "source" : "page"}${entries.length === 1 ? "" : "s"}</span><span class="where">${esc(list.root)}</span></div>` +
    `<div class="find"><input id="find" placeholder="Narrow by words" value="${esc(typed)}" spellcheck="false">` +
    counts.map(([m, n]) => `<span class="chip ${marked === m ? "on" : ""}" data-mark="${m}" title="${esc(MARKS[m][1])}"><b>${n}</b> ${esc(MARKS[m][0])}</span>`).join("") +
    `</div><div class="pages" id="pages"></div>`;
  const input = document.getElementById("find");
  input.addEventListener("input", () => {
    typed = input.value;
    drawEntries(shelf);
  });
  if (focused) {
    input.focus();
    input.setSelectionRange(typed.length, typed.length);
  }
  for (const chip of app.querySelectorAll("[data-mark]")) {
    chip.addEventListener("click", () => {
      marked = marked === chip.dataset.mark ? null : chip.dataset.mark;
      drawList(shelf);
    });
  }
  drawEntries(shelf);
}

/// What a shelf with nothing on it says.
const NOTHING = {
  wiki: "The wiki has no pages yet. An agent writes one with <code>wiki_write</code>, as when the human asks a chat to <b>Save to wiki</b> from the menu of its tab.",
  project:
    "This project's wiki has no pages yet. An agent writes one with <code>wiki_write</code> and <code>keep_in_project</code>, for what holds only of this project, into <code>.cox/wiki</code> in its folder.",
  raw: "No source is kept yet. An agent keeps what it reads with <code>raw_keep</code>, as research on a schedule asks it to.",
};

function drawEntries(shelf) {
  const list = document.getElementById("pages");
  const entries = entriesOf(shelf);
  if (!entries.length) {
    list.innerHTML = `<div class="empty">${NOTHING[shelf]}</div>`;
    return;
  }
  const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
  const fits = entries.filter((p) => {
    const text = `${p.name} ${p.title} ${p.about ?? ""} ${p.only ?? ""} ${p.source ?? ""} ${p.chat ?? ""}`.toLowerCase();
    return words.every((w) => text.includes(w)) && (!marked || p.marks?.includes(marked));
  });
  if (!fits.length) {
    list.innerHTML = '<div class="empty">No page fits.</div>';
    return;
  }
  list.innerHTML = fits.map((p) => (shelf === "raw" ? sourceEntry(p) : pageEntry(shelf, p))).join("");
}

function pageEntry(shelf, p) {
  const last = p.used.at(-1);
  return (
    `<a class="entry" href="${href(shelf, p.name)}"><div><span class="title">${esc(p.title)}</span><span class="name">${esc(p.name)}</span>${only(p)}${marks(p)}</div>` +
    (p.about ? `<div class="about">${esc(p.about)}</div>` : "") +
    (last ? `<div class="last">used ${esc(last.day)} ${esc(last.what)}${p.used.length > 1 ? ` · ${p.used.length} uses` : ""}</div>` : "") +
    `</a>`
  );
}

/// Where a source was read, short: the host of a page of the web, or
/// what it says.
function from(source) {
  try {
    return new URL(source).host || source;
  } catch {
    return source ?? "";
  }
}

function sourceEntry(s) {
  const kept = [s.kept, s.topic && `in ${s.topic}`, s.chat && `by ${s.chat}`].filter(Boolean).join(" ");
  return (
    `<a class="entry" href="${href("raw", s.name)}"><div><span class="title">${esc(s.title)}</span><span class="name">${esc(from(s.source))}</span></div>` +
    (kept ? `<div class="last">kept ${esc(kept)}</div>` : "") +
    `</a>`
  );
}

function drawPage(shelf, name) {
  const back = `<a href="${href(shelf, "")}">${esc(SHELVES[shelf].label)}</a>`;
  if (!page) {
    app.innerHTML = `${shelves(shelf)}<div class="crumbs">${back} › <span>${esc(name)}</span></div><div class="empty">There is no page <code>${esc(name)}</code> here.</div>`;
    return;
  }
  const entries = entriesOf(shelf);
  const titled = (n) => entries.find((p) => p.name === n)?.title ?? n;
  const link = (n) => `<a href="${href(shelf, n)}">${esc(titled(n))}</a>`;
  const translation = page.translation;
  const translated = translation && !written;
  const raw = shelf === "raw";
  const warned = raw ? "" : page.marks.map((m) => `<li>${esc(MARKS[m]?.[1] ?? m)}${m === "broken" ? `: ${page.broken.map((b) => `<code>${esc(b)}</code>`).join(", ")}` : ""}</li>`).join("");
  const aside = raw
    ? `<div class="aside"><h2>Read at</h2>${source(page.source)}</div>` +
      `<div class="aside"><h2>Kept</h2><div>${esc([page.kept, page.chat && `by ${page.chat}`, page.project && `in ${page.project}`].filter(Boolean).join(" ") || "Nowhere it says.")}</div></div>`
    : `<div class="aside"><h2>Used in</h2>${page.used.length ? `<ul>${page.used.map((u) => `<li><span class="day">${esc(u.day)}</span>${esc(u.what)}</li>`).join("")}</ul>` : '<div class="none">Nowhere it says.</div>'}</div>` +
      `<div class="aside"><h2>Linked from</h2>${page.linked_from.length ? `<ul>${page.linked_from.map((n) => `<li>${link(n)}</li>`).join("")}</ul>` : '<div class="none">No page.</div>'}</div>` +
      (warned ? `<div class="aside"><h2>Cox marks</h2><ul>${warned}</ul></div>` : "");
  app.innerHTML =
    shelves(shelf) +
    `<div class="crumbs">${back} › <span>${esc(page.name)}</span>${only(page)}${raw ? "" : marks(page)}<a class="where" href="cox://open${esc(page.file)}" title="opens the file on a tab">${esc(page.file)}</a></div>` +
    (translation ? languages(translation) : "") +
    `<div class="page">${translated ? translation.html : page.html}</div>` +
    aside;
  for (const chip of app.querySelectorAll("[data-written]")) {
    chip.addEventListener("click", () => {
      written = chip.dataset.written === "yes";
      drawPage(shelf, name);
    });
  }
  const there = new Set(entries.map((p) => p.name));
  for (const a of app.querySelectorAll(".page a[href]")) {
    const to = a.getAttribute("href");
    if (to.startsWith("#")) {
      // A link to a page goes to the page of that name on this shelf.
      const named = decodeURIComponent(to.slice(1));
      a.setAttribute("href", href(shelf, named));
      if (!there.has(named)) a.classList.add("missing");
    } else if (/^[a-z]+:/i.test(to)) {
      // A page of the web gets a tab of its own, beside the wiki.
      a.target = "_blank";
    }
  }
}

/// Where a source was read: a link, on a tab of its own, for a page of
/// the web, or else what it says, as a command.
function source(from) {
  if (!from) return '<div class="none">Nowhere it says.</div>';
  return /^https?:\/\//i.test(from) ? `<a href="${esc(from)}" target="_blank">${esc(from)}</a>` : `<code>${esc(from)}</code>`;
}

/// The chips that choose the language a page is read in, `translation`'s
/// or as it is written, and how far it is translated.
function languages(translation) {
  const state = translation.error
    ? `<span class="translating failed" title="${esc(translation.error)}">Cannot translate: ${esc(translation.error)}</span>`
    : translation.done < translation.of
      ? `<span class="translating">Translating, ${translation.done} of ${translation.of} blocks</span>`
      : "";
  return (
    `<div class="languages">` +
    `<span class="chip ${written ? "" : "on"}" data-written="no">${esc(translation.language)}</span>` +
    `<span class="chip ${written ? "on" : ""}" data-written="yes" title="The page as it is written">As written</span>` +
    `${state}</div>`
  );
}

window.addEventListener("hashchange", () => {
  window.scrollTo(0, 0);
  if (!shown().name) marked = null;
  load();
});
load();
setInterval(load, EVERY);
