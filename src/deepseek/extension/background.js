// Talks to the Scout desktop app on this PC, through its local server, and runs its requests on a chat.deepseek.com tab.
// scout.json, which Scout writes into this folder, has the port and the token.

const tabs = new Map(); // tab id -> { port, status, seen }
let jobTab = null; // The tab running Scout's current request.
let events = [];
let running = false;
let state = { connected: false, error: null };

async function config() {
  const r = await fetch(chrome.runtime.getURL('scout.json'), { cache: 'no-store' });
  return r.json();
}

function bestTab() {
  let best = null;
  for (const [id, t] of tabs) {
    const score = (t.status && t.status.signedIn ? 2 : 0) + t.seen / 1e13;
    if (!best || score > best.score) best = { id, score };
  }
  return best && best.id;
}

// The toolbar icon: DeepSeek's whale, with a dot for the state. None: no DeepSeek tab (or still connecting), red: an
// error or signed out, green: connected.
const WHALE = new Path2D('M19.6 8.3c-.3-.1-.5.1-.7.3l-.2.2c-.4.4-.9.7-1.5.7-.9 0-1.6.3-2.2.9-.1-.8-.5-1.2-1.2-1.5-.4-.2-.7-.4-1-.7-.2-.3-.2-.6-.3-.9 0-.2 0-.4-.3-.4-.2 0-.3.2-.4.4-.4.7-.5 1.5-.5 2.3.1 1.8.8 3.2 2.3 4.2.2.1.2.2.2.4l-.3 1c-.1.2-.1.3-.4.2-.8-.4-1.5-.9-2.1-1.5-1-1-1.9-2.1-3.1-2.9l-.8-.5c-1.1-1.1.1-2 .4-2.1.3-.1.1-.5-.9-.5S4.6 8.7 4.3 8.8c-.4-.1-.8-.1-1.3-.1-.6 0-1.1.2-1.6.4.7-.1.5.4.4.6-.3.9-.2 2.1.2 3.1.6 1.6 1.7 2.9 3.2 3.9 1.6 1.1 3.4 1.4 5.3 1.3 1.1-.1 2.4-.2 3.9-1.5.4.2.8.2 1.4.2.5 0 .9-.1 1.2-.1.5-.1.5-.6.3-.7-1.5-.7-1.2-.4-1.5-.6.7-.9 1.8-1.8 2.3-4.7 0-.2 0-.4.1-.4.3-.1.7-.1 1.1 0 .9.5 1.8.1 1.9-.5 0-.5-.6-.9-1.6-1.4z');
const DOTS = { error: '#ef4452', signin: '#ef4452', ready: '#22c55e', working: '#22c55e' };
let painted;
let saved = null;

function draw(size, dot) {
  const c = new OffscreenCanvas(size, size);
  const g = c.getContext('2d');
  g.scale(size / 24, size / 24);
  g.fillStyle = '#4D6BFE';
  g.beginPath();
  g.roundRect(0, 0, 24, 24, 6);
  g.fill();
  g.fillStyle = '#fff';
  g.fill(WHALE);
  if (dot) {
    // Cut a ring out first, so the dot stands apart from the icon on any toolbar.
    g.globalCompositeOperation = 'destination-out';
    g.beginPath();
    g.arc(18.5, 18.5, 6.5, 0, Math.PI * 2);
    g.fill();
    g.globalCompositeOperation = 'source-over';
    g.fillStyle = dot;
    g.beginPath();
    g.arc(18.5, 18.5, 4.6, 0, Math.PI * 2);
    g.fill();
  }
  return g.getImageData(0, 0, size, size);
}

function paint(phase) {
  const dot = DOTS[phase] || null;
  if (painted === dot) return;
  painted = dot;
  try {
    chrome.action.setIcon({ imageData: { 16: draw(16, dot), 32: draw(32, dot) } }).catch(() => {});
  } catch {}
  const title = { none: 'DeepSeek not detected', connecting: 'Connecting…', error: 'Error', signin: 'Sign in to DeepSeek', ready: 'Connected', working: 'Working' }[phase];
  chrome.action.setTitle({ title: `Scout: ${title}` }).catch(() => {});
}

function phaseOf(s) {
  if (!s.tabs) return 'none';
  if (s.error) return 'error';
  if (!s.connected || !s.reported) return 'connecting';
  if (!s.signedIn) return 'signin';
  return s.busy ? 'working' : 'ready';
}

function save() {
  const id = bestTab();
  const t = tabs.get(id);
  const st = t && t.status;
  const s = { ...state, tabs: tabs.size, tabId: id || null, reported: !!st, signedIn: !!(st && st.signedIn), busy: !!(st && st.busy) || !!jobTab, stage: st && st.stage !== 'idle' ? st.stage : null };
  s.phase = phaseOf(s);
  paint(s.phase);
  // The loop runs many times a second; only write when something changed, so an open popup isn't redrawn for nothing.
  const json = JSON.stringify(s);
  if (json === saved) return;
  saved = json;
  chrome.storage.session.set({ state: s }).catch(() => {});
}

// Says what went wrong in words a person can act on.
function explain(e) {
  const m = String(e && e.message ? e.message : e);
  if (/Failed to fetch|NetworkError|ERR_CONNECTION/i.test(m)) return "Can't reach Scout. Is the desktop app open?";
  if (/JSON|scout.json/i.test(m)) return 'The extension is missing its Scout settings. Set it up again from Scout.';
  return m;
}

paint('none');
save();
chrome.runtime.onStartup.addListener(save);
chrome.runtime.onInstalled.addListener(save);

// Claude connector setup: real input. Clicks and keys sent from inside a page don't count as the user's, and Claude's
// form ignores them, so after the user approves a step claude.js asks for it here. Chrome's debugger sends it to the tab
// as real mouse and keyboard input, and shows its "started debugging this browser" bar while attached. Only claude.ai
// tabs, and only clicks, plain text and Tab; detached when the setup ends.
const debugging = new Set();
chrome.debugger.onDetach.addListener((src) => debugging.delete(src.tabId));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function realInput(tab, url, m) {
  if (!/^https:\/\/(claude\.ai\/|gemini\.google\.com\/|scoutcode\.dev\/oauth\/authorize)/.test(url || '')) throw new Error('Only on claude.ai, Gemini and ScoutCode\'s consent page.');
  const target = { tabId: tab.id };
  if (m.kind === 'release') {
    if (debugging.delete(tab.id)) await chrome.debugger.detach(target).catch(() => {});
    return { ok: true };
  }
  if (!debugging.has(tab.id)) {
    await chrome.debugger.attach(target, '1.3');
    debugging.add(tab.id);
  }
  const send = (method, params) => chrome.debugger.sendCommand(target, method, params);
  if (m.kind === 'click') {
    const at = { x: Math.round(m.x), y: Math.round(m.y) };
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...at });
    await wait(60);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...at, button: 'left', buttons: 1, clickCount: 1 });
    await wait(70);
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...at, button: 'left', buttons: 0, clickCount: 1 });
  } else if (m.kind === 'text') {
    await send('Input.insertText', { text: String(m.text).slice(0, 500) });
  } else if (m.kind === 'tab') {
    const key = { key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 };
    await send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...key });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', ...key });
  }
  return { ok: true };
}

chrome.runtime.onMessage.addListener((m, sender, reply) => {
  if (!m || !m.scoutInput || !sender.tab) return;
  realInput(sender.tab, sender.url, m.scoutInput).then(reply, (e) => reply({ error: String(e && e.message || e) }));
  return true;
});

// Claude connector setup (claude.js): passes a claude.ai tab's reports to Scout, and Scout's job back.
chrome.runtime.onMessage.addListener((m, sender, reply) => {
  if (!m || !m.scoutClaude || !sender.tab) return;
  (async () => {
    const cfg = await config();
    const r = await fetch(`http://127.0.0.1:${cfg.port}/claude/sync`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Scout-Token': cfg.token },
      body: JSON.stringify(m.scoutClaude),
    });
    if (!r.ok) throw new Error(`Scout answered ${r.status}`);
    return r.json();
  })().then(reply, (e) => reply({ job: null, error: explain(e) }));
  return true;
});

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== 'deepseek' || !port.sender || !port.sender.tab) return;
  const id = port.sender.tab.id;
  tabs.set(id, { port, status: null, seen: Date.now() });
  save();
  port.onMessage.addListener((m) => {
    const t = tabs.get(id);
    if (!t) return;
    t.seen = Date.now();
    if (m.type === 'status') { t.status = m.status; save(); }
    else if (m.type === 'event') events.push(m.event);
  });
  port.onDisconnect.addListener(() => {
    // Read, so Chrome doesn't report it: a tab's port closes whenever it opens another chat or goes into the cache.
    void chrome.runtime.lastError;
    if (tabs.get(id) && tabs.get(id).port === port) { tabs.delete(id); save(); }
  });
  loop();
});

async function loop() {
  if (running) return;
  running = true;
  let cfg = null;
  while (tabs.size > 0) {
    try {
      // Read again between requests, so it follows the Scout that started last (each writes its port here).
      if (!cfg || !jobTab) cfg = await config();
      const id = jobTab && tabs.has(jobTab) ? jobTab : bestTab();
      const t = tabs.get(id);
      // While a request runs, the page is nudged along, since a background tab's timers are slow.
      if (jobTab && t) try { t.port.postMessage({ type: 'tick' }); } catch {}
      const body = { status: t ? t.status : null, tabs: tabs.size, events: events.splice(0) };
      const r = await fetch(`http://127.0.0.1:${cfg.port}/deepseek/sync`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-Scout-Token': cfg.token },
        body: JSON.stringify(body),
      });
      if (r.status === 403) throw new Error('Scout refused the token. Set up the extension again from Scout.');
      const { commands } = await r.json();
      state = { connected: true, error: null };
      for (const c of commands || []) {
        if (c.type === 'start') jobTab = id;
        // A hold can come before any job, so it goes to the tab the next job will use.
        const target = tabs.get(jobTab || id);
        if (target) try { target.port.postMessage(c); } catch {}
      }
      if (body.events.some((e) => e.kind === 'done' || e.kind === 'error')) jobTab = null;
      save();
      await new Promise((r) => setTimeout(r, jobTab ? 150 : 50));
    } catch (e) {
      cfg = null;
      state = { connected: false, error: explain(e) };
      save();
      await new Promise((r) => setTimeout(r, 3000));
    }
  }
  running = false;
  state = { connected: false, error: null };
  save();
}
