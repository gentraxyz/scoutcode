// The sign-in pieces, which the first-run screen, the model menu and Settings all share.
import { useState } from 'react';
import { act } from './api';
import { core, useView } from './store';
import { Link, Spinner } from './widgets';

export const COPILOT_NOTE = "Uses your own GitHub account's Copilot plan, from this PC. Each request counts against that plan's allowance, so don't share an account or run it unattended at volume. Signing in downloads about 150 MB.";
export const KIRO_NOTE = "Uses your own Kiro account and plan, from this PC, so your chats go to AWS and its model providers, and Kiro's free plan allows 50 credits a month. Don't share an account. Kiro runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. Signing in downloads about 330 MB and shares the sign-in with any other Kiro CLI on this PC.";
export const DEVIN_NOTE = "Uses your own Devin account and plan, from this PC, so your chats go to Cognition and its model providers. Don't share an account. Scout's permissions and undo cover Devin's edits and commands; its searches run inside Devin. Signing in downloads about 75 MB and shares the sign-in with any other Devin CLI on this PC.";

/** A password field and a button under it. */
export function KeyField({ hint, button, onSave }: { hint: string; button: string; onSave: (key: string) => void }) {
  const [key, setKey] = useState('');
  const save = () => {
    if (key.trim()) {
      onSave(key.trim());
      setKey('');
    }
  };
  return (
    <div className="stack">
      <input className="input" type="password" placeholder={hint} value={key} onChange={(e) => setKey(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && save()} />
      <button type="button" className="button wide" disabled={!key.trim()} onClick={save}>{button}</button>
    </div>
  );
}

export function BrowserEntry() {
  const login = useView('login');
  return (
    <div className="stack">
      <button type="button" className="button primary wide tall" disabled={login.busy} onClick={() => act('browserLogin')}>Sign in with your browser</button>
      {login.busy && <div className="row"><Spinner /><span className="muted small">Waiting for the browser…</span></div>}
      {login.busy && login.url && <Link href={login.url} className="small">Open the sign-in page again</Link>}
    </div>
  );
}

export function ScoutKeyEntry() {
  const login = useView('login');
  return (
    <div className="stack">
      <KeyField hint="sk-…" button="Save key" onSave={(key) => act('saveApiKey', { key })} />
      <Link href="https://scoutcode.dev/account" className="small">Make a key on your account page</Link>
      {login.error && <div className="red small">{login.error}</div>}
    </div>
  );
}

export function NvidiaEntry() {
  return (
    <div className="stack">
      <Link href="https://build.nvidia.com" className="small">Make a key at build.nvidia.com</Link>
      <KeyField hint="nvapi-…" button="Save NVIDIA key" onSave={(key) => act('saveNvidiaKey', { key })} />
    </div>
  );
}

const SIGN_INS = {
  copilot: { name: 'Copilot', button: 'Sign in with GitHub' },
  devin: { name: 'Devin', button: 'Sign in to Devin' },
  kiro: { name: 'Kiro', button: 'Sign in to Kiro' },
};

/** The sign-in button, or the code and the download's progress while it's going on. */
export function SignInEntry({ which }: { which: 'copilot' | 'devin' | 'kiro' }) {
  const login = useView('login');
  const s = login[which];
  const error = login[`${which}Error`];
  const { name, button } = SIGN_INS[which];
  if (!s) {
    return (
      <div className="stack">
        <button type="button" className="button wide" onClick={() => act('signIn', { which })}>{button}</button>
        {error && <div className="red small">{error}</div>}
      </div>
    );
  }
  const line = s.installed ? `${name} is installed.`
    : s.waiting ? 'Finish signing in with the browser window that opened.'
    : s.total > 0 ? `Installing ${name}… ${Math.floor((s.done * 100) / s.total)}%`
    : `Installing ${name}…`;
  return (
    <div className="stack">
      {s.code ? (
        <>
          <div className="sign-in-code">{s.code}</div>
          <div className="small dim">Enter this code on GitHub to sign in. Your browser should have opened the page.</div>
          {s.url && <Link href={s.url} className="small">Open the page again</Link>}
        </>
      ) : <Spinner />}
      <div className="small muted">{line}</div>
      <button type="button" className="button" onClick={() => act('cancelSignIn', { which })}>Cancel</button>
    </div>
  );
}

function AdditionalKey({ id }: { id: string }) {
  const a = core.get().statics.additionalProviders.find((p) => p.id === id)!;
  return (
    <div className="stack">
      <Link href={a.keyUrl} className="small">{a.keyLink}</Link>
      <KeyField hint={a.keyHint} button="Save key" onSave={(key) => act('linkAdditional', { id, key })} />
    </div>
  );
}

export { AdditionalKey };

/** Logos for the additional providers, and for the model menu's rail. */
const logos = import.meta.glob('../../assets/providers/*.svg', { eager: true, query: '?url', import: 'default' }) as Record<string, string>;
export function logo(id: string): string {
  return logos[`../../assets/providers/${id}.svg`] ?? '';
}

/** Every additional provider not linked yet, and how to link it. */
export function AdditionalOptions() {
  const { additional } = useView('cfg');
  const providers = core.get().statics.additionalProviders;
  return (
    <div className="stack">
      {providers.filter((a) => !additional.some((l) => l.id === a.id)).map((a) => (
        <div key={a.id} className="provider-card">
          <div className="row"><img className="logo" src={logo(a.id)} width={18} height={18} alt="" /><b className="strong">{a.name}</b></div>
          <div className="small dim">{a.note}</div>
          {a.needsKey
            ? <AdditionalKey id={a.id} />
            : <button type="button" className="button wide" onClick={() => act('linkAdditional', { id: a.id, key: null })}>Use {a.name} for free</button>}
        </div>
      ))}
    </div>
  );
}
