// The sign-in pieces, which the first-run screen, the model menu and Settings all share.
import { useState } from 'react';
import { act } from './api';
import { core, useView } from './store';
import { Link, Spinner } from './widgets';
import scoutClassic from '../../assets/icon.png';

export const COPILOT_NOTE = "Uses your own GitHub account's Copilot plan, from this PC. Each request counts against that plan's allowance, so don't share an account or run it unattended at volume. Signing in downloads about 150 MB.";
export const KIRO_NOTE = "Uses your own Kiro account and plan, from this PC, so your chats go to AWS and its model providers, and Kiro's free plan allows 50 credits a month. Don't share an account. Kiro runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. Signing in downloads about 330 MB and shares the sign-in with any other Kiro CLI on this PC.";
export const CLAUDE_NOTE = "Uses your own Claude Pro or Max plan, from this PC, so your chats go to Anthropic and count against that plan's limits. Don't share an account. Claude Code runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. It doesn't load a project's Claude Code settings, hooks or MCP servers. Signing in installs Node.js and Claude Code, about 450 MB, and keeps the sign-in apart from any other Claude Code on this PC.";
export const CODEX_NOTE = "Uses your own ChatGPT plan, from this PC, so your chats go to OpenAI and count against that plan's Codex limits. Don't share an account. Codex runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode gives it full access. Signing in installs Node.js and Codex, about 500 MB, and keeps the sign-in apart from any other Codex on this PC.";
export const OPENCODE_WARNING = "Some of OpenCode's free models may keep your prompts or use them to improve the model, so don't send them anything private.";
export const OPENCODE_NOTE = `Its free models need no account, and signing in to your OpenCode account adds the paid ones, billed by OpenCode. ${OPENCODE_WARNING} OpenCode runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. It downloads about 80 MB, and keeps its sign-in and settings apart from any other OpenCode on this PC.`;
export const GROK_NOTE = "Uses your own Grok sign-in, or an xAI API key billed per token, from this PC, so your chats go to xAI. Don't share an account. Grok Build runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. It doesn't read a project's AGENTS.md or other Grok settings, and won't run in your home folder. It downloads about 50 MB, and keeps its sign-in and settings apart from any other Grok on this PC.";
export const HERMES_WARNING = "Nous may use what you send to train its models unless Privacy Mode is on for your Portal account, so don't send anything private.";
export const HERMES_NOTE = `Uses your own Nous Portal account or key, from this PC, so your chats go to Nous and its model providers. Models marked Free cost nothing, and the rest use your Portal credit. ${HERMES_WARNING} Scout's own tools, permissions and undo all apply, and nothing is installed.`;
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

/** `compact` is for an optional sign-in, when the provider already works without one. */
export function BrowserEntry({ compact = false }: { compact?: boolean }) {
  const login = useView('login');
  return (
    <div className="stack">
      <button type="button" className={compact ? 'button compact' : 'button primary wide tall'} disabled={login.busy} onClick={() => act('browserLogin')}>Sign in with your browser</button>
      {login.busy && <div className="row"><Spinner /><span className="muted small">Waiting for the browser…</span></div>}
      {login.busy && login.url && <Link href={login.url} className="small">Open the sign-in page again</Link>}
    </div>
  );
}

export const GUEST_NOTE = 'Free models only, with far fewer requests a day than signing in.';

/** Using ScoutCode's free models without an account. */
export function GuestEntry() {
  return (
    <div className="stack">
      <button type="button" className="button wide" onClick={() => act('setGuest', { on: true })}>Continue without an account</button>
      <div className="tiny dim">{GUEST_NOTE}</div>
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

/** Turning on OpenCode for its free models, or signing in for the rest. Either one downloads its CLI. */
export function OpencodeEntry() {
  const login = useView('login');
  if (login.opencode) return <SignInEntry which="opencode" />;
  return (
    <div className="stack">
      <button type="button" className="button wide" onClick={() => act('setOpencode', { on: true, uninstall: false })}>Use OpenCode's free models</button>
      <SignInEntry which="opencode" />
    </div>
  );
}

/** Signing in to Grok, or an API key instead. Either one downloads its CLI. */
export function GrokEntry() {
  const login = useView('login');
  if (login.grok) return <SignInEntry which="grok" />;
  return (
    <div className="stack">
      <SignInEntry which="grok" />
      <GrokKeyEntry />
    </div>
  );
}

export function GrokKeyEntry() {
  return (
    <div className="stack">
      <div className="small dim">Or use an API key:</div>
      <Link href="https://console.x.ai" className="small">Make a key at console.x.ai</Link>
      <KeyField hint="xai-…" button="Save xAI key" onSave={(key) => act('saveGrokKey', { key })} />
    </div>
  );
}

/** Signing in to Nous Portal, or an API key instead. */
export function HermesEntry() {
  const login = useView('login');
  if (login.hermes) return <SignInEntry which="hermes" />;
  return (
    <div className="stack">
      <SignInEntry which="hermes" />
      <HermesKeyEntry />
    </div>
  );
}

export function HermesKeyEntry() {
  return (
    <div className="stack">
      <div className="small dim">Or use an API key:</div>
      <Link href="https://portal.nousresearch.com" className="small">Make a key on Nous Portal</Link>
      <KeyField hint="sk-…" button="Save Nous Portal key" onSave={(key) => act('saveHermesKey', { key })} />
    </div>
  );
}

const SIGN_INS = {
  copilot: { name: 'Copilot', button: 'Sign in with GitHub' },
  devin: { name: 'Devin', button: 'Sign in to Devin' },
  kiro: { name: 'Kiro', button: 'Sign in to Kiro' },
  codex: { name: 'Codex', button: 'Sign in with ChatGPT' },
  claude: { name: 'Claude Code', button: 'Sign in with Claude' },
  opencode: { name: 'OpenCode', button: 'Sign in to OpenCode' },
  grok: { name: 'Grok Build', button: 'Sign in to Grok' },
  hermes: { name: 'Hermes', button: 'Sign in with Nous Portal' },
};

export type SignInWhich = 'copilot' | 'devin' | 'kiro' | 'codex' | 'claude' | 'opencode' | 'grok' | 'hermes';

/** The sign-in button, or the code and the download's progress while it's going on. */
export function SignInEntry({ which, compact = false }: { which: SignInWhich; compact?: boolean }) {
  const login = useView('login');
  const s = login[which];
  const error = login[`${which}Error`];
  const { name, button } = SIGN_INS[which];
  if (!s) {
    return (
      <div className="stack">
        <button type="button" className={compact ? 'button compact' : 'button wide'} onClick={() => act('signIn', { which })}>{button}</button>
        {error && <div className="red small">{error}</div>}
      </div>
    );
  }
  const line = (which === 'opencode' || which === 'grok' || which === 'hermes') && s.code ? 'Waiting for you to approve the code.'
    : which === 'hermes' ? 'Contacting Nous Portal…'
    : s.installed ? `${name} is installed.`
    : s.waiting ? 'Finish signing in with the browser window that opened.'
    : s.total > 0 ? `Installing ${name}… ${Math.floor((s.done * 100) / s.total)}%`
    : `Installing ${name}…`;
  return (
    <div className="stack">
      {s.code ? (
        <>
          <div className="sign-in-code">{s.code}</div>
          <div className="small dim">Enter this code on {which === 'opencode' ? 'OpenCode' : which === 'grok' ? 'xAI' : which === 'hermes' ? 'Nous Portal' : 'GitHub'} to sign in. Your browser should have opened the page.</div>
          {s.url && <Link href={s.url} className="small">Open the page again</Link>}
        </>
      ) : <Spinner />}
      {!s.code && s.waiting && s.url && <Link href={s.url} className="small">Open the sign-in page again</Link>}
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
  const name = id === 'scout' ? 'scoutcode' : id;
  return logos[`../../assets/providers/${name}.svg`] ?? (name === 'scoutcode' ? scoutClassic : '');
}

/** Whether an unlinked provider is Free or Paid. */
export function providerStatus(id: string): 'Free' | 'Paid' {
  return ['claude', 'codex', 'devin', 'grok', 'deepseek'].includes(id) ? 'Paid' : 'Free';
}

/** Every additional provider not linked yet, and how to link it. */
export function AdditionalOptions() {
  const { additional } = useView('cfg');
  const providers = core.get().statics.additionalProviders;
  return (
    <div className="stack">
      {providers.filter((a) => !additional.some((l) => l.id === a.id)).map((a) => {
        const status = providerStatus(a.id);
        return (
          <div key={a.id} className="provider-card">
            <div className="row">
              <img className="logo" src={logo(a.id)} width={18} height={18} alt="" />
              <b className="strong grow">{a.name}</b>
              <span className={`badge ${status === 'Paid' ? 'paid' : ''}`}>{status}</span>
            </div>
            <div className="small dim">{a.note}</div>
            {a.needsKey
              ? <AdditionalKey id={a.id} />
              : <button type="button" className="button wide" onClick={() => act('linkAdditional', { id: a.id, key: null })}>Use {a.name} for free</button>}
          </div>
        );
      })}
    </div>
  );
}
