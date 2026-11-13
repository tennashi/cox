globalThis.coxVim ??= (() => {
  const CLICKABLE = 'a[href],button,input:not([type=hidden]),textarea,select,summary,label,'
    + '[role=button],[role=link],[role=tab],[role=checkbox],[role=menuitem],[onclick],'
    + '[contenteditable=""],[contenteditable=true],[tabindex]:not([tabindex="-1"])';
  const TYPELESS = ['button', 'submit', 'checkbox', 'radio', 'reset', 'image', 'file', 'range',
    'color', 'hidden'];
  const isField = (element) => element.isContentEditable || element.tagName === 'TEXTAREA'
    || (element.tagName === 'INPUT' && !TYPELESS.includes(element.type));
  let found = [];
  let layer = null;

  const walk = (root, offset, into) => {
    for (const element of root.querySelectorAll(CLICKABLE)) into.push([element, offset]);
    for (const host of root.querySelectorAll('*')) {
      if (host.shadowRoot) walk(host.shadowRoot, offset, into);
    }
    for (const frame of root.querySelectorAll('iframe')) {
      let inner = null;
      try { inner = frame.contentDocument; } catch (error) { inner = null; }
      if (inner) {
        const rect = frame.getBoundingClientRect();
        walk(inner, { x: offset.x + rect.left, y: offset.y + rect.top }, into);
      } else {
        into.push([frame, offset]);
      }
    }
  };

  // Where `element` is in sight, in the top page's coordinates: the first
  // of the boxes it is laid out in (a link that wraps a heading and a line
  // under it is more than one) whose middle shows the element and not
  // something over it, or null.
  const boxIn = (element, offset) => {
    if (getComputedStyle(element).visibility === 'hidden') return null;
    const root = element.getRootNode();
    const finder = root.elementFromPoint ? root : element.ownerDocument;
    for (const box of element.getClientRects()) {
      const rect = { x: offset.x + box.left, y: offset.y + box.top, width: box.width, height: box.height };
      if (rect.width < 2 || rect.height < 2) continue;
      if (rect.y + rect.height < 0 || rect.x + rect.width < 0) continue;
      if (rect.y > innerHeight || rect.x > innerWidth) continue;
      if (element.tagName === 'IFRAME') return rect;
      const top = finder.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      if (!top || top === element || element.contains(top) || top.contains(element)
        || (element.labels && [...element.labels].includes(top))) return rect;
    }
    return null;
  };

  const kindOf = (element) => {
    if (isField(element)) return { kind: 'field', href: null };
    if (element.tagName === 'A' && element.href) return { kind: 'link', href: element.href };
    return { kind: 'other', href: null };
  };

  const deepActive = () => {
    let active = document.activeElement;
    for (;;) {
      if (active && active.shadowRoot && active.shadowRoot.activeElement) {
        active = active.shadowRoot.activeElement;
      } else if (active && active.tagName === 'IFRAME') {
        let inner = null;
        try { inner = active.contentDocument; } catch (error) { inner = null; }
        if (!inner || !inner.activeElement || inner.activeElement === inner.body) return active;
        active = inner.activeElement;
      } else {
        return active;
      }
    }
  };

  const scroller = () => {
    let element = document.elementFromPoint(innerWidth / 2, innerHeight / 2);
    while (element) {
      const style = getComputedStyle(element);
      if (element.scrollHeight > element.clientHeight
        && ['auto', 'scroll', 'overlay'].includes(style.overflowY)) return element;
      element = element.parentElement || element.getRootNode().host || null;
    }
    return document.scrollingElement || document.documentElement;
  };

  return {
    hints() {
      const all = [];
      walk(document, { x: 0, y: 0 }, all);
      found = all.map(([element, offset]) => [element, boxIn(element, offset)])
        .filter(([, rect]) => rect);
      return found.map(([element, rect], id) => ({
        id,
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        ...kindOf(element),
        text: (element.innerText || element.value || element.getAttribute('aria-label') || '')
          .trim().slice(0, 80),
      }));
    },
    show(labels, look) {
      this.clear();
      const host = document.createElement('div');
      host.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647';
      const shadow = host.attachShadow({ mode: 'closed' });
      for (const [id, label] of labels) {
        const [, rect] = found[id] ?? [];
        if (!rect) continue;
        const tag = document.createElement('span');
        tag.textContent = label;
        tag.style.cssText = `position:fixed;left:${Math.max(0, rect.x)}px;top:${Math.max(0, rect.y)}px;`
          + `box-sizing:border-box;${look}`;
        shadow.appendChild(tag);
      }
      document.documentElement.appendChild(host);
      layer = host;
      return labels.length;
    },
    clear() {
      layer?.remove();
      layer = null;
      return true;
    },
    focus(id) {
      const entry = found[id];
      if (!entry) return false;
      entry[0].focus();
      return true;
    },
    field() {
      const fields = [...document.querySelectorAll('input,textarea,[contenteditable]')]
        .filter((element) => isField(element) && element.getClientRects().length > 0
          && getComputedStyle(element).visibility !== 'hidden' && !element.disabled);
      if (!fields.length) return false;
      fields[0].scrollIntoView({ block: 'center' });
      fields[0].focus();
      return true;
    },
    blur() {
      const active = deepActive();
      if (!active || active === document.body) return false;
      active.blur();
      return true;
    },
    scroll(bottom) {
      const element = scroller();
      element.scrollTop = bottom ? element.scrollHeight : 0;
      return element.scrollTop;
    },
  };
})();
