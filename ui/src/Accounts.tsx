// The sign-in pieces, which the first-run screen, the model menu and Settings all share.
import { useEffect, useRef, useState } from 'react';
import { act, invoke, type CustomProvider } from './api';
import { core, useView } from './store';
import { Link, Select, Spinner } from './widgets';
import scoutClassic from '../../assets/icon.png';

export const COPILOT_NOTE = "Uses your own GitHub account's Copilot plan, from this PC. Each request counts against that plan's allowance, so don't share an account or run it unattended at volume. Signing in downloads about 150 MB.";
export const KIRO_NOTE = "Uses your own Kiro account and plan, from this PC, so your chats go to AWS and its model providers, and Kiro's free plan allows 50 credits a month. Don't share an account. Kiro runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. Signing in downloads about 330 MB and shares the sign-in with any other Kiro CLI on this PC.";
export const ANTIGRAVITY_NOTE = "Uses your own Google account's Antigravity plan, from this PC, so your chats go to Google and count against that account's limits. Scout runs Google's own Antigravity CLI, from the official ACP registry: it does the Google sign-in and keeps the login, and every request to Google comes from it, so Scout never sees the login or calls Google itself. Google decides which apps may drive that CLI, and can limit or turn off accounts it thinks break its terms. Don't share an account. Antigravity runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits. Signing in downloads about 100 MB.";
export const VIBE_NOTE = "Uses your own Mistral account, from this PC, so your chats go to Mistral and count against that account's plan. Signing in goes through Mistral AI Studio in your browser, and the key it makes is kept by Vibe and shared with any other Mistral Vibe on this PC. Its settings, agents and skills in ~/.vibe apply here too. Mistral Vibe runs its own tools: Scout shows them and asks first, but Scout's undo doesn't cover its edits, and Yolo mode lets it act without asking. Signing in downloads about 95 MB.";
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
  antigravity: { name: 'Antigravity', button: 'Sign in with Google' },
  vibe: { name: 'Mistral Vibe', button: 'Sign in with Mistral' },
  codex: { name: 'Codex', button: 'Sign in with ChatGPT' },
  claude: { name: 'Claude Code', button: 'Sign in with Claude' },
  opencode: { name: 'OpenCode', button: 'Sign in to OpenCode' },
  grok: { name: 'Grok Build', button: 'Sign in to Grok' },
  hermes: { name: 'Hermes', button: 'Sign in with Nous Portal' },
};

export type SignInWhich = 'copilot' | 'devin' | 'kiro' | 'antigravity' | 'vibe' | 'codex' | 'claude' | 'opencode' | 'grok' | 'hermes';

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
  const own = logos[`../../assets/providers/${name}.svg`];
  if (own) return own;
  if (name === 'scoutcode') return scoutClassic;
  if (id === CUSTOM_NEW) return letterIcon('+');
  const custom = core.get()?.statics.additionalProviders.find((p) => p.id === id)?.custom;
  return custom ? custom.icon ?? letterIcon(custom.name) : '';
}

const letters = new Map<string, string>();
/** A custom provider without an icon: the first letter of its name on a colour picked from the name. */
export function letterIcon(name: string): string {
  const hit = letters.get(name);
  if (hit) return hit;
  const letter = (name.trim()[0] ?? '?').toUpperCase().replace(/[<>&"']/g, '?');
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="7" fill="hsl(${h} 45% 42%)"/><text x="16" y="21.5" text-anchor="middle" font-family="Segoe UI, sans-serif" font-size="17" font-weight="600" fill="#fff">${letter}</text></svg>`;
  const url = `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
  letters.set(name, url);
  return url;
}

/** Whether an unlinked provider is Free or Paid. */
export function providerStatus(id: string): 'Free' | 'Paid' | 'Your own' {
  if (id === CUSTOM_NEW) return 'Your own';
  return ['claude', 'devin', 'grok', 'deepseek'].includes(id) ? 'Paid' : 'Free';
}

/** Settings' entry for adding a provider of the user's own. */
export const CUSTOM_NEW = 'custom-new';

const PRESETS: { label: string; name: string; kind: CustomProvider['kind']; url: string }[] = [
  { label: 'OpenAI', name: 'OpenAI', kind: 'openai', url: 'https://api.openai.com/v1' },
  { label: 'Anthropic', name: 'Anthropic', kind: 'anthropic', url: 'https://api.anthropic.com/v1' },
  { label: 'Other', name: '', kind: 'openai', url: '' },
];

export const CUSTOM_NOTE = "Any service that speaks OpenAI's or Anthropic's API: OpenAI, Anthropic, Groq, Together, Mistral, or a server on this PC such as Ollama (http://localhost:11434/v1) or LM Studio. Its models bill that service's account, and its key is saved on this PC.";

/** A picture the user chose, made small enough to keep with the settings. */
function shrinkImage(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("That file couldn't be read."));
    reader.onload = () => {
      const data = String(reader.result);
      if (file.type === 'image/svg+xml') {
        if (data.length > 300_000) reject(new Error('That SVG is too big. Pick one under 200 KB.'));
        else resolve(data);
        return;
      }
      const img = new Image();
      img.onerror = () => reject(new Error("That file isn't a picture Scout can show."));
      img.onload = () => {
        const size = 64;
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d')!;
        const scale = Math.min(size / img.width, size / img.height);
        const w = img.width * scale;
        const h = img.height * scale;
        ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
        resolve(canvas.toDataURL('image/png'));
      };
      img.src = data;
    };
    reader.readAsDataURL(file);
  });
}

/**
 * Adding a provider of the user's own, or changing one (`existing`). The icon is the site's favicon, found as the
 * address is typed, unless the user picks a picture or the plain letter.
 */
export function CustomProviderForm({ existing, onDone }: { existing?: CustomProvider; onDone?: () => void }) {
  const [name, setName] = useState(existing?.name ?? '');
  const [kind, setKind] = useState<CustomProvider['kind']>(existing?.kind ?? 'openai');
  const [url, setUrl] = useState(existing?.url ?? '');
  const [key, setKey] = useState('');
  const [icon, setIcon] = useState<string | null>(existing?.icon ?? null);
  // 'site': follow the address's favicon; 'user': a picture they chose, or the letter, so it's left alone.
  const [iconFrom, setIconFrom] = useState<'site' | 'user'>(existing ? 'user' : 'site');
  const [looking, setLooking] = useState(false);
  const [iconNote, setIconNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const asked = useRef(0);

  const suggest = (address: string) => {
    const n = ++asked.current;
    if (!address.trim()) return;
    setLooking(true);
    setIconNote(null);
    invoke<string>('provider_icon', { address: address.trim() })
      .then((data) => { if (n === asked.current) { setIcon(data); setIconFrom('site'); } })
      .catch((e) => { if (n === asked.current) setIconNote(String(e)); })
      .finally(() => { if (n === asked.current) setLooking(false); });
  };
  // While the icon follows the site, it's looked up again once the address stops changing.
  useEffect(() => {
    if (iconFrom !== 'site' || !/^https?:\/\/[^/\s]+/i.test(url.trim())) return;
    const t = window.setTimeout(() => suggest(url), 700);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [url, iconFrom]);

  const preset = (p: typeof PRESETS[number]) => {
    setKind(p.kind);
    setUrl(p.url);
    if (p.name || !name) setName(p.name);
    setIconFrom('site');
    if (!p.url) setIcon(null);
  };
  const choose = (f: File | undefined) => {
    if (!f) return;
    shrinkImage(f).then((data) => { setIcon(data); setIconFrom('user'); setIconNote(null); }, (e: Error) => setIconNote(e.message));
  };
  const save = () => {
    setSaving(true);
    setError(null);
    invoke('act', { action: { type: 'saveCustomProvider', id: existing?.id ?? null, name, kind, url, key: key.trim() || null, icon } })
      .then(() => {
        setKey('');
        if (!existing) { setName(''); setUrl(''); setIcon(null); setIconFrom('site'); }
        onDone?.();
      }, (e) => setError(String(e)))
      .finally(() => setSaving(false));
  };
  const shown = icon ?? letterIcon(name || '?');
  return (
    <div className="stack">
      {!existing && (
        <div className="row wrap">
          {PRESETS.map((p) => (
            <button key={p.label} type="button" className={`button compact ${p.url === url && p.kind === kind ? 'primary' : ''}`} onClick={() => preset(p)}>{p.label}</button>
          ))}
        </div>
      )}
      <input className="input" placeholder="Name, e.g. Groq" value={name} onChange={(e) => setName(e.target.value)} />
      <input className="input" placeholder="API address, e.g. https://api.groq.com/openai/v1" value={url} onChange={(e) => setUrl(e.target.value)} spellCheck={false} />
      <label className="row small">
        <span className="dim">Speaks</span>
        <Select className="input select compact grow" aria-label="API" value={kind} onChange={(e) => setKind(e.target.value as CustomProvider['kind'])}>
          <option value="openai">OpenAI's API (chat completions)</option>
          <option value="anthropic">Anthropic's API</option>
        </Select>
      </label>
      <input className="input" type="password" placeholder={existing ? 'New API key (leave empty to keep the saved one)' : 'API key (leave empty if it takes none)'} value={key} onChange={(e) => setKey(e.target.value)} />
      <div className="row">
        <img className="custom-icon" src={shown} alt="" />
        <div className="grow min0 tiny dim">
          {looking ? "Looking for the site's icon…" : iconNote ?? (iconFrom === 'site' && icon ? "The site's own icon." : icon ? 'Your picture.' : 'The first letter of its name.')}
        </div>
        <button type="button" className="button compact" disabled={!url.trim() || looking} title="Use the favicon of the site this address is on" onClick={() => { setIconFrom('site'); suggest(url); }}>Suggest</button>
        <button type="button" className="button compact" onClick={() => file.current?.click()}>Choose…</button>
        {icon && <button type="button" className="button compact" title="Show the first letter of its name instead" onClick={() => { setIcon(null); setIconFrom('user'); }}>Letter</button>}
        <input ref={file} type="file" accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml,image/x-icon" hidden onChange={(e) => { choose(e.target.files?.[0]); e.target.value = ''; }} />
      </div>
      {error && <div className="small red">{error}</div>}
      <div className="row end">
        {onDone && existing && <button type="button" className="button" onClick={onDone}>Cancel</button>}
        <button type="button" className="button primary" disabled={saving || !name.trim() || !url.trim()} onClick={save}>{saving ? 'Saving…' : existing ? 'Save changes' : 'Add provider'}</button>
      </div>
    </div>
  );
}

/** A custom provider that's been added: what it is, and changing or removing it. */
export function CustomProviderDetails({ p, masked }: { p: CustomProvider; masked: string | null }) {
  const [editing, setEditing] = useState(false);
  if (editing) return <CustomProviderForm existing={p} onDone={() => setEditing(false)} />;
  return (
    <div className="stack">
      <div className="small dim">{p.kind === 'anthropic' ? "Anthropic's API" : "OpenAI's API"} at {p.url}</div>
      <div className="small dim">{masked ? `Key ${masked}` : 'No key'}</div>
      <div className="row">
        <button type="button" className="button" onClick={() => setEditing(true)}>Edit</button>
        <button type="button" className="button" onClick={() => act('unlinkAdditional', { id: p.id })}>Remove</button>
      </div>
    </div>
  );
}

/** Every additional provider not linked yet, and how to link it. */
/** Shown where OpenRouter or an additional provider is linked: their API-key models only run inside a custom agent. */
export const AGENT_ONLY_NOTE = 'These models run on your own API key and can only be used in a custom agent: once linked, add them to a route under Custom agents (+ Add to route).';

export function AdditionalOptions() {
  const { additional } = useView('cfg');
  const providers = core.get().statics.additionalProviders;
  return (
    <div className="stack">
      <div className="small dim">{AGENT_ONLY_NOTE}</div>
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
      <div className="provider-card">
        <div className="row">
          <img className="logo" src={letterIcon('+')} width={18} height={18} alt="" />
          <b className="strong grow">Your own provider</b>
          <span className="badge">OpenAI or Anthropic API</span>
        </div>
        <div className="small dim">{CUSTOM_NOTE}</div>
        <CustomProviderForm />
      </div>
    </div>
  );
}
