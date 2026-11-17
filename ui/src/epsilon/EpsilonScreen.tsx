/// <reference path="./novnc.d.ts" />
// One Epsilon computer on screen: its desktop (noVNC) and terminal (xterm.js), connected straight to the Epsilon
// server's WebSockets with short-lived tokens, and the boot screen shown while it starts, saves or is wiped.
// The desktop app (Cloud.tsx) and the website (src/components/EpsilonWeb.tsx, through preact/compat) both draw it,
// so neither frames a page from the Epsilon server. The parent owns the machine's state (it polls the site) and passes
// it in; the terminal socket also reports state changes as they happen (host.onStatus).
import { useEffect, useMemo, useRef, useState } from 'react';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import './epsilon-screen.css';

export type ScreenKind = 'persistent' | 'temporary';
export type ScreenMachine = {
  kind?: ScreenKind;
  state: 'none' | 'stopped' | 'starting' | 'running' | 'saving' | 'stopping' | 'error';
  type?: string;
  memoryMB?: number | null;
  desktop?: boolean;
  mode?: 'resumed' | 'fresh' | 'booted' | null;
  hasSavedState?: boolean;
  expiresAt?: string | null;
  lowCredits?: boolean;
};
export type ScreenHost = {
  /** The Epsilon server, e.g. https://epsilon.scoutcode.dev. */
  server: string;
  /** A short-lived user token for it (from the site's /api/epsilon/token), or null when signed out. */
  token: () => Promise<string | null>;
  /** The terminal socket said the machine changed (e.g. it stopped on its own). */
  onStatus?: (m: ScreenMachine) => void;
};

const MARK = 'M10 0h14v12h12v14a10 10 0 0 1-10 10H10A10 10 0 0 1 0 26V10A10 10 0 0 1 10 0z';
const BUSY = new Set(['starting', 'saving', 'stopping']);

/** Keeps one token per host until a minute before it expires. */
function tokens(host: ScreenHost) {
  let cached: { token: string; exp: number } | null = null;
  return async () => {
    if (cached && cached.exp - Date.now() > 60_000) return cached.token;
    const t = await host.token();
    if (!t) return null;
    let exp = Date.now() + 5 * 60_000;
    try { exp = JSON.parse(atob(t.split('.')[1].replace(/-/g, '+').replace(/_/g, '/'))).exp * 1000; } catch {}
    cached = { token: t, exp };
    return t;
  };
}

// ---------- the boot screen ----------

type Phase = 'start' | 'connect' | 'save' | 'wipe';

function stepsFor(phase: Phase, m: ScreenMachine, fast: boolean): { title: string; steps: string[]; at: (s: number) => number; expect: number } {
  const temp = m.kind === 'temporary';
  if (phase === 'save') return { title: 'Saving your computer', steps: ['Pausing it', 'Writing memory and files to disk', 'Powering off'], at: (s) => (s < 1 ? 0 : 1), expect: 18 };
  if (phase === 'wipe') return { title: 'Wiping the temporary computer', steps: ['Shutting it down', 'Erasing its disk'], at: (s) => (s < 1.2 ? 0 : 1), expect: 4 };
  const resume = !!m.hasSavedState || m.mode === 'resumed';
  const middle = resume ? 'Restoring where you left off' : fast || m.mode === 'fresh' ? 'Loading a fresh machine' : 'Booting Debian';
  return {
    title: temp ? 'Starting a temporary computer' : resume ? 'Waking your computer' : 'Starting your computer',
    steps: ['Finding room on a server', middle, m.desktop === false ? 'Opening the terminal' : 'Connecting the screen'],
    at: (s) => (phase === 'connect' ? 2 : s < 0.8 ? 0 : 1),
    expect: resume || fast ? 7 : 55,
  };
}

const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s) % 60).padStart(2, '0')}`;
const gb = (mb?: number | null) => (mb ? `${Math.round((mb / 1024) * 10) / 10} GB` : '');

/** The screen shown while a computer starts, connects, saves or is wiped. */
export function EpsilonBoot({ phase, machine, typeName, fast = false, leaving = false }: { phase: Phase; machine: ScreenMachine; typeName?: string; fast?: boolean; leaving?: boolean }) {
  const [t0, setT0] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  // The clock restarts when the computer goes from running to saving or wiping, not between starting and connecting.
  const group = phase === 'start' || phase === 'connect' ? 'up' : phase;
  useEffect(() => { setT0(Date.now()); }, [group]);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(t);
  }, []);
  const s = Math.max(0, (now - t0) / 1000);
  const { title, steps, at, expect } = stepsFor(phase, machine, fast);
  const step = at(s);
  // Never quite full until it's done: it eases toward 92%, and the last step starts from at least 80%.
  let p = 0.92 * (1 - Math.exp(-s / (expect * 0.55)));
  if (phase === 'connect') p = Math.max(p, 0.8 + 0.15 * (1 - Math.exp(-s / 3)));
  if (leaving) p = 1;
  const details = [typeName, gb(machine.memoryMB)].filter(Boolean).join(' · ');
  return (
    <div className={`eps-boot ${phase} ${leaving ? 'leaving' : ''}`} role="status" aria-live="polite">
      <div className="eps-sky" />
      <div className="eps-floor" />
      <div className="eps-core">
        <div className="eps-halo" />
        <div className="eps-ring a" />
        <div className="eps-ring b" />
        <div className="eps-ring c" />
        <svg className="eps-mark" viewBox="-3 -3 42 42" aria-hidden="true"><path d={MARK} /></svg>
      </div>
      <div className="eps-title">{title}</div>
      {details && <div className="eps-details">{details}</div>}
      <ol className="eps-steps">
        {steps.map((label, i) => (
          <li key={label} className={i < step || leaving ? 'done' : i === step ? 'now' : ''}>
            <span className="eps-tick" aria-hidden="true">
              {i < step || leaving ? <svg viewBox="0 0 12 12"><path d="M2.5 6.2l2.3 2.3 4.7-5" /></svg> : null}
            </span>
            {label}
          </li>
        ))}
      </ol>
      <div className="eps-progress"><i style={{ transform: `scaleX(${p})` }} /></div>
      <div className="eps-time">{clock(s)}{phase !== 'connect' && s > expect * 1.6 ? ' · taking longer than usual' : ''}</div>
    </div>
  );
}

// ---------- the computer ----------

type View = 'desktop' | 'term';

export function EpsilonScreen({ kind, machine, host, typeName, fast }: { kind: ScreenKind; machine: ScreenMachine; host: ScreenHost; typeName?: string; fast?: boolean }) {
  const token = useMemo(() => tokens(host), [host.server]);
  const root = useRef<HTMLDivElement>(null);
  const screenEl = useRef<HTMLDivElement>(null);
  const termEl = useRef<HTMLDivElement>(null);
  const running = machine.state === 'running';
  const desktop = machine.desktop !== false;
  const [view, setView] = useState<View>(desktop ? 'desktop' : 'term');
  const [screenUp, setScreenUp] = useState(false);
  const [termUp, setTermUp] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const onStatus = useRef(host.onStatus);
  onStatus.current = host.onStatus;

  useEffect(() => { if (!desktop) setView('term'); }, [desktop]);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  const wsUrl = async (what: 'vnc' | 'term') => {
    const t = await token();
    if (!t) throw new Error('You’re signed out. Sign in again to use your computer.');
    return `${host.server.replace(/\/+$/, '').replace(/^http/, 'ws')}/ws/machines/${kind}/${what}?token=${encodeURIComponent(t)}`;
  };

  // The desktop: one RFB connection while it runs, reconnecting if it drops.
  useEffect(() => {
    if (!running || !desktop || !screenEl.current) return;
    let alive = true, rfb: any = null, retry = 0, timer = 0;
    const el = screenEl.current;
    const connect = async () => {
      try {
        const [{ default: RFB }, url] = await Promise.all([import('@novnc/novnc'), wsUrl('vnc')]);
        if (!alive) return;
        rfb = new RFB(el, url, { wsProtocols: ['binary'] });
        rfb.scaleViewport = true;
        rfb.background = '#000';
        rfb.focusOnClick = true;
        rfb.addEventListener('connect', () => { retry = 0; setScreenUp(true); setProblem(null); });
        rfb.addEventListener('disconnect', () => {
          rfb = null;
          setScreenUp(false);
          if (alive) timer = window.setTimeout(connect, Math.min(8000, 800 * 2 ** retry++));
        });
      } catch (e) {
        if (!alive) return;
        setProblem(e instanceof Error ? e.message : String(e));
        timer = window.setTimeout(connect, 4000);
      }
    };
    connect();
    return () => {
      alive = false;
      clearTimeout(timer);
      setScreenUp(false);
      if (rfb) { const r = rfb; rfb = null; try { r.disconnect(); } catch {} }
      el.replaceChildren();
    };
  }, [running, desktop, kind, host.server]);

  // The terminal: made once, connected while it runs. Its socket also reports the machine's state.
  const term = useRef<{ t: Terminal; fit: FitAddon; ws: WebSocket | null } | null>(null);
  useEffect(() => {
    if (!termEl.current) return;
    const t = new Terminal({
      cursorBlink: true, fontSize: 13.5, scrollback: 5000, allowProposedApi: false,
      fontFamily: '"Cascadia Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      theme: { background: '#0b0d10', foreground: '#d8dee9', cursor: '#7dd3fc', selectionBackground: '#2b3b55' },
    });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(termEl.current);
    term.current = { t, fit, ws: null };
    t.onData((d) => { const ws = term.current?.ws; if (ws && ws.readyState === 1) ws.send(JSON.stringify({ t: 'in', d })); });
    const ro = new ResizeObserver(() => { try { fit.fit(); } catch {} });
    ro.observe(termEl.current);
    return () => { ro.disconnect(); t.dispose(); term.current = null; };
  }, []);
  useEffect(() => {
    if (!running) return;
    let alive = true, timer = 0, retry = 0;
    const connect = async () => {
      let url: string;
      try { url = await wsUrl('term'); } catch (e) { if (alive) { setProblem(e instanceof Error ? e.message : String(e)); timer = window.setTimeout(connect, 4000); } return; }
      if (!alive || !term.current) return;
      const ws = new WebSocket(url);
      ws.binaryType = 'arraybuffer';
      term.current.ws = ws;
      ws.onopen = () => { retry = 0; setTermUp(true); };
      ws.onmessage = (ev) => {
        if (typeof ev.data !== 'string') return term.current?.t.write(new Uint8Array(ev.data));
        try {
          const msg = JSON.parse(ev.data);
          if (msg.t === 'status' && msg.machine) onStatus.current?.(msg.machine);
        } catch {}
      };
      ws.onclose = () => {
        if (term.current?.ws === ws) term.current.ws = null;
        setTermUp(false);
        if (alive) timer = window.setTimeout(connect, Math.min(8000, 800 * 2 ** retry++));
      };
    };
    connect();
    return () => {
      alive = false;
      clearTimeout(timer);
      const ws = term.current?.ws;
      if (term.current) term.current.ws = null;
      ws?.close();
      term.current?.t.reset();
      setTermUp(false);
    };
  }, [running, kind, host.server]);

  useEffect(() => {
    if (view !== 'term' || !term.current) return;
    const t = setTimeout(() => { try { term.current?.fit.fit(); } catch {} term.current?.t.focus(); }, 0);
    return () => clearTimeout(t);
  }, [view, termUp]);

  const ready = running && (view === 'desktop' ? screenUp : termUp);
  // Once it has been up, a dropped connection shows a small note rather than the whole boot screen.
  const [wasUp, setWasUp] = useState(false);
  useEffect(() => { if (ready) setWasUp(true); if (!running) setWasUp(false); }, [ready, running]);
  const phase: Phase | null = machine.state === 'saving' ? 'save' : machine.state === 'stopping' ? 'wipe' : machine.state === 'starting' ? 'start' : running && !ready && !wasUp ? 'connect' : null;
  // Keep the boot screen a moment after it's up, so it fades out instead of vanishing.
  const [fading, setFading] = useState(false);
  const lastPhase = useRef<Phase | null>(null);
  useEffect(() => {
    if (phase) { lastPhase.current = phase; setFading(false); return; }
    if (!lastPhase.current) return;
    setFading(true);
    const t = setTimeout(() => { setFading(false); lastPhase.current = null; }, 650);
    return () => clearTimeout(t);
  }, [phase]);

  const left = running && machine.expiresAt ? Math.max(0, Date.parse(machine.expiresAt) - now) / 1000 : null;
  const full = () => {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else root.current?.requestFullscreen?.().catch(() => {});
  };
  const shownPhase = phase ?? (fading ? lastPhase.current : null);

  return (
    <div className="eps" ref={root}>
      <div className="eps-bar">
        <div className="eps-tabs" role="tablist">
          {desktop && <button type="button" role="tab" aria-selected={view === 'desktop'} onClick={() => setView('desktop')}>Desktop</button>}
          <button type="button" role="tab" aria-selected={view === 'term'} onClick={() => setView('term')}>Terminal</button>
        </div>
        <span className={`eps-live ${ready ? 'up' : BUSY.has(machine.state) || (running && !ready) ? 'busy' : ''}`} />
        <span className="eps-dim">{ready ? 'Connected' : running ? 'Connecting…' : machine.state === 'starting' ? 'Starting…' : machine.state === 'saving' ? 'Saving…' : machine.state === 'stopping' ? 'Wiping…' : ''}</span>
        <span className="eps-grow" />
        {left !== null && <span className={`eps-dim eps-num ${left < 300 ? 'eps-warn-text' : ''}`}>{clock(left)} left</span>}
        <button type="button" className="eps-icon" title="Full screen" aria-label="Full screen" onClick={full}>
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" /></svg>
        </button>
      </div>
      {machine.lowCredits && running && (
        <div className="eps-warn">
          {kind === 'temporary' ? 'You’re nearly out of Epsilon credits. This temporary computer is wiped when they run out.' : 'You’re nearly out of Epsilon credits. Your computer is saved and stopped when they run out.'}
        </div>
      )}
      {problem && running && <div className="eps-warn bad" role="alert">{problem}</div>}
      {running && wasUp && !ready && <div className="eps-reconnect">Reconnecting…</div>}
      <div className="eps-stage">
        <div ref={screenEl} className="eps-screen" hidden={view !== 'desktop'} tabIndex={0} />
        <div ref={termEl} className="eps-term" hidden={view !== 'term'} />
        {shownPhase && <EpsilonBoot phase={shownPhase} machine={machine} typeName={typeName} fast={fast} leaving={!phase} />}
      </div>
    </div>
  );
}
