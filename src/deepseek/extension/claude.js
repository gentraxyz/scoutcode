// On claude.ai or Gemini: adds Scout to the chatbot as a custom connector, when Scout asked for it (see claude_setup.rs).
//
// It only sets the connector up, never chats. Once the user presses Start in Scout it runs the steps by itself, with a
// cursor that moves across the page to each button and field, and a Pause button. It works through the page's own buttons
// and fields, the way a mouse and keyboard would; it runs no code in the page.
(() => {
  if (window.top !== window || window.__scoutClaude) return;
  window.__scoutClaude = true;

  // This tab's id, kept across reloads, so Scout gives the job to one tab only.
  let tab;
  try {
    tab = sessionStorage.getItem('scoutSetupTab');
    if (!tab) { tab = Math.random().toString(36).slice(2) + Date.now().toString(36); sessionStorage.setItem('scoutSetupTab', tab); }
  } catch { tab = Math.random().toString(36).slice(2); }

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const alive = () => { try { return !!chrome.runtime && !!chrome.runtime.id; } catch { return false; } };

  /** Tells Scout what happened and hears whether this tab has a job. */
  async function talk(events = []) {
    if (!alive()) return null;
    try { return await chrome.runtime.sendMessage({ scoutClaude: { tab, bot: BOT, events } }); } catch { return null; }
  }

  // ---------- finding things on the page ----------

  /** Which chatbot this page is. Scout only gives a tab a job for its own chatbot. */
  const GEMINI = location.hostname === 'gemini.google.com';
  const BOT = GEMINI ? 'gemini' : 'claude';
  /** Its name in what Scout shows; on ScoutCode's consent page, the chatbot linking. */
  let SITE = GEMINI ? 'Gemini' : 'Claude';
  /** The connector settings: Gemini's Customize → Connectors; Claude's view over the new-chat page, or the old settings page. */
  const CONNECTORS_PATH = GEMINI ? '/customize/connectors' : '/new#customize/connectors';
  const onConnectors = () => GEMINI
    ? location.pathname.startsWith('/customize/connectors')
    : /customize\/connectors/.test(location.hash) || location.pathname.startsWith('/settings/connectors');

  const visible = (el) => {
    if (!el || !el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && !el.closest('[aria-hidden="true"]') && !el.disabled;
  };
  const words = (el) => (el.innerText || el.textContent || el.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim();
  const clickables = (root = document) => [...root.querySelectorAll('button, a[href], [role="button"], [role="menuitem"], [role="option"]')].filter(visible);
  /** The first clickable whose words match one of the patterns, trying each pattern in turn. */
  const byText = (patterns, root) => {
    const all = clickables(root);
    for (const p of patterns) {
      const hit = all.find((el) => p.test(words(el)) || p.test(el.getAttribute('aria-label') || ''));
      if (hit) return hit;
    }
    return null;
  };
  const dialog = () => [...document.querySelectorAll('[role="dialog"], [role="alertdialog"], dialog[open]')].filter(visible).pop() || null;
  const fields = (root) => [...root.querySelectorAll('input:not([type]), input[type="text"], input[type="url"], textarea')].filter(visible);
  /** A field in the open dialog, by its label, placeholder or name. */
  function field(pattern, fallback) {
    // The add-connector form: the top dialog with a name and an address field, not the connectors view's search box.
    const d = [...document.querySelectorAll('[role="dialog"], [role="alertdialog"], dialog[open]')].filter((x) => visible(x) && fields(x).length >= 2).pop();
    if (!d) return null;
    const all = fields(d);
    // Claude's form is just a name then an address, with no labels to go by.
    if (all.length === 2) return all[fallback];
    const hint = (el) => {
      const id = el.id && d.querySelector(`label[for="${CSS.escape(el.id)}"]`);
      // A label around the whole form would make every field look like every other.
      const label = el.closest('label');
      const own = label && fields(label).length === 1 ? label : null;
      return [el.placeholder, el.name, el.getAttribute('aria-label'), id && words(id), own && words(own)].filter(Boolean).join(' ');
    };
    return all.find((el) => pattern.test(hint(el))) || all[fallback] || null;
  }
  const nameField = () => field(/\bname\b/i, 0);
  /** The form's save button, Continue now, whether or not it's enabled yet. */
  function saveButton() {
    const f = nameField();
    const d = f && f.closest('[role="dialog"], [role="alertdialog"], dialog, form');
    if (!d) return null;
    const shown = [...d.querySelectorAll('button')].filter((b) => { const r = b.getBoundingClientRect(); return r.width > 1 && r.height > 1; });
    for (const p of [/^continue$/i, /^add$/i, /^add connector$/i, /^save$/i, /^create$/i]) {
      const hit = shown.find((b) => p.test(words(b)));
      if (hit) return hit;
    }
    return shown.find((b) => b.type === 'submit') || null;
  }
  const urlField = () => field(/url|https?:|server|address/i, 1);

  // ---------- real mouse and keyboard ----------
  //
  // Each step's clicks and keys go through the background worker and Chrome's debugger, as real input: the page can't
  // tell it from the user's own. The events further down are only for when that isn't available.

  const real = async (m) => {
    try { return (await chrome.runtime.sendMessage({ scoutInput: m })) || { error: 'No answer' }; } catch (e) { return { error: String(e) }; }
  };
  let attached = null; // null: not tried yet, true or false after.
  /** Attaches once; Chrome's bar can push the page down as it appears, so positions are read after it settles. */
  let attachError = '';
  async function canReal() {
    if (attached === null) {
      // Another extension's frame on the page makes Chrome refuse; it's sometimes only there for a moment.
      for (let n = 0; n < 3 && attached !== true; n++) {
        if (n) await sleep(1200);
        const r = await real({ kind: 'attach' });
        attached = !r.error;
        attachError = r.error || '';
      }
      if (!attached) console.warn('Scout setup: real input unavailable,', attachError);
      await sleep(600);
    }
    return attached;
  }
  /** Why Chrome won't let Scout type here, in words a person can act on. */
  const blockedWhy = () => /different extension/i.test(attachError)
    ? 'Chrome won\'t let Scout type on this page, because another browser extension has added something to it (often an ad blocker, password manager or another AI extension).'
    : `Chrome won't let Scout type on this page (${esc(attachError || 'no reason given')}).`;
  const release = () => { if (attached) real({ kind: 'release' }); attached = null; };

  /** Clicks the middle of the element with the real mouse. */
  async function click(el) {
    if (!(await canReal())) return press(el);
    el.scrollIntoView({ block: 'center', inline: 'nearest' });
    await sleep(150);
    const r = el.getBoundingClientRect();
    // Where the on-screen cursor is, when that's on the element: what you see is where it clicks.
    const c = ui.at();
    const on = c.x > r.left + 2 && c.x < r.right - 2 && c.y > r.top + 2 && c.y < r.bottom - 2;
    ui.hidePanel();
    ui.tap();
    const res = await real({ kind: 'click', x: on ? c.x : r.left + r.width / 2, y: on ? c.y : r.top + r.height / 2 });
    ui.showPanel();
    if (res.error) press(el);
  }

  /** Clicks into the field, selects what's there and types over it a key at a time, then tabs out. */
  async function type(el, text) {
    if (!(await canReal())) return typeEvents(el, text);
    await click(el);
    await sleep(100);
    el.select && el.select();
    for (const ch of text) {
      await gate();
      if (stopped) return;
      const r = await real({ kind: 'text', text: ch });
      if (r.error) return typeEvents(el, text);
      await sleep(keyGap(ch));
    }
    await sleep(250 + Math.random() * 200);
    await real({ kind: 'tab' });
  }

  /** The pause after a key: an unhurried pace that varies, a bit longer after a space or punctuation, and now and then
   * a longer stop. Only the timing varies; every key typed is the text's own. */
  function keyGap(ch) {
    let ms = 110 + Math.random() * 130;
    if (/[\s./:@-]/.test(ch)) ms += 80 + Math.random() * 160;
    if (Math.random() < 0.06) ms += 300 + Math.random() * 400;
    return ms;
  }

  // ---------- Pause ----------

  let paused = false;
  /** Waits here while the user has the setup paused. */
  async function gate() {
    while (paused && !stopped) await sleep(200);
  }

  // ---------- in-page events, when real input isn't available ----------

  function press(el) {
    el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    const r = el.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, button: 0 };
    const ptr = { ...at, pointerId: 1, pointerType: 'mouse', isPrimary: true };
    el.dispatchEvent(new PointerEvent('pointerover', ptr));
    el.dispatchEvent(new MouseEvent('mouseover', at));
    el.dispatchEvent(new PointerEvent('pointermove', ptr));
    el.dispatchEvent(new MouseEvent('mousemove', at));
    el.dispatchEvent(new PointerEvent('pointerdown', { ...ptr, buttons: 1 }));
    el.dispatchEvent(new MouseEvent('mousedown', { ...at, buttons: 1 }));
    if (typeof el.focus === 'function') el.focus({ preventScroll: true });
    el.dispatchEvent(new PointerEvent('pointerup', ptr));
    el.dispatchEvent(new MouseEvent('mouseup', at));
    el.dispatchEvent(new MouseEvent('click', at));
  }

  async function typeEvents(el, text) {
    press(el);
    el.focus();
    el.select && el.select();
    if (el.value && !document.execCommand('delete')) setValue(el, '');
    if (el.value) setValue(el, '');
    let want = '';
    for (const ch of text) {
      want += ch;
      el.dispatchEvent(new KeyboardEvent('keydown', { key: ch, bubbles: true }));
      if (!document.execCommand('insertText', false, ch)) setValue(el, want, ch);
      el.dispatchEvent(new KeyboardEvent('keyup', { key: ch, bubbles: true }));
      // The page can move the caret or reformat what's there; keep the field to what's been typed so far.
      if (el.value !== want) setValue(el, want, ch);
      await gate();
      await sleep(keyGap(ch));
    }
    if (el.value !== text) setValue(el, text);
    el.dispatchEvent(new Event('change', { bubbles: true }));
    // Leave the field, like pressing Tab: Claude's form checks a field when it's left, and keeps Continue greyed out
    // until it has.
    await sleep(120);
    el.blur();
  }
  function setValue(el, v, ch) {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: ch || null, inputType: ch ? 'insertText' : 'deleteContentBackward' }));
  }

  // ---------- the overlay: a cursor, a ring around the target, and the Approve panel ----------

  const ui = (() => {
    let host, root, cursor, ring, panel, target = null, frame = 0;
    let pos = { x: innerWidth - 80, y: innerHeight - 80 };
    function mount() {
      if (host && host.isConnected) return;
      host = document.createElement('scout-setup');
      host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none';
      root = host.attachShadow({ mode: 'closed' });
      root.innerHTML = `<style>
        :host { all: initial; }
        * { box-sizing: border-box; font-family: system-ui, -apple-system, 'Segoe UI', sans-serif; }
        .cursor { position: fixed; left: 0; top: 0; width: 26px; height: 26px; filter: drop-shadow(0 2px 4px rgba(0,0,0,.35)); }
        .cursor.down svg { transform: scale(.82); }
        .cursor svg { transition: transform 120ms; transform-origin: 3px 3px; }
        .cursor .tag { position: absolute; left: 20px; top: 20px; background: #4d6bfe; color: #fff; font-size: 11px; font-weight: 600;
          padding: 2px 7px; border-radius: 99px; white-space: nowrap; }
        .ring { position: fixed; border: 2px solid #4d6bfe; border-radius: 10px; box-shadow: 0 0 0 4px rgba(77,107,254,.22);
          transition: all 200ms; opacity: 0; }
        .panel { position: fixed; width: 340px; pointer-events: auto; background: #16171c; color: #e8e9ee; border: 1px solid #2a2c35;
          border-radius: 12px; padding: 12px; box-shadow: 0 12px 32px rgba(0,0,0,.45); font-size: 13px; line-height: 1.4;
          transition: left 300ms, top 300ms; }
        .head { display: flex; align-items: center; gap: 8px; color: #8a8e9b; font-size: 11.5px; margin-bottom: 6px; }
        .head b { color: #e8e9ee; font-weight: 600; flex: 1; }
        .text { margin-bottom: 10px; }
        .text code { background: #23252d; padding: 1px 5px; border-radius: 5px; font-family: ui-monospace, Consolas, monospace; font-size: 12px;
          word-break: break-all; }
        .row { display: flex; gap: 6px; }
        button { font: inherit; font-size: 12.5px; font-weight: 600; border: 0; border-radius: 8px; padding: 7px 10px; cursor: pointer;
          background: #23252d; color: #e8e9ee; }
        button:hover { background: #2c2f38; }
        button.go { background: #4d6bfe; color: #fff; flex: 1; }
        button.go:hover { background: #5f7bff; }
        .note { color: #8a8e9b; font-size: 11.5px; margin-top: 8px; }
        .bar { display: flex; align-items: center; gap: 10px; }
        .bar .what { flex: 1; min-width: 0; }
        .bar .what span { display: block; color: #8a8e9b; font-size: 11.5px; }
        .bar .what b { display: block; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .dot { width: 8px; height: 8px; border-radius: 50%; background: #34c38f; flex: none; animation: pulse 1.2s ease-in-out infinite; }
        .dot.paused { background: #f2b84b; animation: none; }
        @keyframes pulse { 50% { opacity: .35; } }
      </style>
      <div class="ring"></div>
      <div class="cursor"><svg width="26" height="26" viewBox="0 0 26 26"><path d="M3 2l17 9.6-7.4 1.6L9.4 20z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></svg><span class="tag">Scout</span></div>
      <div class="panel"></div>`;
      cursor = root.querySelector('.cursor');
      ring = root.querySelector('.ring');
      panel = root.querySelector('.panel');
      document.documentElement.appendChild(host);
      place(pos.x, pos.y, 0);
      const follow = () => { frame = requestAnimationFrame(follow); track(); };
      follow();
    }
    function place(x, y) {
      cursor.style.transform = `translate(${x - 3}px, ${y - 3}px)`;
      pos = { x, y };
    }
    /** Keeps the ring and panel on the target as the page moves. */
    function track() {
      if (!target || !target.isConnected) { ring.style.opacity = 0; placePanel(null); return; }
      const r = target.getBoundingClientRect();
      Object.assign(ring.style, { opacity: 1, left: `${r.left - 5}px`, top: `${r.top - 5}px`, width: `${r.width + 10}px`, height: `${r.height + 10}px` });
      placePanel(r);
    }
    /** The panel sits at the bottom middle, out of the way. */
    function placePanel() {
      const w = Math.min(340, innerWidth - 24), h = panel.offsetHeight || 70;
      panel.style.width = `${w}px`;
      panel.style.left = `${(innerWidth - w) / 2}px`;
      panel.style.top = `${innerHeight - h - 18}px`;
    }
    /**
     * Moves the cursor across the page to the element: along a gentle curve that bends a little differently each time,
     * easing in and out, slower over longer distances. It's the on-screen cursor only, so you can follow along.
     */
    async function moveTo(el) {
      mount();
      el.scrollIntoView({ block: 'center', inline: 'nearest' });
      await sleep(150);
      target = el;
      const r = el.getBoundingClientRect();
      const to = { x: r.left + r.width / 2 + (Math.random() - 0.5) * Math.min(10, r.width / 4), y: r.top + r.height / 2 + (Math.random() - 0.5) * Math.min(6, r.height / 4) };
      const from = { ...pos };
      const dist = Math.hypot(to.x - from.x, to.y - from.y);
      if (dist < 2) return;
      // The bend: off to one side of the straight line, by up to a fifth of the distance.
      const bend = (Math.random() - 0.5) * 0.4 * dist;
      const mid = { x: (from.x + to.x) / 2 - ((to.y - from.y) / dist) * bend, y: (from.y + to.y) / 2 + ((to.x - from.x) / dist) * bend };
      const ms = Math.min(1300, 350 + dist * (0.7 + Math.random() * 0.3));
      const start = performance.now();
      await new Promise((done) => {
        const step = (now) => {
          const t = Math.min(1, (now - start) / ms);
          const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
          const u = 1 - e;
          place(u * u * from.x + 2 * u * e * mid.x + e * e * to.x, u * u * from.y + 2 * u * e * mid.y + e * e * to.y);
          if (t < 1 && host) requestAnimationFrame(step);
          else done();
        };
        requestAnimationFrame(step);
        // A tab in the background gets no frames; don't wait on them.
        setTimeout(done, ms + 400);
      });
      place(to.x, to.y);
    }
    function tap() {
      cursor.classList.add('down');
      setTimeout(() => cursor.classList.remove('down'), 160);
    }
    /**
     * Shows the panel and waits for a button. `buttons` is [id, label] pairs; the first is the main one. Each button
     * calls `act(id)` right in its click, so a click Scout makes still counts as the user's (Claude's sign-in window
     * needs that), then resolves with the id.
     */
    function ask({ step, text, note, buttons, act }) {
      mount();
      const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
      panel.innerHTML = `<div class="head"><b>Scout · Add to ${SITE}</b>${step ? `<span>${esc(step)}</span>` : ''}</div>
        <div class="text">${text}</div>
        <div class="row">${buttons.map(([id, label], i) => `<button data-id="${esc(id)}" class="${i === 0 ? 'go' : ''}">${esc(label)}</button>`).join('')}</div>
        ${note ? `<div class="note">${esc(note)}</div>` : ''}`;
      return new Promise((resolve) => {
        panel.querySelectorAll('button').forEach((b) => b.addEventListener('click', (e) => {
          e.stopPropagation();
          const id = b.dataset.id;
          panel.querySelectorAll('button').forEach((x) => (x.disabled = true));
          try { act && act(id); } catch (err) { console.warn('Scout setup:', err); }
          resolve(id);
        }, { once: true }));
      });
    }
    function say(text, note) {
      mount();
      panel.innerHTML = `<div class="head"><b>Scout · Add to ${SITE}</b></div><div class="text">${text}</div>${note ? `<div class="note">${note}</div>` : ''}`;
    }
    /** The bar while it runs on its own: which step, what it's doing, and Pause. Paused, it also offers Stop. */
    let onStop = null;
    let shown = { step: '', text: '' };
    function status(step, text) {
      mount();
      shown = { step, text };
      const plain = String(text).replace(/<[^>]+>/g, '');
      panel.innerHTML = `<div class="bar"><span class="dot ${paused ? 'paused' : ''}"></span>
        <div class="what"><span>Scout · ${paused ? 'Paused' : step}</span><b title="${plain.replace(/"/g, '&quot;')}">${plain}</b></div>
        ${paused ? '<button data-id="stop">Stop</button><button data-id="pause" class="go" style="flex:none">Resume</button>' : '<button data-id="pause">Pause</button>'}</div>`;
      panel.querySelectorAll('button').forEach((b) => b.addEventListener('click', (e) => {
        e.stopPropagation();
        if (b.dataset.id === 'pause') { paused = !paused; status(shown.step, shown.text); }
        else if (onStop) onStop();
      }));
    }
    function stopWith(fn) { onStop = fn; }
    const at = () => ({ ...pos });
    function point(el) { target = el; }
    function hidePanel() { if (panel) panel.style.visibility = "hidden"; }
    function showPanel() { if (panel) panel.style.visibility = ""; }
    function close() {
      cancelAnimationFrame(frame);
      if (host) host.remove();
      host = null;
      target = null;
    }
    return { moveTo, tap, ask, say, status, stopWith, at, point, close, hidePanel, showPanel };
  })();

  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

  // ---------- the setup ----------

  /** Waits for `test` to give something, up to `ms`. Gives up early when the job was stopped. */
  async function until(test, ms) {
    const end = Date.now() + ms;
    for (;;) {
      if (stopped) return null;
      let v = null;
      try { v = test(); } catch {}
      if (v) return v;
      if (Date.now() > end) return null;
      await sleep(250);
    }
  }

  let stopped = false;
  let running = false;

  const customItem = () => byText([/^add custom connector$/i, /add custom connector/i]);
  /** Claude's Add custom connector form, while it's open. */
  const formDialog = () => [...document.querySelectorAll('[role="dialog"], [role="alertdialog"], dialog[open]')].filter((d) => visible(d) && /add custom connector/i.test(d.innerText || '')).pop() || null;
  /** Its second page, after Continue: the sign-in options and the Add button. */
  const secondPage = () => { const d = formDialog(); return !!d && /step 2 of 2|\bauthentication\b/i.test(d.innerText || ''); };
  const enabled = (b) => !b.disabled && b.getAttribute('aria-disabled') !== 'true';

  function steps(job) {
    const quoted = `<code>${esc(job.name)}</code>`;
    return [
      {
        id: 'open',
        text: 'Click <b>Add custom connector</b>.',
        // Claude puts it in a menu under Add. The menu closes at any click outside it, Approve included, so one
        // approval covers both clicks: Add, then the menu's item.
        find: () => customItem() || byText([/^add (a )?connector$/i, /^add$/i, /^\+$/]),
        textFor: (el) => customItem() === el ? null : 'Click <b>Add</b>, then <b>Add custom connector</b> in the menu that opens.',
        act: async (el) => {
          if (customItem() === el) return click(el);
          await click(el);
          const item = await until(customItem, 4000);
          if (!item) return;
          await ui.moveTo(item);
          await click(item);
        },
        done: () => nameField(),
      },
      {
        id: 'name',
        text: `Type ${quoted} as the name.`,
        find: nameField,
        type: job.name,
        done: () => { const f = nameField(); return f && f.value.trim() === job.name; },
      },
      {
        id: 'url',
        text: `Type the address <code>${esc(job.url)}</code>.`,
        find: urlField,
        type: job.url,
        done: () => { const f = urlField(); return f && f.value.trim() === job.url; },
      },
      {
        id: 'continue',
        text: 'Click <b>Continue</b>.',
        // Claude's form says Continue now; Add and the rest are what it said before, or might say again.
        // Found only once it's enabled: Claude keeps Continue greyed out until the form has taken what was typed.
        find: () => { if (secondPage()) return null; const b = saveButton(); return b && enabled(b) ? b : null; },
        why: () => saveButton() ? 'Claude\'s <b>Continue</b> is still greyed out, so the form hasn\'t taken what was typed. Click into the address box, type a space and delete it, then press Continue here.' : null,
        act: (el) => click(el),
        done: () => secondPage() || !formDialog(),
      },
      {
        id: 'save',
        // Claude's second page: how users sign in. Scout leaves the options Claude detected from the server.
        text: 'Click <b>Add</b> to save the connector. Claude\'s detected sign-in settings stay as they are.',
        skip: () => !formDialog(),
        find: () => { const d = secondPage() && formDialog(); const b = d && [...d.querySelectorAll('button')].find((x) => /^(add|save|add connector)$/i.test(words(x))); return b && enabled(b) && visible(b) ? b : null; },
        act: (el) => click(el),
        done: () => !formDialog(),
      },
      {
        id: 'connect',
        text: `Click <b>Connect</b> on ${quoted}. Claude then opens ScoutCode in a new window, where Scout points you to the last step.`,
        find: connectButton,
        act: (el) => click(el),
        done: () => true,
        optional: true,
      },
    ];
  }

  // Gemini: Create custom app opens "Connect to an MCP server" (just the address), whose Next opens "Connect this MCP
  // server", a notice with a checkbox the user ticks themselves before Connect. Gemini asks for no name.
  const dialogs = () => [...document.querySelectorAll('[role="dialog"], dialog[open]')].filter(visible);
  const gmForm = () => dialogs().filter((d) => d.querySelector('input[type="url"]')).pop() || null;
  const gmNotice = () => dialogs().filter((d) => /connect this mcp server/i.test(d.innerText || '') || d.querySelector('input[type="checkbox"]')).pop() || null;
  const gmBox = () => { const d = gmNotice(); return d && d.querySelector('input[type="checkbox"]'); };
  const gmButton = (d, pattern) => { const b = d && [...d.querySelectorAll('button')].find((x) => pattern.test(words(x))); return b && enabled(b) ? b : null; };

  function geminiSteps(job) {
    return [
      {
        id: 'open',
        text: 'Click <b>Create custom app</b>.',
        find: () => (gmForm() ? null : byText([/^create custom app$/i])),
        act: (el) => click(el),
        done: () => gmForm(),
      },
      {
        id: 'url',
        text: `Type the address <code>${esc(job.url)}</code>.`,
        find: () => { const d = gmForm(); return d && d.querySelector('input[type="url"]'); },
        type: job.url,
        done: () => { const d = gmForm(); const f = d && d.querySelector('input[type="url"]'); return f && f.value.trim() === job.url; },
      },
      {
        id: 'next',
        text: 'Click <b>Next</b>.',
        find: () => (gmNotice() ? null : gmButton(gmForm(), /^next$/i)),
        act: (el) => click(el),
        done: () => !!gmNotice(),
      },
      {
        id: 'accept',
        // Gemini's notice about custom apps: Scout ticks its box, as its setup says before Start.
        text: 'Tick <b>I understand and accept</b> on Gemini\'s notice.',
        skip: () => { const b = gmBox(); return !!b && b.checked; },
        find: () => { const b = gmBox(); return b && !b.checked ? b : null; },
        act: (el) => click(el),
        done: () => { const b = gmBox(); return !!b && b.checked; },
      },
      {
        id: 'connect',
        text: 'Click <b>Connect</b>. Google then asks to link your Google Account (click <b>Agree and continue</b>), and ScoutCode\'s sign-in comes after.',
        find: () => gmButton(gmNotice(), /^connect$/i),
        act: (el) => click(el),
        done: () => !gmNotice(),
      },
      {
        id: 'save',
        // After Link in ScoutCode's window, Gemini asks to save the app, its name filled in from the server.
        text: 'Approve <b>Link</b> in ScoutCode\'s window. Scout then clicks <b>Connect</b> to save the app in Gemini.',
        wait: 600000,
        find: () => gmButton(dialogs().filter((d) => /save your custom app/i.test(d.innerText || '')).pop(), /^connect$/i),
        act: (el) => click(el),
        done: () => !dialogs().some((d) => /save your custom app/i.test(d.innerText || '')),
      },
    ];
  }

  /** Whether Gemini already lists the app: an installed app is a button named "<name>, <description>". */
  const gmListed = (job) => {
    const host = (job.url.split('/')[2] || '').toLowerCase();
    return clickables().some((el) => {
      const n = (el.getAttribute('aria-label') || words(el)).toLowerCase();
      return n === job.name.toLowerCase() || n.startsWith(`${job.name.toLowerCase()},`) || (host && n.includes(host));
    });
  };

  /** The Connect button in the connector's own row: the biggest box around it that holds no other Connect button. */
  function connectButton() {
    const all = clickables().filter((el) => /^connect$/i.test(words(el)));
    const name = new RegExp(`\\b${connectorName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`, 'i');
    return all.find((el) => {
      let row = el;
      while (row.parentElement && row.parentElement !== document.body && all.filter((b) => row.parentElement.contains(b)).length === 1) row = row.parentElement;
      return name.test(row.innerText || '');
    }) || (all.length === 1 && !onConnectors() ? all[0] : null);
  }
  let connectorName = 'Scout';

  /** Whether the connectors list already shows one by this name: in the connectors view, not the chat list beside it. */
  const listed = (name) => {
    const scope = [...document.querySelectorAll('[role="dialog"]')].filter(visible)[0] || document.querySelector('main') || document.body;
    return [...scope.querySelectorAll('*')].some((el) => el.childElementCount === 0 && (el.textContent || '').trim() === name && visible(el));
  };

  async function run(job) {
    running = true;
    stopped = false;
    connectorName = job.name;
    const list = GEMINI ? geminiSteps(job) : steps(job);
    const report = (e) => talk([{ id: job.id, ...e }]).then((r) => { if (r && r.stopped) stop(false); return r; });
    // Keeps Scout posted, and hears when it was stopped from Scout.
    const beat = setInterval(() => report({ kind: 'beat' }), 3000);
    const finish = (kind, extra) => { clearInterval(beat); running = false; release(); return report({ kind, ...extra }); };

    try {
      if (!GEMINI && /^\/login|^\/logout/.test(location.pathname)) {
        ui.say('Sign in to Claude first. Scout carries on with the connector setup after.');
        clearInterval(beat);
        running = false;
        return;
      }
      ui.stopWith(() => stop(true));
      if (!onConnectors()) {
        ui.status('Getting started', 'Opening Customize → Connectors');
        await sleep(900);
        await gate();
        if (stopped) return;
        // On Claude only the # changes when already on /new, so the page doesn't reload and this carries on here. On
        // Gemini the page loads again, and this script there picks the job up.
        location.assign(CONNECTORS_PATH);
        if (!(await until(onConnectors, 8000))) return;
        ui.say('Opening Connectors…');
      }
      // Part-way steps only make sense with the dialog open, which a reload closed.
      let i = !GEMINI && job.step >= 5 ? 5 : 0;
      await until(() => clickables().length > 3, 15000);
      await sleep(800);
      if (GEMINI && gmListed(job)) {
        ui.say(`Gemini already has Scout's app. Nothing to add.`);
        await finish('done', { label: 'Already added to Gemini' });
        setTimeout(ui.close, 6000);
        return;
      }
      if (!GEMINI && i === 0 && listed(job.name)) {
        if (!connectButton()) {
          ui.say(`Claude already has a connector called <code>${esc(job.name)}</code>. Nothing to add.`);
          await finish('done', { label: 'Already added' });
          setTimeout(ui.close, 6000);
          return;
        }
        i = 5;
      }
      let tries = 0;
      while (i < list.length) {
        if (stopped) return;
        const s = list[i];
        if (s.skip && s.skip()) { i++; tries = 0; continue; }
        const label = `Step ${i + 1} of ${list.length}`;
        report({ kind: 'step', step: i, label: s.text.replace(/<[^>]+>/g, '') });
        const el = await until(s.find, s.wait || 10000);
        if (stopped) return;
        if (!el) {
          if (s.optional) break;
          const reason = s.why && s.why();
          const a = await ui.ask({ step: label, text: reason || `Scout can't find where to do this: ${s.text}`, note: reason ? null : 'Do it yourself and press Continue, or Stop.', buttons: [['retry', 'Continue'], ['stop', 'Stop']] });
          if (a === 'stop') return stop(true);
          if (s.done()) i++;
          continue;
        }
        // It runs on its own now: the bar says what it's doing, and Pause holds it before the next move or key.
        ui.status(label, (s.textFor && s.textFor(el)) || s.text);
        await gate();
        if (stopped) return;
        await ui.moveTo(el);
        await sleep(250 + Math.random() * 350);
        await gate();
        if (stopped) return;
        // A step that's the user's (Gemini's notice): Scout points at it and waits until it's done.
        if (s.user) {
          ui.status(label, `Your turn: ${s.text}`);
          if (await until(s.done, 600000)) { i++; tries = 0; continue; }
          if (stopped) return;
          const a = await ui.ask({ step: label, text: s.text, buttons: [['retry', 'Continue'], ['stop', 'Stop']] });
          if (a === 'stop') return stop(true);
          continue;
        }
        // Claude's form only takes real typing. When Chrome won't allow it, the user can clear the way or type it.
        if (!s.act && !(await canReal())) {
          const b = await ui.ask({
            step: label,
            text: `${blockedWhy()} Turn that extension off for claude.ai and press Try again, or type <code>${esc(s.type)}</code> in the highlighted box yourself.`,
            buttons: [['retry', 'Try again'], ['self', 'I\'ll type it'], ['stop', 'Stop']],
          });
          if (b === 'stop') return stop(true);
          if (b === 'retry') { attached = null; continue; }
          ui.status(label, `Your turn: type ${s.type} in the highlighted box.`);
          if (await until(s.done, 180000)) { i++; tries = 0; }
          continue;
        }
        const at = el.isConnected ? el : s.find() || el;
        const typedInto = s.act ? null : at;
        const acting = s.act ? s.act(at) : type(at, s.type);
        await Promise.race([acting, sleep(s.act ? 20000 : 90000)]);
        // Typing is done when the field it typed into says exactly that, whatever finding the field again gives.
        const done = typedInto ? () => typedInto.isConnected && typedInto.value.trim() === s.type.trim() : s.done;
        const ok = await until(done, 6000);
        if (stopped) return;
        if (ok) { i++; tries = 0; continue; }
        // The step ran but the page didn't move on: an Add menu opened, or Claude showed an error. Try it again.
        if (++tries >= 3) {
          const err = dialog() && (dialog().querySelector('[role="alert"], .text-danger-000, [class*="error"]') || {}).innerText;
          const b = await ui.ask({ step: label, text: err ? `Claude says: ${esc(err.slice(0, 200))}` : `That didn't seem to work: ${s.text}`, buttons: [['retry', 'Try again'], ['stop', 'Stop']] });
          if (b === 'stop') return stop(true);
          tries = 0;
        }
      }
      ui.point(null);
      // Gemini's last step saved the app after Link, so it's ready: a new chat to ask it in.
      if (GEMINI && i >= list.length) {
        ui.say('Gemini is completely ready. Opening a new chat: ask it to work on your project.');
        await finish('done', { label: 'Gemini is linked to Scout and completely ready. Ask it to work on your project in the new chat.' });
        setTimeout(() => location.assign('/app'), 2500);
        return;
      }
      // Past the last step means Connect was clicked; stopping at it means there was no Connect button to click.
      ui.say(i >= list.length
        ? `Scout is added to ${SITE}. Finish in the ScoutCode window ${SITE} opened: sign in if it asks, then approve <b>Link</b>.`
        : 'Scout is added to Claude. Click <b>Connect</b> next to it and sign in to ScoutCode to finish.');
      await finish('done', { label: `Added to ${SITE}` });
      setTimeout(ui.close, 12000);
    } catch (e) {
      ui.say(`Something went wrong: ${esc(e && e.message || e)}`);
      await finish('error', { text: String(e && e.message || e) });
      setTimeout(ui.close, 8000);
    }

    function stop(tell) {
      stopped = true;
      running = false;
      clearInterval(beat);
      release();
      ui.close();
      if (tell) talk([{ id: job.id, kind: 'stopped' }]);
    }
  }

  // ---------- ScoutCode's consent page, after Connect ----------

  const onScoutCode = location.hostname === 'scoutcode.dev';

  /**
   * Claude's (or Gemini's) Connect opens ScoutCode's sign-in in its own window. Signed out, ScoutCode shows its login first: that's the
   * user's to do, so Scout stops and says so. Signed in, it's the consent page, where Scout points at Link and waits
   * for Approve: that click is what lets Claude reach this computer, so it stays the user's.
   */
  async function consent(job) {
    running = true;
    const tell = (label) => chrome.runtime.sendMessage({ scoutClaude: { tab, consent: true, events: [{ label }] } }).catch(() => {});
    // Only the request of the chatbot the user started the setup for in Scout.
    const bot = job.bot === 'gemini' ? 'gemini' : 'claude';
    const site = bot === 'gemini' ? 'Gemini' : 'Claude';
    SITE = site;
    try {
      if (location.pathname.startsWith('/login')) {
        ui.say(`Sign in to ScoutCode to finish linking ${site} to Scout. Scout carries on once you're signed in.`, 'Scout doesn\'t sign in for you.');
        tell('Waiting for you to sign in to ScoutCode');
        return;
      }
      const page = await until(() => /link a chatbot/i.test(document.body.innerText || '') && document.body, 10000);
      if (!page || !new RegExp(bot, 'i').test(page.innerText || '')) return;
      const link = await until(() => byText([/^link$/i, /^allow$/i]), 10000);
      if (!link) return;
      await ui.moveTo(link);
      tell(`Waiting for you to approve linking ${site}`);
      let acting = null;
      const a = await ui.ask({
        step: 'Last step',
        text: `Click <b>Link</b> to let ${site} use Scout through your ScoutCode account.`,
        note: `This is what lets ${site} reach your computer through Scout.`,
        buttons: [['approve', 'Approve'], ['self', 'I\'ll do it'], ['stop', 'Stop']],
        act: (id) => { if (id === 'approve') { acting = click(link); } },
      });
      if (a === 'stop') { release(); ui.close(); tell('Stopped at ScoutCode\'s consent page'); return; }
      if (acting) { ui.say('Linking…'); await Promise.race([acting, sleep(20000)]); }
      else ui.say('Go ahead: click <b>Link</b>.', 'Scout closes this once it\'s done.');
      const gone = await until(() => !link.isConnected || link.disabled || !/link a chatbot/i.test(document.body.innerText || ''), a === 'self' ? 180000 : 15000);
      release();
      ui.point(null);
      if (gone) {
        tell(`${site} is linked to Scout`);
        ui.say(`Done. ${site} is linked to Scout: go back to ${site} and ask it to work on your project.`);
        setTimeout(ui.close, 8000);
      } else {
        ui.say('That didn\'t seem to go through. Click <b>Link</b> yourself to finish.');
      }
    } finally {
      running = false;
    }
  }

  async function check() {
    if (running) return;
    if (onScoutCode) {
      const r = await chrome.runtime.sendMessage({ scoutClaude: { tab, consent: true, events: [] } }).catch(() => null);
      if (r && r.job) consent(r.job);
      return;
    }
    const r = await talk();
    if (r && r.job) run(r.job);
  }
  check();
  // Scout may ask after this tab loaded, such as when it's already open.
  document.addEventListener('visibilitychange', () => { if (!document.hidden) check(); });
  window.addEventListener('focus', check);
})();
