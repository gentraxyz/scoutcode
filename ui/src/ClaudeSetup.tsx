import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { act } from './api';
import { useView } from './store';
import { CopyButton, Spinner } from './widgets';

/** Where the chatbot connector setup is. See claude_setup.rs. */
export type SetupStatus = {
  state: 'idle' | 'waiting' | 'running' | 'done' | 'failed' | 'stopped';
  /** Which chatbot it's adding Scout to. */
  bot?: SetupBot;
  /** native: Scout works the browser itself (Windows). extension: the Scout browser extension does. */
  mode?: 'native' | 'extension';
  paused?: boolean;
  /** Why it's paused. */
  note?: string | null;
  step?: number;
  label?: string;
  error?: string | null;
  extensionSeen?: boolean;
  waitedSecs?: number;
  extensionPath?: string;
};

/** The chatbots Scout can add itself to (claude_setup.rs `Site`). */
export type SetupBot = 'claude' | 'gemini';

/** Each one's name and connector settings (claude_setup.rs opens the same). */
const SITES: Record<SetupBot, { name: string; connectors: string }> = {
  claude: { name: 'Claude', connectors: 'https://claude.ai/new#customize/connectors' },
  gemini: { name: 'Gemini', connectors: 'https://gemini.google.com/customize/connectors' },
};

/** A browser the setup can open (claude_setup.rs `browsers`). */
type Browser = { name: string; path: string; default: boolean };

/** The browser picked here last time: a program path, or '' for the default browser. Only this window's convenience. */
const BROWSER_KEY = 'scout.claudeSetupBrowser';
const savedBrowser = () => { try { return localStorage.getItem(BROWSER_KEY) ?? ''; } catch { return ''; } };
const saveBrowser = (path: string) => { try { localStorage.setItem(BROWSER_KEY, path); } catch { /* not kept */ } };

/** How long Scout waits to hear from the extension before suggesting it isn't installed. */
const NO_EXTENSION_SECS = 20;

/**
 * Adds Scout to claude.ai (or Gemini, with `bot`) as a connector. On Windows Scout opens Claude in the browser and works it with the mouse and
 * keyboard, with its own bar and Pause over the browser. Elsewhere, or when that can't find the browser, the Scout
 * browser extension does it. Signing in to ScoutCode, and the final Link, stay with the user.
 */
export function ClaudeSetup({ address, disabled, bare, bot = 'claude' }: { address: string; disabled?: boolean; bare?: boolean; bot?: SetupBot }) {
  const [job, setS] = useState<SetupStatus>({ state: 'idle' });
  // Scout runs one setup at a time: one for another chatbot shows as nothing here.
  const s: SetupStatus = (job.bot ?? 'claude') === bot ? job : { state: 'idle' };
  const { name: site, connectors: CONNECTORS } = SITES[bot];
  const [error, setError] = useState<string | null>(null);
  const [folder, setFolder] = useState<string | null>(null);
  const link = useView('link');
  const [browsers, setBrowsers] = useState<Browser[]>([]);
  const [browser, setBrowser] = useState(savedBrowser);
  const active = s.state === 'waiting' || s.state === 'running';
  const native = s.mode === 'native';

  useEffect(() => {
    invoke<Browser[]>('claude_setup_browsers').then(setBrowsers, () => {});
  }, []);
  const fallback = browsers.find((b) => b.default);
  const others = browsers.filter((b) => !b.default);
  // A browser picked before that's gone now: back to the default.
  const chosen = browser && browsers.some((b) => b.path === browser) ? browser : '';
  const pick = (path: string) => { setBrowser(path); saveBrowser(path); };

  useEffect(() => {
    let on = true;
    const tick = () => invoke<SetupStatus>('claude_setup_status').then((x) => on && setS(x), () => {});
    tick();
    const t = setInterval(tick, active ? 1000 : 4000);
    return () => { on = false; clearInterval(t); };
  }, [active]);

  const start = () => {
    setError(null);
    // The connector is no use with the link off: turn it on too, in the mode saved for it.
    if (!link.on) act('toggleLink', { mode: '' });
    invoke<SetupStatus>('claude_setup_start', { bot, name: 'Scout', url: address, browser: chosen || null }).then(setS, (e) => setError(String(e)));
  };
  const stop = () => invoke<SetupStatus>('claude_setup_stop').then(setS, () => {});
  const pause = (paused: boolean) => invoke<SetupStatus>('claude_setup_pause', { paused }).then(setS, () => {});
  const setUpExtension = () => invoke<string>('claude_setup_extension').then(setFolder, (e) => setError(String(e)));
  const noExtension = s.mode === 'extension' && s.state === 'waiting' && !s.extensionSeen && (s.waitedSecs ?? 0) >= NO_EXTENSION_SECS;
  const finish = s.label === `Added to ${site}`
    ? `Finish in the ScoutCode window ${site} opened: sign in if it asks, then approve Link.`
    : /^already/i.test(s.label ?? '')
      ? `${site} already has Scout. If it isn't linked yet, connect it there and approve Link in ScoutCode.`
      : 'Click Connect next to Scout on claude.ai, sign in to ScoutCode, then approve Link.';

  return (
    <div className="stack">
      <div className="row">
        {/* `bare`: the place showing it already says what it does. */}
        <div className="stack tight grow min0">
          {!bare && <b className="strong small">Add to {site} for me</b>}
          <span className="tiny dim">{bare
            ? `Opens ${site} and adds Scout while you watch. Hands off the browser until it’s done (Esc pauses).${bot === 'gemini' ? ' It accepts Gemini’s notice and Google’s account link for you;' : ''} You click Link at the end.`
            : `Opens ${site} in your browser and adds the connector while you watch, with Scout's own cursor. The browser ignores your clicks and keys while it works; press Esc or Pause to take over.${bot === 'gemini' ? ' It ticks Gemini’s notice about custom apps and agrees to Google’s account link for you.' : ''} You approve the final Link yourself.`}</span>
        </div>
        {active && native && s.state === 'running' && (
          <button type="button" className="button compact" onClick={() => pause(!s.paused)}>{s.paused ? 'Resume' : 'Pause'}</button>
        )}
        {active
          ? <button type="button" className="button compact" onClick={stop}>Stop</button>
          : <button type="button" className="button compact primary" disabled={disabled} onClick={start}>{s.state === 'done' ? 'Run again' : 'Start'}</button>}
      </div>
      {browsers.length > 0 && (
        <label className="row tiny dim">
          <span>Browser</span>
          <select className="input select compact" value={chosen} disabled={active} onChange={(e) => pick(e.target.value)}>
            <option value="">{fallback ? `Default (${fallback.name})` : 'Default browser'}</option>
            {others.map((b) => <option key={b.path} value={b.path}>{b.name}</option>)}
          </select>
        </label>
      )}
      {s.state === 'waiting' && !noExtension && <div className="row tiny dim"><Spinner />Couldn't find the browser in front, so Scout is waiting for the Scout extension on {site}…</div>}
      {s.state === 'running' && (s.paused
        ? <div className="tiny dim">{s.note || 'Paused.'}</div>
        : <div className="row tiny dim"><Spinner /><span className="ellipsis">{s.label || `Adding Scout on ${site}…`}</span></div>)}
      {s.state === 'done' && (/completely ready/i.test(s.label ?? '')
        ? <div className="tiny green">{site} is completely ready. Scout opened a new chat there: ask it to work on your project.</div>
        : /linked to scout/i.test(s.label ?? '')
        ? <div className="tiny green">{site} is linked to Scout. Turn the link on here, then ask {site} to work on your project.</div>
        : <div className="tiny green">Added to {site}. {s.label && /^waiting/i.test(s.label) ? 'Finish in the ScoutCode window.' : finish}</div>)}
      {s.state === 'failed' && <div className="tiny red">{s.error || 'The setup stopped.'}</div>}
      {s.state === 'stopped' && <div className="tiny dim">Stopped.</div>}
      {noExtension && (
        <div className="card-note">
          <div className="small"><b className="strong">Scout couldn't work the browser itself, and hasn't heard from the Scout extension.</b> <span className="dim">Keep {site} in front and press Start again, or install the extension below and open <span className="mono">{CONNECTORS}</span> in that browser.</span></div>
        </div>
      )}
      <details className="stack tight" open={noExtension || undefined}>
        <summary className="small">If that doesn't work: the Scout browser extension (Chrome or Edge)</summary>
        <ol className="small ds-steps">
          <li><button type="button" className="button compact" onClick={setUpExtension}>Set up the extension</button> <span className="dim">writes it to a folder and shows it.</span></li>
          <li>Open <span className="mono">chrome://extensions</span> or <span className="mono">edge://extensions</span>, and turn on Developer mode.</li>
          <li>Click Load unpacked and pick that folder. If Scout is already there, click its Reload instead.</li>
          <li>Press Start above. If Scout can't work the browser itself, the extension takes over on <span className="mono">{CONNECTORS}</span>.</li>
        </ol>
        {(folder || s.extensionPath) && <div className="row wrap tiny dim"><span className="mono ellipsis grow">{folder ?? s.extensionPath}</span><CopyButton text={(folder ?? s.extensionPath)!} label="Copy path" /></div>}
      </details>
      {error && <div className="tiny red">{error}</div>}
    </div>
  );
}
