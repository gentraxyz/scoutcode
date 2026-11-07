// In the extension's own world on chat.deepseek.com: links the page (main.js) to the background worker, which talks
// to Scout.
(() => {
  let port = null;
  let dead = false;
  // After the extension is reloaded or updated, this copy in an already open tab is cut off for good. It stops quietly;
  // refreshing the tab starts the new one.
  const alive = () => {
    try { return !!chrome.runtime && !!chrome.runtime.id; } catch { return false; }
  };
  const connect = () => {
    if (dead || !alive()) { dead = true; return; }
    try {
      port = chrome.runtime.connect({ name: 'deepseek' });
    } catch {
      dead = true;
      return;
    }
    port.onMessage.addListener((m) => window.postMessage({ __scoutDSIn: m }, location.origin));
    // Asks the page for its status, so the worker knows it as soon as the port is up.
    window.postMessage({ __scoutDSIn: { type: 'hello' } }, location.origin);
    port.onDisconnect.addListener(() => {
      // Read, so Chrome doesn't report it: the port closes whenever the page is cached or the worker restarts.
      void chrome.runtime.lastError;
      port = null;
      if (alive()) setTimeout(connect, 1000);
      else dead = true;
    });
  };
  connect();
  // A page brought back from the back/forward cache lost its port while it was cached.
  window.addEventListener('pageshow', (e) => {
    if (e.persisted && !port && !dead) connect();
  });
  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || !e.data.__scoutDSOut || !port) return;
    try { port.postMessage(e.data.__scoutDSOut); } catch {}
  });
  // Keeps the background worker awake while the tab is open.
  const ping = setInterval(() => {
    if (dead) return clearInterval(ping);
    try { port && port.postMessage({ type: 'ping' }); } catch {}
  }, 20000);
})();
