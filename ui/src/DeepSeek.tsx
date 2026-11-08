import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState, useSyncExternalStore } from 'react';
import { CopyButton, Link } from './widgets';

/** DeepSeek Web: the user's chat.deepseek.com account, through the Scout for DeepSeek browser extension. See deepseek.rs. */
export type DeepSeekStatus = {
  enabled: boolean;
  signedIn: boolean;
  extensionConnected: boolean;
  extensionTab: boolean;
  extensionPath: string;
  port: number | null;
  pacing: Pacing;
  /** DeepSeek is showing an hCaptcha challenge, which the user has to solve on the page. */
  challenge: boolean;
  /** Scout chats that used DeepSeek Web today (UTC), and how many it allows before asking the user to stop. */
  sessionsToday: number;
  sessionsPerDay: number;
  warning: string;
};

/** How hard Scout paces DeepSeek Web. The waits themselves are set in deepseek.rs and aren't shown. */
export type Pacing = 'passive' | 'moderate' | 'aggressive';

const PACINGS: [Pacing, string, string][] = [
  ['passive', 'Passive', 'Waits the longest between requests. Slowest, and the gentlest on the account.'],
  ['moderate', 'Moderate', 'A balanced pause between requests.'],
  ['aggressive', 'Aggressive', 'Shortest pauses. Fastest, but the most likely to get the account limited.'],
];

export const DEEPSEEK_WEB_NOTE = "Uses your own chat.deepseek.com account, with no API key. Scout sends each request through DeepSeek's website, through the Scout for DeepSeek extension in your browser, and runs the tools itself, so Scout's tools, MCP servers, permissions and undo all apply. Pick DeepSeek, DeepThink, or either with web search in the model menu.";

let status: DeepSeekStatus | null = null;
const listeners = new Set<() => void>();
const set = (s: DeepSeekStatus) => { status = s; listeners.forEach((l) => l()); };

export async function refreshDeepSeek() {
  try { set(await invoke<DeepSeekStatus>('deepseek_status')); } catch {}
}

/** DeepSeek Web's state, kept fresh while anything shows it. */
export function useDeepSeek(): DeepSeekStatus | null {
  useEffect(() => {
    refreshDeepSeek();
    const t = setInterval(refreshDeepSeek, 2500);
    return () => clearInterval(t);
  }, []);
  return useSyncExternalStore((l) => { listeners.add(l); return () => listeners.delete(l); }, () => status);
}

async function change(enabled: boolean): Promise<string | null> {
  try {
    set(await invoke<DeepSeekStatus>('deepseek_set', { enabled }));
    return null;
  } catch (e) {
    return String(e);
  }
}

async function changePacing(pacing: Pacing): Promise<string | null> {
  try {
    set(await invoke<DeepSeekStatus>('deepseek_pacing', { pacing }));
    return null;
  } catch (e) {
    return String(e);
  }
}

function Dot({ on }: { on: boolean }) {
  return <span className={`ds-dot ${on ? 'on' : ''}`} aria-hidden />;
}

/** How hard Scout paces DeepSeek Web: a preset, which sets how long it waits between requests. */
function PacingPresets({ value }: { value: Pacing }) {
  const [error, setError] = useState<string | null>(null);
  const current = PACINGS.find((p) => p[0] === value) ?? PACINGS[1];
  return (
    <div className="stack">
      <div className="row wrap small">
        <span className="small dim grow">Pacing</span>
        <div className="ds-seg" role="radiogroup" aria-label="How hard Scout paces DeepSeek Web">
          {PACINGS.map(([id, label]) => (
            <button key={id} type="button" role="radio" aria-checked={value === id} className={`chip ${value === id ? 'on' : ''}`} onClick={async () => setError(await changePacing(id))}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="tiny dim">{current[2]} Scout waits a random time each request, so they don't come at a steady rhythm.</div>
      {error && <div className="small red">{error}</div>}
    </div>
  );
}

/** Everything for DeepSeek Web: the warning, turning it on, the extension, and the pacing. */
export function DeepSeekWebPanel({ compact = false }: { compact?: boolean }) {
  const s = useDeepSeek();
  const [error, setError] = useState<string | null>(null);
  const [folder, setFolder] = useState<string | null>(null);
  if (!s) return <div className="small dim">Loading…</div>;
  const run = async (p: Promise<string | null>) => setError(await p);
  const setUp = () => invoke<string>('deepseek_extension').then(setFolder, (e) => setError(String(e)));
  const warning = <div className="card-note ds-warning"><div className="small"><b className="strong">Your DeepSeek account could be limited or banned.</b> <span className="dim">{s.warning}</span></div></div>;
  if (!s.enabled) {
    return (
      <div className="stack">
        {!compact && <p className="small dim">{DEEPSEEK_WEB_NOTE}</p>}
        {warning}
        <button type="button" className="button primary wide" onClick={() => run(change(true))}>
          I understand, turn on DeepSeek Web
        </button>
        {error && <div className="small red">{error}</div>}
      </div>
    );
  }
  return (
    <div className="stack">
      {!compact && <p className="small dim">{DEEPSEEK_WEB_NOTE}</p>}
      {!compact && warning}
      <div className="row small"><Dot on={s.extensionConnected} /><span className="grow">{s.extensionConnected ? 'The extension is connected' : 'The extension isn\'t connected'}</span></div>
      <div className="row small"><Dot on={s.signedIn} /><span className="grow">{!s.extensionConnected ? 'Waiting for the extension' : !s.extensionTab ? 'Open chat.deepseek.com in that browser' : s.signedIn ? 'Signed in on chat.deepseek.com' : 'Sign in on the chat.deepseek.com tab'}</span></div>
      {s.challenge && <div className="card-note"><div className="small"><b className="strong">Solve the CAPTCHA to continue.</b> <span className="dim">DeepSeek is showing one on the chat.deepseek.com tab. Scout waits until you solve it.</span></div></div>}
      {!s.extensionConnected && (
        <ol className="small ds-steps">
          <li><button type="button" className="button compact" onClick={setUp}>Set up the extension</button> <span className="dim">writes it to a folder and shows it.</span></li>
          <li>Open <span className="mono">chrome://extensions</span> (or <span className="mono">edge://extensions</span>) and turn on Developer mode.</li>
          <li>Click Load unpacked and pick that folder.</li>
          <li>Open <Link href="https://chat.deepseek.com">chat.deepseek.com</Link>, sign in, and keep the tab open.</li>
        </ol>
      )}
      {(folder || s.extensionConnected) && (
        <div className="row wrap tiny dim"><span className="mono ellipsis grow">{folder ?? s.extensionPath}</span><CopyButton text={folder ?? s.extensionPath} label="Copy path" /></div>
      )}
      {s.extensionConnected && <div className="tiny dim">After a Scout update, click Reload on the extension in your browser's extensions page.</div>}
      <div className="row small">
        <Dot on={s.sessionsToday < s.sessionsPerDay} />
        <span className="grow">{s.sessionsToday >= s.sessionsPerDay ? 'Done for today' : 'Sessions today'}</span>
        <span className="dim">{Math.min(s.sessionsToday, s.sessionsPerDay)} of {s.sessionsPerDay}</span>
      </div>
      <div className="tiny dim">
        {s.sessionsToday >= s.sessionsPerDay
          ? 'DeepSeek limits how much one account can do. Give it a rest: have a few normal chats on chat.deepseek.com, and come back tomorrow. Today\'s chats can carry on.'
          : `Each Scout chat on DeepSeek Web is one session. After ${s.sessionsPerDay} a day, Scout asks you to stop, since DeepSeek rate-limits the website.`}
      </div>
      <PacingPresets value={s.pacing} />
      {!compact && <div><button type="button" className="button" onClick={() => run(change(false))}>Turn off DeepSeek Web</button></div>}
      {error && <div className="small red">{error}</div>}
    </div>
  );
}
