// In the page's own world, next to driver.js: passes the driver's events out to bridge.js, and Scout's commands in.
(() => {
  const api = window.__scoutDS;
  if (!api || api.bridged) return;
  api.bridged = true;
  const out = (msg) => window.postMessage({ __scoutDSOut: msg }, location.origin);
  // Sent whenever it changes (the page signs in, a request moves on), and in full when the bridge asks, since a
  // status sent before the bridge was listening, or before the worker restarted, is lost.
  let last = '';
  const status = (force) => {
    const s = api.status();
    const json = JSON.stringify(s);
    if (!force && json === last) return;
    last = json;
    out({ type: 'status', status: s });
  };
  setInterval(status, 1000);
  // Anything the driver said before this was set up, such as after a chat opened.
  for (const e of api.drain().events) out({ type: 'event', event: e });
  api.onEvent = (e) => out({ type: 'event', event: e });
  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || !e.data.__scoutDSIn) return;
    const m = e.data.__scoutDSIn;
    if (m.type === 'start') api.start(m.job);
    else if (m.type === 'stop') api.stop();
    else if (m.type === 'tick') api.tick();
    status(m.type === 'hello');
  });
  status();
})();
