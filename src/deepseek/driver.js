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
  // The requests a send starts with: the proof-of-work challenge, then the completion.
  const SEND_START = /\/create_pow_challenge|\/chat\/completions?(\?|$)/;
  // The request the page's Continue button makes, for an answer that hit its length limit.
  const CONTINUE = /\/api\/v\d+\/chat\/(continue|resume)/;
  // How many times one answer is continued before Scout takes what it has.
  const MAX_CONTINUES = 8;
  const queue = [];
  let job = null; // The job being run: { id, prompt, display, thinking, search, chat }.
  let stage = 'idle'; // idle, opening, sending, waiting, streaming
  let stageAt = Date.now();
  let armed = null; // The job whose request is about to go out.
  let abort = null;

  const api = { version: VERSION, onEvent: null };
  const emit = (e) => (api.onEvent ? api.onEvent(e) : queue.push(e));
  const send = (j, kind, text) => j && emit({ id: j.id, kind, text: text ?? '' });
  // Scout holds the page between requests (its pause, and the tools running) so the overlay stays up for the whole
  // turn. The hold runs out on its own in case Scout never lets go, and it's kept in the tab, since opening a chat
  // reloads the page.
  const HELD = 'scout-ds-held';
  let heldUntil = 0;
  try { heldUntil = Number(sessionStorage.getItem(HELD)) || 0; } catch {}
  const covered = () => stage !== 'idle' || Date.now() < heldUntil;
  const setStage = (s) => { stage = s; stageAt = Date.now(); overlay(covered()); };
  // Whether the page was signed in when Scout took it, so a sign-out while it's held can be told apart.
  let hadToken = false;
  const hold = (on) => {
    heldUntil = on ? Date.now() + 15 * 60000 : 0;
    if (on) hadToken = hadToken || hasToken();
    try { on ? sessionStorage.setItem(HELD, String(heldUntil)) : sessionStorage.removeItem(HELD); } catch {}
    overlay(covered());
  };

  /**
   * Stops everything and gives the page back: the job ends, the overlay comes off, and Scout is told why, so it ends
   * the turn instead of sending again. `why` is 'override' (the user took over) or 'signin' (the account signed out).
   */
  function shutdown(why) {
    const j = job;
    if (abort) try { abort.abort(); } catch {}
    hold(false);
    hadToken = false;
    emit({ id: 0, kind: 'shutdown', text: why });
    if (j) finish(j, 'error', why);
    else overlay(false);
    console.info(`[Scout] Gave the page back: ${why}.`);
  }
  // hCaptcha's challenge is a visible frame with `frame=challenge` in its address (the checkbox alone isn't one).
  // While it's up, the user has to solve it, so the overlay comes off and the job's timers wait.
  let challenged = false;
  const challengeUp = () => [...document.querySelectorAll('iframe[src*="hcaptcha"]')].some((f) => {
    const r = f.getBoundingClientRect();
    return f.src.includes('frame=challenge') && r.width > 0 && r.height > 0;
  });

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
      document.addEventListener('DOMContentLoaded', () => overlay(covered()), { once: true });
      return;
    }
    cover = document.createElement('div');
    cover.setAttribute('data-scout-overlay', '');
    const shadow = cover.attachShadow({ mode: 'closed' });
    shadow.innerHTML = `
      <style>
        :host { all: initial; position: fixed; inset: 0; z-index: 2147483647; cursor: not-allowed; }
        .shade { position: absolute; inset: 0; background: rgba(8, 12, 28, 0.18); }
        .ring { position: absolute; inset: 0; pointer-events: none; box-shadow: inset 0 0 0 3px #4d6bfe; }
        .badge { position: absolute; top: 16px; left: 50%; transform: translateX(-50%); display: flex; align-items: center;
          gap: 10px; padding: 9px 16px; border-radius: 999px; background: #4d6bfe; color: #fff;
          font: 600 14px/1.2 system-ui, -apple-system, 'Segoe UI', sans-serif; box-shadow: 0 6px 24px rgba(77, 107, 254, 0.45); }
        .dot { width: 8px; height: 8px; border-radius: 50%; background: #fff; animation: pulse 1.2s ease-in-out infinite; }
        .sub { font-weight: 400; opacity: 0.85; }
        .take { all: unset; cursor: pointer; margin-left: 4px; padding: 5px 12px; border-radius: 999px; background: #fff;
          color: #2b4bf0; font: 600 13px/1.2 system-ui, -apple-system, 'Segoe UI', sans-serif; }
        .take:hover { background: #e8ecff; }
        .take:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
        @keyframes pulse { 50% { opacity: 0.45; } }
      </style>
      <div class="shade"></div><div class="ring"></div>
      <div class="badge"><span class="dot"></span>Scout is using DeepSeek<span class="sub">· Please don't type here</span>
        <button class="take" type="button" title="Stop Scout's turn and use this tab yourself">Take over</button></div>`;
    const take = shadow.querySelector('.take');
    take.addEventListener('click', (e) => { if (e.isTrusted) shutdown('override'); });
    // Swallows the user's input over the page, but not the driver's, and not the Take over button.
    for (const type of ['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel', 'touchstart', 'pointerdown']) {
      cover.addEventListener(type, (e) => {
        if (e.composedPath().includes(take)) return;
        e.preventDefault();
        e.stopPropagation();
      }, { capture: true, passive: false });
    }
    document.body.appendChild(cover);
  }
  // Keys go to whatever has focus, so a trusted keypress (the user's, never the driver's) is stopped while it's up.
  window.addEventListener('keydown', (e) => { if (cover && e.isTrusted && !e.composedPath().includes(cover)) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
  window.addEventListener('paste', (e) => { if (cover && e.isTrusted) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
  const finish = (j, kind, text) => {
    if (!j || j.over) return;
    j.over = true;
    send(j, kind, text);
    if (job === j) { job = null; armed = null; challenged = false; setStage('idle'); }
  };

  // ---------- the page ----------

  const composer = () => document.querySelector('textarea#chat-input') || document.querySelector('textarea') || document.querySelector('[contenteditable="true"]');
  const onSignIn = () => /sign_in|sign-in|signin|login|register/i.test(location.pathname);
  // The site keeps its sign-in in localStorage. The composer showing is the fallback, in case that key moves.
  const hasToken = () => { try { return Object.keys(localStorage).some((k) => /token/i.test(k) && localStorage.getItem(k)); } catch { return false; } };
  const signedIn = () => !onSignIn() && (hasToken() || !!composer());

  /** Edits the composer: a textarea by its value, anything else by the editing commands. */
  function field(ta) {
    const isArea = ta.tagName === 'TEXTAREA';
    const write = (s) => {
      if (isArea) {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(ta, s);
        ta.dispatchEvent(new Event('input', { bubbles: true }));
      }
    };
    const selectAll = () => { ta.focus(); document.execCommand('selectAll'); };
    return {
      set(s) {
        if (isArea) return write(s);
        selectAll();
        document.execCommand('insertText', false, s);
      },
      put(c) {
        if (isArea) return write(ta.value + c);
        ta.focus();
        document.execCommand('insertText', false, c);
      },
      back() {
        if (isArea) return write(ta.value.slice(0, -1));
        ta.focus();
        document.execCommand('delete');
      },
    };
  }

  // Keys a typist is likely to hit by mistake: the neighbours of each letter on a QWERTY keyboard.
  const NEAR = {
    a: 'qwsz', b: 'vghn', c: 'xdfv', d: 'serfcx', e: 'wsdr', f: 'drtgvc', g: 'ftyhbv', h: 'gyujnb', i: 'ujko',
    j: 'huikmn', k: 'jiolm', l: 'kop', m: 'njk', n: 'bhjm', o: 'iklp', p: 'ol', q: 'wa', r: 'edft', s: 'awedxz',
    t: 'rfgy', u: 'yhji', v: 'cfgb', w: 'qesa', x: 'zsdc', y: 'tghu', z: 'asx',
  };
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const between = (a, b) => a + Math.random() * (b - a);
  const pick = (s) => s[Math.floor(Math.random() * s.length)];
  // Texts longer than this are pasted rather than typed, so a long line doesn't take a minute to appear.
  const TYPE_LIMIT = 90;

  /**
   * Types `text` into the composer the way a person would: a steady pace with pauses at spaces and punctuation, now
   * and then a longer think, and the odd typo that's noticed and backspaced. A long text is pasted in one go. This is
   * only what the page shows; the text that goes to DeepSeek is still the full prompt. Stops when `live()` turns false.
   * Resolves true when the whole text is in.
   */
  async function typeLikeAPerson(ta, text, live) {
    const f = field(ta);
    const chars = [...text];
    if (chars.length > TYPE_LIMIT) {
      f.set(text);
      return live();
    }
    f.set('');
    for (let i = 0; i < chars.length; i++) {
      if (!live()) return false;
      const c = chars[i];
      const lower = c.toLowerCase();
      if (NEAR[lower] && Math.random() < 0.03) {
        // A wrong key: noticed a beat later, sometimes after one more letter, then backspaced.
        f.put(pick(NEAR[lower]));
        await sleep(between(140, 320));
        if (!live()) return false;
        if (Math.random() < 0.4 && i + 1 < chars.length) {
          f.put(chars[i + 1]);
          await sleep(between(60, 140));
          f.back();
          await sleep(between(70, 150));
        }
        f.back();
        await sleep(between(90, 200));
      }
      f.put(c);
      let wait = between(55, 150);
      if (/[ .,!?;:]/.test(c) && Math.random() < 0.5) wait += between(150, 400);
      if (Math.random() < 0.03) wait += between(600, 1200);
      await sleep(wait);
    }
    return live();
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

  /** The Continue button DeepSeek shows under an answer that was cut off, if it's showing. */
  function continueButton() {
    const buttons = [...document.querySelectorAll('div[role="button"], button')].filter((b) => {
      const r = b.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && /^(continue|continue generating|继续|继续生成)$/i.test(b.textContent.trim());
    });
    return buttons[buttons.length - 1] || null;
  }

  /** An answer's stream ended: done, or cut off at its length limit, in which case it's continued. */
  function ended(j, r) {
    if (j.over) return;
    if (r.incomplete && (j.continues || 0) < MAX_CONTINUES) {
      r.incomplete = false;
      r.resumed = true;
      j.reader = r;
      j.contClicked = false;
      setStage('continuing');
      setTimeout(tick, 700);
      return;
    }
    finish(j, 'done');
  }

  const chatPath = (id) => `/a/chat/s/${id}`;
  const onNewChat = () => !/\/chat\/s\//.test(location.pathname) && !onSignIn();

  // ---------- running a job ----------

  function start(j) {
    if (job && job !== j) finish(job, 'error', 'Replaced by a newer request.');
    job = j;
    hadToken = hadToken || hasToken();
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
    // Signed out while Scout had the page (sent to the sign-in page, or the sign-in token gone): stop, don't keep going.
    if (covered() && (onSignIn() || (hadToken && !hasToken()))) return shutdown('signin');
    if (!j) {
      if (cover && !covered()) overlay(false);
      return;
    }
    const up = challengeUp();
    if (up !== challenged) {
      challenged = up;
      if (cover) cover.style.display = up ? 'none' : '';
      send(j, up ? 'challenge' : 'challenge-clear');
    }
    if (up) {
      stageAt = Date.now();
      return;
    }
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
      setStage('typing');
      typeLikeAPerson(ta, j.display || 'Continue.', () => job === j && !j.over).then((ok) => {
        if (!ok) return;
        armed = j;
        setStage('sending');
        setTimeout(tick, 120);
      }, () => finish(j, 'error', "Scout couldn't type into DeepSeek's page. It may have changed; check for a Scout update."));
      return;
    }
    if (stage === 'sending') {
      const ta = composer();
      if (!ta) return;
      // A short wait after typing, so the page has taken in the text before Enter.
      if (!j.pressed) {
        if (!j.enterAfter) j.enterAfter = 400 + Math.floor(Math.random() * 300);
        if (waited < j.enterAfter) { setTimeout(tick, j.enterAfter - waited); return; }
        j.pressed = true;
        pressEnter(ta);
        return;
      }
      // The button is only a fallback for an Enter that did nothing. Once the send's first request is out, the page is
      // sending and that button may be Stop by now, so it's left alone.
      if (waited > 2500 && !j.clicked && !j.requested) {
        j.clicked = true;
        const b = sendButton(ta);
        if (b) b.click();
        return;
      }
      if (waited > 12000) finish(j, 'error', "Scout couldn't send on DeepSeek's page. It may have changed; check for a Scout update.");
      return;
    }
    if (stage === 'continuing') {
      // The page may continue on its own; if it doesn't, Scout presses Continue once the button shows.
      if (waited < 700) return;
      if (!j.contClicked) {
        const b = continueButton();
        if (b) {
          j.contClicked = true;
          j.continues = (j.continues || 0) + 1;
          stageAt = Date.now();
          console.info(`[Scout] The answer was cut off; pressing Continue (${j.continues}).`);
          b.click();
        } else if (waited > 8000) {
          finish(j, 'done');
        }
        return;
      }
      // Pressed, but no continued answer came: keep what there is.
      if (waited > 15000) finish(j, 'done');
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
    // `resumed` is set when the answer is continued: the continued stream may open with the whole answer so far, and
    // only what's past what was already sent is new.
    const self = { incomplete: false, resumed: false };
    const said = { text: 0, reasoning: 0 };
    const say = (kind, text) => {
      if (!text || done) return;
      if (kind in said) said[kind] += text.length;
      send(j, kind, text);
    };
    const fragKind = (t) => (/THINK/i.test(t) ? 'reasoning' : /RESPONSE|ANSWER|TEXT/i.test(t || 'RESPONSE') ? 'text' : null);
    /** A snapshot of the whole answer, as a continued stream may open: says only its new part. */
    const snapshot = (parts) => {
      for (const k of Object.keys(parts)) if (parts[k].length > said[k]) say(k, parts[k].slice(said[k]));
    };
    // INCOMPLETE is an answer cut off at its length limit, which the page offers to continue.
    const status = (v) => {
      if (/INCOMPLETE/i.test(v)) self.incomplete = true;
      else if (/FINISHED|DONE/i.test(v)) { done = true; self.incomplete = false; }
    };
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
        if (typeof f.content === 'string') say(fragKind(f.type), f.content);
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
        if (path && /status$/.test(path)) { status(v); return; }
        const k = kindOf(path);
        if (k) { self.resumed = false; say(k, v); }
        return;
      }
      if (Array.isArray(v) && path && /fragments$/.test(path)) return fragments(v);
      if (v && typeof v === 'object' && v.response) {
        const r = v.response;
        if (self.resumed) {
          self.resumed = false;
          const parts = { text: '', reasoning: '' };
          if (Array.isArray(r.fragments)) {
            kinds = r.fragments.map((f) => f.type || 'RESPONSE');
            for (const f of r.fragments) { const k = fragKind(f.type); if (k && typeof f.content === 'string') parts[k] += f.content; }
          }
          if (typeof r.thinking_content === 'string') parts.reasoning += r.thinking_content;
          if (typeof r.content === 'string') parts.text += r.content;
          snapshot(parts);
        } else {
          if (Array.isArray(r.fragments)) { kinds = []; fragments(r.fragments); }
          if (typeof r.thinking_content === 'string') say('reasoning', r.thinking_content);
          if (typeof r.content === 'string') say('text', r.content);
        }
        if (typeof r.status === 'string') status(r.status);
      }
    };
    Object.assign(self, {
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
    });
    Object.defineProperty(self, 'done', { get: () => done });
    return self;
  }

  /** Reads an answer's stream. A continued answer passes the reader it started with, so it reads on from there. */
  async function follow(j, res, carried) {
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
    const r = carried || reader(j);
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
      ended(j, r);
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

  /** Notes that the job's send has started, so the fallback click doesn't send it again. */
  const noteRequest = (url) => { if (job && stage === 'sending' && SEND_START.test(url)) job.requested = true; };

  const nativeFetch = window.fetch;
  window.fetch = function (input, init) {
    const url = typeof input === 'string' ? input : input && input.url ? input.url : String(input);
    noteRequest(url);
    // The page continuing a cut-off answer: sent as it is, and read into the same answer.
    if (job && stage === 'continuing' && CONTINUE.test(url)) {
      const j = job;
      const args = arguments;
      const self = this;
      setStage('waiting');
      return (async () => {
        try {
          const res = await nativeFetch.apply(self, args);
          follow(j, res.clone(), j.reader);
          return res;
        } catch (e) {
          finish(j, 'error', `Couldn't reach DeepSeek: ${e && e.message ? e.message : e}`);
          throw e;
        }
      })();
    }
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
    noteRequest(this.__scoutUrl || '');
    const continued = job && stage === 'continuing' && CONTINUE.test(this.__scoutUrl || '') ? job : null;
    const j = continued || armed;
    if (!j || (!continued && !COMPLETION.test(this.__scoutUrl || ''))) return xs.apply(this, arguments);
    if (!continued) {
      armed = null;
      try {
        body = rewrite(j, typeof body === 'string' ? body : body instanceof ArrayBuffer || ArrayBuffer.isView(body) ? new TextDecoder().decode(body) : null);
      } catch {
        finish(j, 'error', "Scout couldn't put its prompt into DeepSeek's request. The site may have changed; check for a Scout update.");
        return this.abort();
      }
    }
    setStage('waiting');
    if (!continued) sent(j);
    const r = continued ? j.reader : reader(j);
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
      else ended(j, r);
    });
    this.addEventListener('error', () => finish(j, 'error', "Couldn't reach DeepSeek."));
    abort = { abort: () => this.abort() };
    return xs.call(this, body);
  };

  // ---------- what Scout and the extension call ----------

  api.start = (j) => start(j);
  api.stop = () => stop();
  api.tick = () => tick();
  api.hold = (on) => hold(!!on);
  api.status = () => ({ v: VERSION, signedIn: signedIn(), signIn: onSignIn(), path: location.pathname, busy: !!job, stage, composer: !!composer(), token: hasToken(), challenge: challenged });
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
