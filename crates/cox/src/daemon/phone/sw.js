// The phone page's service worker: shows what coxd pushes of the chats
// that need the human, and shows the chat when the human taps it.

"use strict";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

// What coxd pushes: { key, title, body }, a chat and what it came to. A
// chat's notice takes the place of the one it pushed before.
self.addEventListener("push", (event) => {
  let told = {};
  try {
    told = event.data ? event.data.json() : {};
  } catch (_) {
    told = {};
  }
  const key = typeof told.key === "number" ? told.key : null;
  event.waitUntil(self.registration.showNotification(told.title || "Cox", {
    body: told.body || "",
    tag: key === null ? "cox" : `chat-${key}`,
    data: { key },
  }));
});

// The chat a tap asks to show, kept for the page to read as it comes to
// the front: an iPhone does not hand a page put away what is posted to
// it, nor open the address asked, but the app as it was.
const TAPPED = "/tapped";

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const key = event.notification.data && event.notification.data.key;
  const hash = typeof key === "number" ? `#chat/${key}` : "";
  event.waitUntil((async () => {
    if (hash) {
      const kept = await caches.open("cox");
      await kept.put(TAPPED, new Response(JSON.stringify({ show: hash, at: Date.now() })));
    }
    const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const page of pages) {
      page.postMessage({ show: hash });
      if ("focus" in page) await page.focus().catch(() => {});
      return;
    }
    await self.clients.openWindow(`/${hash}`);
  })());
});
