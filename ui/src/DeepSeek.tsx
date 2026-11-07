import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState, useSyncExternalStore } from 'react';
import { CopyButton, Link } from './widgets';

/** DeepSeek Web: the user's chat.deepseek.com account, through Scout's own window or the Scout extension. See deepseek.rs. */
export type DeepSeekStatus = {
  enabled: boolean;
  via: 'window' | 'extension';
  signedIn: boolean;
  windowOpen: boolean;
  extensionConnected: boolean;
  extensionTab: boolean;
  extensionPath: string;
  port: number | null;
  warning: string;
};

export const DEEPSEEK_WEB_NOTE = "Uses your own chat.deepseek.com account, with no API key. Scout sends each request through DeepSeek's website and runs the tools itself, so Scout's tools, MCP servers, permissions and undo all apply. Pick DeepSeek, DeepThink, or either with web search in the model menu.";

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

async function change(enabled: boolean | null, via: 'window' | 'extension' | null): Promise<string | null> {
  try {
    set(await invoke<DeepSeekStatus>('deepseek_set', { enabled, via }));
    return null;
  } catch (e) {
    return String(e);
  }
}

function Dot({ on }: { on: boolean }) {
  return <span className={`ds-dot ${on ? 'on' : ''}`} aria-hidden />;
}

/** Everything for DeepSeek Web: the warning, turning it on, where it runs, and signing in. */
export function DeepSeekWebPanel({ compact = false }: { compact?: boolean }) {
  const s = useDeepSeek();
  const [via, setVia] = useState<'window' | 'extension'>('window');
  const [error, setError] = useState<string | null>(null);
  const [folder, setFolder] = useState<string | null>(null);
  if (!s) return <div className="small dim">Loading…</div>;
  const run = async (p: Promise<string | null>) => setError(await p);
  const openWindow = () => invoke('deepseek_open').catch((e) => setError(String(e)));
  const setUp = () => invoke<string>('deepseek_extension').then(setFolder, (e) => setError(String(e)));
  const warning = <div className="card-note ds-warning"><div className="small"><b className="strong">Your DeepSeek account could be limited or banned.</b> <span className="dim">{s.warning}</span></div></div>;
  if (!s.enabled) {
    return (
      <div className="stack">
        {!compact && <p className="small dim">{DEEPSEEK_WEB_NOTE}</p>}
        {warning}
        <div className="ds-choices" role="radiogroup" aria-label="Where DeepSeek Web runs">
          <button type="button" role="radio" aria-checked={via === 'window'} className={`ds-choice ${via === 'window' ? 'selected' : ''}`} onClick={() => setVia('window')}>
            <b className="strong small">In Scout</b>
            <span className="tiny dim">Scout keeps its own DeepSeek window, hidden while it works. Sign in to it once.</span>
          </button>
          <button type="button" role="radio" aria-checked={via === 'extension'} className={`ds-choice ${via === 'extension' ? 'selected' : ''}`} onClick={() => setVia('extension')}>
            <b className="strong small">In your browser</b>
            <span className="tiny dim">The Scout for DeepSeek extension uses a chat.deepseek.com tab in Chrome or Edge.</span>
          </button>
        </div>
        <button type="button" className="button primary wide" onClick={() => run(change(true, via).then((e) => { if (!e && via === 'window') openWindow(); return e; }))}>
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
      <div className="row wrap">
        <span className="small dim grow">Runs in</span>
        <div className="ds-seg" role="radiogroup" aria-label="Where DeepSeek Web runs">
          {(['window', 'extension'] as const).map((v) => (
            <button key={v} type="button" role="radio" aria-checked={s.via === v} className={`chip ${s.via === v ? 'on' : ''}`} onClick={() => run(change(null, v))}>
              {v === 'window' ? 'Scout' : 'Browser extension'}
            </button>
          ))}
        </div>
      </div>
      {s.via === 'window' ? (
        <>
          <div className="row small"><Dot on={s.signedIn} /><span className="grow">{s.signedIn ? 'Signed in to DeepSeek' : 'Not signed in yet'}</span></div>
          <div className="row wrap">
            <button type="button" className={`button ${s.signedIn ? '' : 'primary'}`} onClick={openWindow}>{s.signedIn ? 'Show DeepSeek' : 'Sign in to DeepSeek'}</button>
          </div>
          <div className="tiny dim">The window hides when you close it, and keeps you signed in. Scout uses it in the background.</div>
        </>
      ) : (
        <>
          <div className="row small"><Dot on={s.extensionConnected} /><span className="grow">{s.extensionConnected ? 'The extension is connected' : 'The extension isn\'t connected'}</span></div>
          <div className="row small"><Dot on={s.signedIn} /><span className="grow">{!s.extensionConnected ? 'Waiting for the extension' : !s.extensionTab ? 'Open chat.deepseek.com in that browser' : s.signedIn ? 'Signed in on chat.deepseek.com' : 'Sign in on the chat.deepseek.com tab'}</span></div>
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
        </>
      )}
      {!compact && <div><button type="button" className="button" onClick={() => run(change(false, null))}>Turn off DeepSeek Web</button></div>}
      {error && <div className="small red">{error}</div>}
    </div>
  );
}
