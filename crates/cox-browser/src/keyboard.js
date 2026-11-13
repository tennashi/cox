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

  const rectOf = ([element, offset]) => {
    const rect = element.getBoundingClientRect();
    return { x: offset.x + rect.left, y: offset.y + rect.top, width: rect.width, height: rect.height };
  };

  const shown = ([element, offset]) => {
    const rect = rectOf([element, offset]);
    if (rect.width < 2 || rect.height < 2) return false;
    if (rect.y + rect.height < 0 || rect.x + rect.width < 0) return false;
    if (rect.y > innerHeight || rect.x > innerWidth) return false;
    if (getComputedStyle(element).visibility === 'hidden') return false;
    if (element.tagName === 'IFRAME') return true;
    const local = element.getBoundingClientRect();
    const root = element.getRootNode();
    const top = (root.elementFromPoint ? root : element.ownerDocument)
      .elementFromPoint(local.left + local.width / 2, local.top + local.height / 2);
    return !top || top === element || element.contains(top) || top.contains(element)
      || (element.labels && [...element.labels].includes(top));
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
      found = all.filter(shown);
      return found.map((entry, id) => {
        const rect = rectOf(entry);
        return {
          id,
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
          ...kindOf(entry[0]),
          text: (entry[0].innerText || entry[0].value || entry[0].getAttribute('aria-label') || '')
            .trim().slice(0, 80),
        };
      });
    },
    show(labels) {
      this.clear();
      const host = document.createElement('div');
      host.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647';
      const shadow = host.attachShadow({ mode: 'closed' });
      for (const [id, label] of labels) {
        const entry = found[id];
        if (!entry) continue;
        const rect = rectOf(entry);
        const tag = document.createElement('span');
        tag.textContent = label;
        tag.style.cssText = `position:fixed;left:${Math.max(0, rect.x)}px;top:${Math.max(0, rect.y)}px;`
          + 'background:#fd0;color:#000;font:bold 11px/1.2 monospace;padding:0 2px;'
          + 'border:1px solid #a80;border-radius:2px;text-transform:uppercase';
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
