// Talks to the Scout desktop app on this PC, through its local server, and does the Claude connector setup's real clicks
// and typing. scout.json, which Scout writes into this folder, has the port and the token.

async function config() {
  const r = await fetch(chrome.runtime.getURL('scout.json'), { cache: 'no-store' });
  return r.json();
}

// Says what went wrong in words a person can act on.
function explain(e) {
  const m = String(e && e.message ? e.message : e);
  if (/Failed to fetch|NetworkError|ERR_CONNECTION/i.test(m)) return "Can't reach Scout. Is the desktop app open?";
  if (/JSON|scout.json/i.test(m)) return 'The extension is missing its Scout settings. Set it up again from Scout.';
  return m;
}

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
