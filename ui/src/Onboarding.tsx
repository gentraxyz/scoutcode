// The first-run screen: a row of every provider's logo, grouped by what it takes to start. The user can link as many
// as they like: the ones that need no login link as soon as they're picked, and the rest once the user signs in or
// saves a key. Each linked one gets a checkmark.
import { useState, type ReactNode } from 'react';
import { act, type View } from './api';
import {
  AdditionalKey, BrowserEntry, CLAUDE_NOTE, CODEX_NOTE, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GUEST_NOTE, GrokEntry, HERMES_NOTE, HermesEntry, KeyField,
  KIRO_NOTE, logo, NvidiaEntry, OPENCODE_NOTE, ScoutKeyEntry, SignInEntry,
} from './Accounts';
import { core, useCore, useView } from './store';
import { Icon, Link } from './widgets';
import scoutLogo from '../../assets/icon.png';

type Group = 'open' | 'login' | 'setup';
type Cfg = View['cfg'];

interface Provider {
  id: string;
  name: string;
  group: Group;
  linked: (c: Cfg) => boolean;
  /** For the ones that need no login: links it as soon as it's picked. */
  link?: () => void;
  unlink?: () => void;
}

const additional = (id: string) => (c: Cfg) => c.additional.some((a) => a.id === id);

const PROVIDERS: Provider[] = [
  { id: 'scoutcode', name: 'ScoutCode', group: 'open', linked: (c) => c.hasKey || c.guest, link: () => act('setGuest', { on: true }), unlink: () => act('setGuest', { on: false }) },
  { id: 'ovh', name: 'OVH Cloud', group: 'open', linked: additional('ovh'), link: () => act('linkAdditional', { id: 'ovh', key: null }), unlink: () => act('unlinkAdditional', { id: 'ovh' }) },
  { id: 'kilo', name: 'Kilo Code', group: 'open', linked: (c) => c.kilo, link: () => act('saveKilo', { enabled: true, key: null }), unlink: () => act('saveKilo', { enabled: false, key: null }) },
  { id: 'opencode', name: 'OpenCode', group: 'open', linked: (c) => c.opencode, link: () => act('setOpencode', { on: true, uninstall: false }), unlink: () => act('setOpencode', { on: false, uninstall: false }) },
  { id: 'openrouter', name: 'OpenRouter', group: 'login', linked: (c) => c.openrouterKeys.length > 0 },
  { id: 'nvidia', name: 'NVIDIA NIM', group: 'login', linked: (c) => !!c.nvidiaKey },
  { id: 'google', name: 'Google AI Studio', group: 'login', linked: additional('google') },
  { id: 'hermes', name: 'Hermes', group: 'login', linked: (c) => c.hermesSignedIn || !!c.hermesKey },
  { id: 'claude', name: 'Claude Code', group: 'setup', linked: (c) => c.claude },
  { id: 'codex', name: 'Codex', group: 'setup', linked: (c) => c.codex },
  { id: 'copilot', name: 'GitHub Copilot', group: 'setup', linked: (c) => c.copilot },
  { id: 'grok', name: 'Grok Build', group: 'setup', linked: (c) => c.grokSignedIn || !!c.grokKey },
  { id: 'kiro', name: 'Kiro', group: 'setup', linked: (c) => c.kiro },
  { id: 'devin', name: 'Devin', group: 'setup', linked: (c) => c.devin },
];

const GROUPS: [Group, string][] = [['open', 'No login'], ['login', 'Login'], ['setup', 'Login with setup']];

function providerLogo(id: string) {
  return id === 'scoutcode' ? scoutLogo : logo(id);
}

function Tile({ p, linked, focused, onSelect }: { p: Provider; linked: boolean; focused: boolean; onSelect: () => void }) {
  return (
    <button type="button" className={`provider-tile${focused ? ' focused' : ''}${linked ? ' linked' : ''}`} aria-pressed={linked} title={p.name} onClick={onSelect}>
      <img src={providerLogo(p.id)} width={30} height={30} alt="" />
      <span className="ellipsis">{p.name}</span>
      {linked && <span className="tile-check"><Icon name="check" size={12} /></span>}
    </button>
  );
}

function Linked({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="linked-line">
      <Icon name="check" size={16} />
      <div className="stack tight">
        <b className="strong">{title}</b>
        {children && <span className="small dim">{children}</span>}
      </div>
    </div>
  );
}

/** ScoutCode Cloud's terms: one line, with the main points folded away under it. */
function CloudTerms() {
  return (
    <details className="terms small">
      <summary>
        By using ScoutCode Cloud, you agree to the{' '}
        <Link href="https://scoutcode.dev/terms">Terms of Service</Link> and <Link href="https://scoutcode.dev/privacy">Privacy Policy</Link>.
      </summary>
      <ul>
        <li>Requests are answered by ScoutCode's model providers. To keep things working and within limits, a request may be answered by a different model or provider than the one you picked.</li>
        <li>ScoutCode may keep traces of requests and responses to run and improve the service.</li>
        <li>Providers handle requests under their own terms and may keep some of what you send, and some free models may train on it. Don't send secrets.</li>
        <li>Check what models write before you rely on it. Free use has limits, which can change.</li>
      </ul>
    </details>
  );
}

function HigherLimits({ children }: { children: ReactNode }) {
  return (
    <div className="stack">
      <div className="setup-label">Want higher limits?</div>
      {children}
    </div>
  );
}

/** What the picked provider needs, or what it's linked with. */
function Setup({ p, linked }: { p: Provider; linked: boolean }) {
  const cfg = useView('cfg');
  const { kiloWarning, additionalProviders } = useCore((s) => s.statics);
  const note = (id: string) => additionalProviders.find((a) => a.id === id)?.note;
  const done = (what: string) => <Linked title={`Linked to ${p.name}`}>{what}</Linked>;
  switch (p.id) {
    case 'scoutcode':
      if (cfg.hasKey) return done(cfg.browserSignIn ? `Signed in${cfg.username ? ` as ${cfg.username}` : ''}.` : 'Using your ScoutCode key.');
      return (
        <>
          <Linked title="Linked to ScoutCode Cloud">{GUEST_NOTE}</Linked>
          <CloudTerms />
          <HigherLimits>
            <div className="small dim">Sign in to use your account's monthly desktop credit, or paste a key.</div>
            <BrowserEntry compact />
            <ScoutKeyEntry />
          </HigherLimits>
        </>
      );
    case 'ovh':
      return (
        <>
          <Linked title="Linked to OVH Cloud">{note('ovh')}</Linked>
          {!cfg.additional.find((a) => a.id === 'ovh')?.masked && <HigherLimits><AdditionalKey id="ovh" /></HigherLimits>}
        </>
      );
    case 'kilo':
      return (
        <>
          <Linked title="Linked to Kilo Code">Free models with no account or key. {kiloWarning}</Linked>
          {cfg.kiloKey ? <div className="small dim">Using your Kilo key {cfg.kiloKey} for its paid models.</div> : (
            <HigherLimits>
              <div className="small dim">A key from your Kilo account adds its paid models.</div>
              <KeyField hint="Kilo API key" button="Save Kilo key" onSave={(key) => act('saveKilo', { enabled: true, key })} />
            </HigherLimits>
          )}
        </>
      );
    case 'opencode':
      return (
        <>
          <Linked title="Linked to OpenCode">{OPENCODE_NOTE}</Linked>
          {cfg.opencodeSignedIn ? <div className="small dim">Signed in to your OpenCode account for its paid models.</div> : <HigherLimits><SignInEntry which="opencode" compact /></HigherLimits>}
        </>
      );
    case 'openrouter':
      if (linked) return done('Using your OpenRouter key.');
      return (
        <>
          <div className="small dim">Uses your own OpenRouter key, saved on this PC.</div>
          <KeyField hint="sk-or-…" button="Save OpenRouter key" onSave={(key) => act('addOpenrouterKey', { name: 'OpenRouter', key })} />
        </>
      );
    case 'nvidia':
      if (linked) return done('Using your NVIDIA NIM key.');
      return <><div className="small dim">Uses your own NVIDIA NIM key, saved on this PC.</div><NvidiaEntry /></>;
    case 'google':
      if (linked) return done('Using your Google AI Studio key.');
      return <><div className="small dim">{note('google')}</div><AdditionalKey id="google" /></>;
    case 'hermes':
      if (linked) return done(cfg.hermesSignedIn ? 'Signed in to Nous Portal.' : 'Using your Nous Portal key.');
      return <><div className="small dim">{HERMES_NOTE}</div><HermesEntry /></>;
    case 'copilot':
      if (linked) return done(cfg.copilotUser ? `Signed in to GitHub as ${cfg.copilotUser}.` : 'Signed in to GitHub.');
      return <><div className="small dim">{COPILOT_NOTE}</div><SignInEntry which="copilot" /></>;
    case 'grok':
      if (linked) return done(cfg.grokSignedIn ? 'Signed in to Grok.' : 'Using your xAI key.');
      return <><div className="small dim">{GROK_NOTE}</div><GrokEntry /></>;
    case 'claude':
      if (linked) return done('Signed in to Claude Code.');
      return <><div className="small dim">{CLAUDE_NOTE}</div><SignInEntry which="claude" /></>;
    case 'codex':
      if (linked) return done('Signed in to Codex.');
      return <><div className="small dim">{CODEX_NOTE}</div><SignInEntry which="codex" /></>;
    case 'kiro':
      if (linked) return done('Signed in to Kiro.');
      return <><div className="small dim">{KIRO_NOTE}</div><SignInEntry which="kiro" /></>;
    case 'devin':
      if (linked) return done('Signed in to Devin.');
      return <><div className="small dim">{DEVIN_NOTE}</div><SignInEntry which="devin" /></>;
    default:
      return null;
  }
}

export function Onboarding({ onDone }: { onDone: () => void }) {
  const cfg = useView('cfg');
  const [focused, setFocused] = useState<string | null>(null);
  // Only offer the additional providers this build knows about.
  const known = (p: Provider) => !['ovh', 'google'].includes(p.id) || core.get().statics.additionalProviders.some((a) => a.id === p.id);
  const shown = PROVIDERS.filter(known);
  const count = shown.filter((p) => p.linked(cfg)).length;
  const p = shown.find((x) => x.id === focused);
  const pick = (x: Provider) => {
    setFocused(x.id);
    if (x.link && !x.linked(cfg)) x.link();
  };
  return (
    <>
      <p className="muted onboarding-ask">Pick where Scout gets its models. Link as many as you like.</p>
      <div className="provider-picker">
        <div className="picker-label free">Free</div>
        <div className="picker-groups">
          {GROUPS.map(([group, label]) => (
            <div key={group} className="picker-group">
              <div className="picker-label">{label}</div>
              <div className="picker-tiles">
                {shown.filter((x) => x.group === group).map((x) => (
                  <Tile key={x.id} p={x} linked={x.linked(cfg)} focused={focused === x.id} onSelect={() => pick(x)} />
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
      {p ? (
        <div className="login-card stack setup-card">
          <div className="row">
            <img src={providerLogo(p.id)} width={22} height={22} alt="" />
            <b className="strong big grow">{p.name}</b>
            {p.unlink && p.linked(cfg) && !(p.id === 'scoutcode' && cfg.hasKey) && <button type="button" className="button" onClick={() => { p.unlink!(); setFocused(null); }}>Remove</button>}
          </div>
          <Setup p={p} linked={p.linked(cfg)} />
        </div>
      ) : <p className="small dim onboarding-hint">Select one to set it up.</p>}
      <div className="onboarding-done">
        <button type="button" className="button primary wide tall" disabled={count === 0} onClick={onDone}>
          {count === 0 ? 'Link a provider to continue' : `Continue with ${count} provider${count === 1 ? '' : 's'}`}
        </button>
        <div className="tiny dim">You can link more later in Settings.</div>
      </div>
    </>
  );
}
