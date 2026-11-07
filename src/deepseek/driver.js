// Scout's driver for chat.deepseek.com. It runs inside the page, in Scout's own DeepSeek window (as an initialization
// script) or in the user's browser (from the Scout extension, in the page's main world), and it's the same file in both.
//
// Scout hands it a job: the text to send, whether DeepThink and Search are on, and which chat to send it in. The driver
// opens that chat, types a short line into the composer and sends it like the user would, so the page does its own
// sign-in and proof-of-work. As the request goes out, it swaps in the full prompt and the mode, and it reads a copy of
// the streamed answer. The page still shows the chat as normal.
//
// Events go to `onEvent` when the extension sets it, and otherwise wait in a queue for Scout to `drain`.
(() => {
  if (window.__scoutDS) return;
  const VERSION = 1;
  const SAVED = 'scout-ds-job';
  const COMPLETION = /\/api\/v\d+\/chat\/completions?(\?|$)/;
  const queue = [];
  let job = null; // The job being run: { id, prompt, display, thinking, search, chat }.
  let stage = 'idle'; // idle, opening, sending, waiting, streaming
  let stageAt = Date.now();
  let armed = null; // The job whose request is about to go out.
  let abort = null;

  const api = { version: VERSION, onEvent: null };
  const emit = (e) => (api.onEvent ? api.onEvent(e) : queue.push(e));
  const send = (j, kind, text) => j && emit({ id: j.id, kind, text: text ?? '' });
  const setStage = (s) => { stage = s; stageAt = Date.now(); overlay(s !== 'idle'); };

  // ---------- the overlay ----------

  // While Scout works, the page is covered so a click or a keypress can't get in the way. The driver's own typing and
  // clicks come from script, so they still reach the page underneath.
  let cover = null;
  function overlay(on) {
    if (!on) {
      if (cover) cover.remove();
      cover = null;
      return;
    }
    if (cover && cover.isConnected) return;
    if (!document.body) {
      document.addEventListener('DOMContentLoaded', () => overlay(stage !== 'idle'), { once: true });
      return;
    }
    cover = document.createElement('div');
    cover.setAttribute('data-scout-overlay', '');
    const shadow = cover.attachShadow({ mode: 'closed' });
    shadow.innerHTML = `
      <style>
        :host { all: initial; position: fixed; inset: 0; z-index: 2147483647; cursor: not-allowed; }
        .shade { position: absolute; inset: 0; background: rgba(8, 12, 28, 0.18); }
        .ring { position: absolute; inset: 0; pointer-events: none; padding: 4px; overflow: hidden;
          -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
          -webkit-mask-composite: xor; mask-composite: exclude; }
        .ring::before { content: ''; position: absolute; left: 50%; top: 50%; width: 200vmax; height: 200vmax;
          background: conic-gradient(#4d6bfe, #9fb1ff, #2b4bf0, #c3cdff, #4d6bfe);
          transform: translate(-50%, -50%); animation: spin 3s linear infinite; }
        .glow { position: absolute; inset: 0; pointer-events: none; box-shadow: inset 0 0 40px rgba(77, 107, 254, 0.55);
          animation: pulse 2s ease-in-out infinite; }
        .badge { position: absolute; top: 16px; left: 50%; transform: translateX(-50%); display: flex; align-items: center;
          gap: 10px; padding: 9px 16px; border-radius: 999px; background: #4d6bfe; color: #fff;
          font: 600 14px/1.2 system-ui, -apple-system, 'Segoe UI', sans-serif; box-shadow: 0 6px 24px rgba(77, 107, 254, 0.45); }
        .dot { width: 8px; height: 8px; border-radius: 50%; background: #fff; animation: pulse 1.2s ease-in-out infinite; }
        .sub { font-weight: 400; opacity: 0.85; }
        @keyframes spin { to { transform: translate(-50%, -50%) rotate(360deg); } }
        @keyframes pulse { 50% { opacity: 0.45; } }
      </style>
      <div class="shade"></div><div class="glow"></div><div class="ring"></div>
      <div class="badge"><span class="dot"></span>Scout is using DeepSeek<span class="sub">· please don't type here</span></div>`;
    // Swallows the user's input over the page, but not the driver's.
    for (const type of ['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel', 'touchstart', 'pointerdown']) {
      cover.addEventListener(type, (e) => { e.preventDefault(); e.stopPropagation(); }, { capture: true, passive: false });
    }
    document.body.appendChild(cover);
  }
  // Keys go to whatever has focus, so a trusted keypress (the user's, never the driver's) is stopped while it's up.
  window.addEventListener('keydown', (e) => { if (cover && e.isTrusted) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
  window.addEventListener('paste', (e) => { if (cover && e.isTrusted) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
  const finish = (j, kind, text) => {
    if (!j || j.over) return;
    j.over = true;
    send(j, kind, text);
    if (job === j) { job = null; armed = null; setStage('idle'); }
  };

  // ---------- the page ----------

  const composer = () => document.querySelector('textarea#chat-input') || document.querySelector('textarea') || document.querySelector('[contenteditable="true"]');
  const onSignIn = () => /sign_in|sign-in|signin|login|register/i.test(location.pathname);
  // The site keeps its sign-in in localStorage. The composer showing is the fallback, in case that key moves.
  const hasToken = () => { try { return Object.keys(localStorage).some((k) => /token/i.test(k) && localStorage.getItem(k)); } catch { return false; } };
  const signedIn = () => !onSignIn() && (hasToken() || !!composer());

  function type(ta, text) {
    if (ta.tagName === 'TEXTAREA') {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      setter.call(ta, text);
      ta.dispatchEvent(new Event('input', { bubbles: true }));
      return;
    }
    ta.focus();
    document.execCommand('selectAll');
    document.execCommand('insertText', false, text);
  }

  function pressEnter(ta) {
    ta.focus();
    for (const type of ['keydown', 'keypress', 'keyup']) {
      ta.dispatchEvent(new KeyboardEvent(type, { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }));
    }
  }

  /** The send button: the last icon-only button around the composer that isn't turned off. */
  function sendButton(ta) {
    let box = ta;
    for (let i = 0; i < 6 && box.parentElement; i++) box = box.parentElement;
    const buttons = [...box.querySelectorAll('div[role="button"], button')].filter((b) => b.querySelector('svg') && !b.textContent.trim() && b.getAttribute('aria-disabled') !== 'true' && !b.disabled);
    return buttons[buttons.length - 1] || null;
  }

  const chatPath = (id) => `/a/chat/s/${id}`;
  const onNewChat = () => !/\/chat\/s\//.test(location.pathname) && !onSignIn();

  // ---------- running a job ----------

  function start(j) {
    if (job && job !== j) finish(job, 'error', 'Replaced by a newer request.');
    job = j;
    setStage('opening');
    const here = j.chat ? location.pathname === chatPath(j.chat) : j.reopened ? onNewChat() || !onSignIn() : false;
    if (!here && j.reopened) {
      // The chat went somewhere else after opening: it was deleted, or the sign-in ran out.
      return finish(j, 'error', onSignIn() ? 'signin' : 'chat-missing');
    }
    if (!here) {
      // Opening another chat reloads the page, and this script with it, so the job waits in the tab until it's back.
      j.reopened = true;
      try { sessionStorage.setItem(SAVED, JSON.stringify(j)); } catch {}
      location.assign(j.chat ? chatPath(j.chat) : '/');
      return;
    }
    tick();
  }

  function stop() {
    if (abort) try { abort.abort(); } catch {}
    if (job) finish(job, 'error', 'Stopped.');
  }

  /** Moves the job along. It's called often, from Scout's polling and timers, since a hidden page's timers run slowly. */
  function tick() {
    const j = job;
    if (!j) return;
    const waited = Date.now() - stageAt;
    if (stage === 'opening') {
      if (onSignIn()) return finish(j, 'error', 'signin');
      const ta = composer();
      if (!ta) {
        if (waited > 45000) finish(j, 'error', "DeepSeek's page didn't finish loading.");
        return;
      }
      // Lets the page settle after a chat opens, so the send isn't lost.
      if (waited < 600) return;
      type(ta, j.display || 'Continue.');
      armed = j;
      setStage('sending');
      setTimeout(tick, 120);
      return;
    }
    if (stage === 'sending') {
      const ta = composer();
      if (!ta) return;
      if (!j.pressed) { j.pressed = true; pressEnter(ta); return; }
      if (waited > 2500 && !j.clicked) {
        j.clicked = true;
        const b = sendButton(ta);
        if (b) b.click();
        return;
      }
      if (waited > 12000) finish(j, 'error', "Scout couldn't send on DeepSeek's page. It may have changed; check for a Scout update.");
      return;
    }
    if (stage === 'waiting' && waited > 180000) finish(j, 'error', "DeepSeek didn't answer.");
  }

  // ---------- the request and its answer ----------

  /** The page's request body with the job's prompt and mode in it. */
  function rewrite(j, body) {
    const b = JSON.parse(body);
    b.prompt = j.prompt;
    b.thinking_enabled = !!j.thinking;
    b.search_enabled = !!j.search;
    b.ref_file_ids = [];
    if (j.modelType) b.model_type = j.modelType;
    if (b.chat_session_id) send(j, 'chat', b.chat_session_id);
    return JSON.stringify(b);
  }

  /** Reads DeepSeek's stream. It has had a few shapes, and this follows all of them. */
  function reader(j) {
    let path = null; // The field the last update was to, which a bare { v } continues.
    let kinds = []; // Each answer fragment's type, for the fragment shape.
    let done = false;
    const say = (kind, text) => { if (text && !done) send(j, kind, text); };
    const kindOf = (p) => {
      if (!p) return 'text';
      const f = p.match(/fragments\/(-?\d+)\/content$/);
      if (f) {
        const i = Number(f[1]);
        const t = kinds[i < 0 ? kinds.length + i : i] || 'RESPONSE';
        return /THINK/i.test(t) ? 'reasoning' : /RESPONSE|ANSWER|TEXT/i.test(t) ? 'text' : null;
      }
      if (/thinking/i.test(p)) return 'reasoning';
      if (/(^|\/)content$/.test(p)) return 'text';
      return null;
    };
    const fragments = (list) => {
      for (const f of list || []) {
        kinds.push(f.type || 'RESPONSE');
        if (typeof f.content === 'string') say(/THINK/i.test(f.type) ? 'reasoning' : /RESPONSE|ANSWER|TEXT/i.test(f.type || 'RESPONSE') ? 'text' : null, f.content);
      }
    };
    const apply = (d, base) => {
      if (d.p !== undefined) path = base ? `${base}/${d.p}` : d.p;
      const v = d.v;
      if (d.o === 'BATCH' && Array.isArray(v)) {
        const at = path;
        for (const x of v) apply(x, at);
        path = at;
        return;
      }
      if (typeof v === 'string') {
        if (path && /status$/.test(path)) { if (/FINISHED|DONE/i.test(v)) done = true; return; }
        const k = kindOf(path);
        if (k) say(k, v);
        return;
      }
      if (Array.isArray(v) && path && /fragments$/.test(path)) return fragments(v);
      if (v && typeof v === 'object' && v.response) {
        const r = v.response;
        if (Array.isArray(r.fragments)) { kinds = []; fragments(r.fragments); }
        if (typeof r.thinking_content === 'string') say('reasoning', r.thinking_content);
        if (typeof r.content === 'string') say('text', r.content);
      }
    };
    return {
      line(data) {
        let d;
        try { d = JSON.parse(data); } catch { return; }
        if (d.choices) {
          const c = d.choices[0] || {};
          const delta = c.delta || {};
          if (delta.content) say(delta.type === 'thinking' ? 'reasoning' : 'text', delta.content);
          if (c.finish_reason) done = true;
          return;
        }
        if (d.code && d.msg && d.v === undefined) {
          finish(j, 'error', d.msg);
          return;
        }
        if ('v' in d) apply(d, null);
      },
      get done() { return done; },
    };
  }

  async function follow(j, res) {
    const kind = res.headers.get('content-type') || '';
    if (!res.ok || kind.includes('application/json')) {
      // Refusals come back as JSON: rate limits, a sign-in that ran out, a failed check.
      let text = '';
      try { text = await res.text(); } catch {}
      let msg = `DeepSeek refused the request (${res.status}).`;
      try {
        const b = JSON.parse(text);
        msg = b.data?.biz_msg || b.msg || msg;
        if (b.code === 40003 || res.status === 401) msg = 'signin';
      } catch {}
      return finish(j, 'error', msg);
    }
    setStage('streaming');
    const r = reader(j);
    const dec = new TextDecoder();
    let rest = '';
    const take = (chunk) => {
      rest += chunk;
      const lines = rest.split('\n');
      rest = lines.pop();
      for (const l of lines) if (l.startsWith('data:')) r.line(l.slice(5).trim());
    };
    try {
      const rd = res.body.getReader();
      for (;;) {
        const { value, done } = await rd.read();
        if (done) break;
        take(dec.decode(value, { stream: true }));
        if (j.over) return;
      }
      take(dec.decode() + '\n');
      finish(j, 'done');
    } catch (e) {
      finish(j, 'error', j.over ? '' : `The answer stopped: ${e && e.message ? e.message : e}`);
    }
  }

  /** A request body as text, whatever form the page sent it in. */
  async function asText(b) {
    if (b == null) return null;
    if (typeof b === 'string') return b;
    if (b instanceof Blob) return b.text();
    if (b instanceof ArrayBuffer || ArrayBuffer.isView(b)) return new TextDecoder().decode(b);
    if (b instanceof URLSearchParams) return b.toString();
    return null;
  }

  /** Tells Scout the full prompt went in, which it waits for before it trusts the answer. */
  function sent(j) {
    send(j, 'sent', String(j.prompt.length));
    console.info(`[Scout] Sent DeepSeek Scout's full prompt: ${j.prompt.length} characters.`);
  }

  const nativeFetch = window.fetch;
  window.fetch = function (input, init) {
    const url = typeof input === 'string' ? input : input && input.url ? input.url : String(input);
    const j = armed;
    if (!j || !COMPLETION.test(url)) {
      if (j && /\/api\/.*chat/.test(url)) console.info('[Scout] Passed over a DeepSeek request:', url);
      return nativeFetch.apply(this, arguments);
    }
    armed = null;
    const self = this;
    return (async () => {
      const isRequest = typeof Request !== 'undefined' && input instanceof Request;
      const raw = init && init.body !== undefined ? init.body : isRequest ? await input.clone().text() : null;
      let body;
      try {
        body = rewrite(j, await asText(raw));
      } catch (e) {
        // Never let the short line go out alone: DeepSeek would answer it without Scout's instructions and tools.
        finish(j, 'error', "Scout couldn't put its prompt into DeepSeek's request. The site may have changed; check for a Scout update.");
        console.warn('[Scout] Could not rewrite the request:', e, raw);
        throw new DOMException('Stopped by Scout', 'AbortError');
      }
      abort = new AbortController();
      const theirs = (init && init.signal) || (isRequest ? input.signal : null);
      const signal = theirs && AbortSignal.any ? AbortSignal.any([abort.signal, theirs]) : abort.signal;
      setStage('waiting');
      sent(j);
      try {
        const res = isRequest ? await nativeFetch.call(self, new Request(input, { ...(init || {}), body, signal })) : await nativeFetch.call(self, input, { ...init, body, signal });
        follow(j, res.clone());
        return res;
      } catch (e) {
        finish(j, 'error', `Couldn't reach DeepSeek: ${e && e.message ? e.message : e}`);
        throw e;
      }
    })();
  };

  // The same for a page that sends with XMLHttpRequest.
  const xo = XMLHttpRequest.prototype.open;
  const xs = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__scoutUrl = String(url);
    return xo.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function (body) {
    const j = armed;
    if (!j || !COMPLETION.test(this.__scoutUrl || '')) return xs.apply(this, arguments);
    armed = null;
    try {
      body = rewrite(j, typeof body === 'string' ? body : body instanceof ArrayBuffer || ArrayBuffer.isView(body) ? new TextDecoder().decode(body) : null);
    } catch {
      finish(j, 'error', "Scout couldn't put its prompt into DeepSeek's request. The site may have changed; check for a Scout update.");
      return this.abort();
    }
    setStage('waiting');
    sent(j);
    const r = reader(j);
    let seen = 0;
    let rest = '';
    const read = () => {
      const t = this.responseText || '';
      rest += t.slice(seen);
      seen = t.length;
      const lines = rest.split('\n');
      rest = lines.pop();
      for (const l of lines) if (l.startsWith('data:')) r.line(l.slice(5).trim());
    };
    this.addEventListener('progress', () => { setStage('streaming'); read(); });
    this.addEventListener('load', () => {
      rest += '\n';
      read();
      if (this.status >= 400) finish(j, 'error', `DeepSeek refused the request (${this.status}).`);
      else finish(j, 'done');
    });
    this.addEventListener('error', () => finish(j, 'error', "Couldn't reach DeepSeek."));
    abort = { abort: () => this.abort() };
    return xs.call(this, body);
  };

  // ---------- what Scout and the extension call ----------

  api.start = (j) => start(j);
  api.stop = () => stop();
  api.tick = () => tick();
  api.status = () => ({ v: VERSION, signedIn: signedIn(), signIn: onSignIn(), path: location.pathname, busy: !!job, stage, composer: !!composer(), token: hasToken() });
  /** The status, and the events since the last call. */
  api.drain = () => {
    tick();
    return { ...api.status(), events: queue.splice(0) };
  };
  window.__scoutDS = api;

  // A job that opened a chat picks up where it was.
  let saved = null;
  try { saved = sessionStorage.getItem(SAVED); sessionStorage.removeItem(SAVED); } catch {}
  if (saved) {
    try { start(JSON.parse(saved)); } catch {}
  }
  setInterval(tick, 400);
})();
