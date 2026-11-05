// The human's wiki, drawn by a page of the stage: the pages the agents
// of every project keep of what they learned, read as JSON from
// `cox://api/wiki`, and again every two seconds, as Cox's own tabs follow
// what they show. With no hash, the page lists the pages, which the line
// at its top narrows to those whose names, titles or first lines have
// every word typed, and the chips to those Cox marks: with links to no
// page, linked from no page, with no title, or saying nowhere where they
// were used. A page's name as the hash, as `#rust/errors`, opens it,
// read from `cox://api/wiki/<name>`: its text, its links opening the
// pages they name, the pages that link to it, and where it was used.

const API = "cox://api/wiki";
const EVERY = 2000;

/// What each mark of Cox's says, as a chip and on a page.
const MARKS = {
  broken: ["links to no page", "Links to a page there is not"],
  orphan: ["linked from no page", "No page links here"],
  untitled: ["no title", "Has no # title"],
  unused: ["no Used in", "Says nowhere where it was used"],
};

const app = document.getElementById("app");
let wiki = null;
let wikiText = null;
let page = null;
let pageText = null;
// What the line at the top holds, and the mark chosen, as the human
// left them, across what the page reads again.
let typed = "";
let marked = null;

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const shown = () => decodeURIComponent(location.hash.replace(/^#/, ""));
const href = (name) => `#${name.split("/").map(encodeURIComponent).join("/")}`;
const marks = (p) => p.marks.map((m) => `<span class="mark ${m}" title="${esc(MARKS[m]?.[1] ?? m)}">${esc(MARKS[m]?.[0] ?? m)}</span>`).join("");

async function read(url) {
  try {
    const answer = await fetch(url);
    return answer.ok ? await answer.text() : null;
  } catch {
    return null;
  }
}

async function load() {
  const text = await read(API);
  let changed = text !== wikiText;
  if (changed) {
    wikiText = text;
    wiki = text ? JSON.parse(text) : null;
  }
  const name = shown();
  if (name) {
    const text = await read(`${API}/${name.split("/").map(encodeURIComponent).join("/")}`);
    if (text !== pageText || page?.name !== name) {
      pageText = text;
      page = text ? JSON.parse(text) : null;
      changed = true;
    }
  } else if (page) {
    page = null;
    pageText = null;
    changed = true;
  }
  if (changed) draw();
}

function draw() {
  if (shown()) drawPage();
  else drawList();
}

function drawList() {
  if (!wiki) {
    app.innerHTML = '<div class="empty">Cox cannot tell where the wiki is.</div>';
    return;
  }
  const pages = wiki.pages;
  const counts = Object.keys(MARKS).map((m) => [m, pages.filter((p) => p.marks.includes(m)).length]).filter(([, n]) => n);
  const focused = document.activeElement?.id === "find";
  app.innerHTML =
    `<div class="crumbs"><b>Wiki</b><span>${pages.length} page${pages.length === 1 ? "" : "s"}</span><span class="where">${esc(wiki.root)}</span></div>` +
    `<div class="find"><input id="find" placeholder="Narrow by words" value="${esc(typed)}" spellcheck="false">` +
    counts.map(([m, n]) => `<span class="chip ${marked === m ? "on" : ""}" data-mark="${m}" title="${esc(MARKS[m][1])}"><b>${n}</b> ${esc(MARKS[m][0])}</span>`).join("") +
    `</div><div class="pages" id="pages"></div>`;
  const input = document.getElementById("find");
  input.addEventListener("input", () => {
    typed = input.value;
    drawEntries();
  });
  if (focused) {
    input.focus();
    input.setSelectionRange(typed.length, typed.length);
  }
  for (const chip of app.querySelectorAll("[data-mark]")) {
    chip.addEventListener("click", () => {
      marked = marked === chip.dataset.mark ? null : chip.dataset.mark;
      drawList();
    });
  }
  drawEntries();
}

function drawEntries() {
  const list = document.getElementById("pages");
  if (!wiki.pages.length) {
    list.innerHTML = '<div class="empty">The wiki has no pages yet. An agent writes one with <code>wiki_write</code>, as when the human asks a chat to <b>Save to wiki</b> from the menu of its tab.</div>';
    return;
  }
  const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
  const fits = wiki.pages.filter((p) => {
    const text = `${p.name} ${p.title} ${p.about ?? ""}`.toLowerCase();
    return words.every((w) => text.includes(w)) && (!marked || p.marks.includes(marked));
  });
  list.innerHTML = fits.length
    ? fits
        .map((p) => {
          const last = p.used.at(-1);
          return (
            `<a class="entry" href="${href(p.name)}"><div><span class="title">${esc(p.title)}</span><span class="name">${esc(p.name)}</span>${marks(p)}</div>` +
            (p.about ? `<div class="about">${esc(p.about)}</div>` : "") +
            (last ? `<div class="last">used ${esc(last.day)} ${esc(last.what)}${p.used.length > 1 ? ` · ${p.used.length} uses` : ""}</div>` : "") +
            `</a>`
          );
        })
        .join("")
    : '<div class="empty">No page fits.</div>';
}

function drawPage() {
  const name = shown();
  if (!page) {
    app.innerHTML = `<div class="crumbs"><a href="#">Wiki</a> › <span>${esc(name)}</span></div><div class="empty">The wiki has no page <code>${esc(name)}</code>.</div>`;
    return;
  }
  const titled = (n) => wiki?.pages.find((p) => p.name === n)?.title ?? n;
  const link = (n) => `<a href="${href(n)}">${esc(titled(n))}</a>`;
  const warned = page.marks.map((m) => `<li>${esc(MARKS[m]?.[1] ?? m)}${m === "broken" ? `: ${page.broken.map((b) => `<code>${esc(b)}</code>`).join(", ")}` : ""}</li>`).join("");
  app.innerHTML =
    `<div class="crumbs"><a href="#">Wiki</a> › <span>${esc(page.name)}</span>${marks(page)}<a class="where" href="cox://open${esc(page.file)}" title="opens the file on a tab">${esc(page.file)}</a></div>` +
    `<div class="page">${page.html}</div>` +
    `<div class="aside"><h2>Used in</h2>${page.used.length ? `<ul>${page.used.map((u) => `<li><span class="day">${esc(u.day)}</span>${esc(u.what)}</li>`).join("")}</ul>` : '<div class="none">Nowhere it says.</div>'}</div>` +
    `<div class="aside"><h2>Linked from</h2>${page.linked_from.length ? `<ul>${page.linked_from.map((n) => `<li>${link(n)}</li>`).join("")}</ul>` : '<div class="none">No page.</div>'}</div>` +
    (warned ? `<div class="aside"><h2>Cox marks</h2><ul>${warned}</ul></div>` : "");
  const there = new Set(wiki?.pages.map((p) => p.name) ?? []);
  for (const a of app.querySelectorAll(".page a[href]")) {
    const to = a.getAttribute("href");
    if (to.startsWith("#")) {
      if (!there.has(decodeURIComponent(to.slice(1)))) a.classList.add("missing");
    } else if (/^[a-z]+:/i.test(to)) {
      // A page of the web gets a tab of its own, beside the wiki.
      a.target = "_blank";
    }
  }
}

window.addEventListener("hashchange", () => {
  window.scrollTo(0, 0);
  load();
});
load();
setInterval(load, EVERY);
