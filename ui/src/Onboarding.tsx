// Multi-step onboarding flow:
// 0. Welcome: Scout logo, "Welcome to Scout", and "The gateway to free AI models"
// 1. Language: Apple-style searchable list for the interface language (English full, the rest partial), plus AI reply language
// 2. What Makes Scout Different (memory that stays updated, pooled free models, all plans, customization, project manager, remote)
// 3. ScoutCode: sign in (with Scout logo), or continue without an account
// 4. Link Providers (free and paid providers) and Use via another chatbot (with step-by-step image guide & clear account requirement)
// 5. Project Setup (defaults to ~/Scout; preloads detected chats on mount and groups them by project, defaulting to Skip)
// 6. Appearance (theme, accent, window material, logo — with Classic logo fixed)
// 7. General Options (sectioned cards with icons: personal instructions, ScoutMote, Discord RPC, Games, Notifications & updates)
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { act, invoke, openExternal, type Games, type View } from './api';
import { closeGame } from './Games';
import {
  AdditionalKey, BrowserEntry, CLAUDE_NOTE, CODEX_NOTE, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GrokEntry, HERMES_NOTE, HermesEntry, KeyField,
  KIRO_NOTE, logo, NvidiaEntry, OPENCODE_NOTE, ScoutKeyEntry, SignInEntry,
} from './Accounts';
import { isFull, replyLanguageFor, setUiLanguage, useTranslation, SUPPORTED_LANGUAGES, type TranslationKey } from './i18n';
import { ScoutLogo, LOGOS, logoSvg } from './logo';
import { core, ui, useCore, useView } from './store';
import { BACKGROUNDS, ELEMENTS, MATERIALS, getMaterialError, setTheme, useTheme } from './theme';
import { WelcomeIntro } from './WelcomeIntro';
import { CopyButton, Icon, Link, SectionLabel, Spinner, Toggle } from './widgets';
import mcpGuidePlaceholder from '../../assets/mcp-guide-placeholder.png';

type Group = 'open' | 'login' | 'setup';
type Tag = 'free' | 'paid';
type Cfg = View['cfg'];
type StepId = 'language' | 'special' | 'scout' | 'providers' | 'project' | 'appearance' | 'options';

const STEPS: { id: StepId; labelKey: TranslationKey }[] = [
  { id: 'language', labelKey: 'step_language' },
  { id: 'special', labelKey: 'step_why' },
  { id: 'scout', labelKey: 'step_scoutcode' },
  { id: 'providers', labelKey: 'step_providers' },
  { id: 'project', labelKey: 'step_project' },
  { id: 'appearance', labelKey: 'step_appearance' },
  { id: 'options', labelKey: 'step_options' },
];

// ---------- Shared frame ----------

const StepCtx = createContext({ index: 1, total: STEPS.length });

interface Next { label: string; onClick: () => void; disabled?: boolean; ghost?: boolean }

/** One step: a centered head and body that scroll when the window is small, over a footer that never moves. */
function StepFrame({ title, sub, wide, children, back, next }: {
  title: string; sub?: ReactNode; wide?: boolean; children: ReactNode; back?: () => void; next: Next;
}) {
  const { index, total } = useContext(StepCtx);
  const { t } = useTranslation();
  return (
    <div className="ob-frame">
      <div className="ob-scroll">
        <div className={`ob-body${wide ? ' wide' : ''}`} key={title}>
          <div className="ob-head">
            <h2>{title}</h2>
            {sub && <p className="small dim">{sub}</p>}
          </div>
          {children}
        </div>
      </div>
      <footer className="ob-footer">
        {back ? <button type="button" className="button tall ob-back" onClick={back}>{t('btn_back')}</button> : <span />}
        <span className="tiny dim ob-count">{t('step_counter', { index, total })}</span>
        <button type="button" className={`button tall ob-next${next.ghost ? '' : ' primary'}`} disabled={next.disabled} onClick={next.onClick}>
          <span className="ellipsis">{next.label}</span>
        </button>
      </footer>
    </div>
  );
}

// ---------- Preferences (instructions.json) ----------

type Prefs = { global: string; project: string; language: string };

/** Reloads first, so a step only changes what it means to and never writes back another project's instructions. */
async function savePrefs(cwd: string, patch: Partial<Prefs>) {
  const p = await invoke<Prefs>('instructions_load', { cwd });
  await invoke('instructions_save', { cwd, ...p, ...patch });
}

// ---------- Providers ----------

interface Provider {
  id: string;
  name: string;
  group: Group;
  tag: Tag;
  linked: (c: Cfg) => boolean;
  /** For the ones that need no login: links it as soon as it's picked. */
  link?: () => void;
  unlink?: () => void;
}

const additional = (id: string) => (c: Cfg) => c.additional.some((a) => a.id === id);

const PROVIDERS: Provider[] = [
  { id: 'ovh', name: 'OVH Cloud', group: 'open', tag: 'free', linked: additional('ovh'), link: () => act('linkAdditional', { id: 'ovh', key: null }), unlink: () => act('unlinkAdditional', { id: 'ovh' }) },
  { id: 'kilo', name: 'Kilo Code', group: 'open', tag: 'free', linked: (c) => c.kilo, link: () => act('saveKilo', { enabled: true, key: null }), unlink: () => act('saveKilo', { enabled: false, key: null }) },
  { id: 'opencode', name: 'OpenCode', group: 'open', tag: 'free', linked: (c) => c.opencode, link: () => act('setOpencode', { on: true, uninstall: false }), unlink: () => act('setOpencode', { on: false, uninstall: false }) },
  { id: 'openrouter', name: 'OpenRouter', group: 'login', tag: 'free', linked: (c) => c.openrouterKeys.length > 0 },
  { id: 'nvidia', name: 'NVIDIA NIM', group: 'login', tag: 'free', linked: (c) => !!c.nvidiaKey },
  { id: 'google', name: 'Google AI Studio', group: 'login', tag: 'free', linked: additional('google') },
  { id: 'hermes', name: 'Hermes', group: 'login', tag: 'free', linked: (c) => c.hermesSignedIn || !!c.hermesKey },
  { id: 'claude', name: 'Claude Code', group: 'setup', tag: 'paid', linked: (c) => c.claude },
  { id: 'codex', name: 'Codex', group: 'setup', tag: 'paid', linked: (c) => c.codex },
  { id: 'copilot', name: 'GitHub Copilot', group: 'setup', tag: 'free', linked: (c) => c.copilot },
  { id: 'grok', name: 'Grok Build', group: 'setup', tag: 'paid', linked: (c) => c.grokSignedIn || !!c.grokKey },
  { id: 'kiro', name: 'Kiro', group: 'setup', tag: 'free', linked: (c) => c.kiro },
  { id: 'devin', name: 'Devin', group: 'setup', tag: 'paid', linked: (c) => c.devin },
];

const GROUPS: [Group, string][] = [['open', 'No login'], ['login', 'Login'], ['setup', 'Login with setup']];

/** External chatbots that can use Scout's tools here through the MCP link. */
interface WebBot {
  id: string;
  name: string;
  logo: string;
  url?: string;
  aliases: string[];
  steps: { title: string; detail: string; imageCaption: string }[];
}

const WEB_BOTS: WebBot[] = [
  {
    id: 'chatgpt',
    name: 'ChatGPT',
    logo: 'codex',
    url: 'https://chatgpt.com',
    aliases: ['chatgpt', 'openai'],
    steps: [
      { title: 'Enable Developer mode in ChatGPT', detail: 'Open Settings → Apps & Connectors → Advanced settings, and turn on Developer mode.', imageCaption: 'ChatGPT → Settings → Apps & Connectors' },
      { title: 'Create a custom MCP connector', detail: 'Return to Apps & Connectors, click Create, and paste your Scout MCP address below.', imageCaption: 'Paste the Scout MCP URL into the connector dialog' },
      { title: 'Sign in and approve Scout', detail: 'Complete the ScoutCode sign-in prompt in ChatGPT, then ask it to read or edit files in your project.', imageCaption: 'Approve ScoutCode OAuth in your browser' },
    ],
  },
  {
    id: 'gemini',
    name: 'Gemini',
    logo: 'gemini',
    url: 'https://gemini.google.com',
    aliases: ['gemini', 'google'],
    steps: [
      { title: 'Open Gemini connector settings', detail: 'In Gemini, open Extensions / Connectors and choose to add a custom remote MCP server.', imageCaption: 'Gemini → Extensions / Custom MCP connector' },
      { title: 'Paste your Scout MCP address', detail: 'Name the connector Scout and paste the MCP endpoint URL shown below.', imageCaption: 'Add Scout MCP server URL' },
      { title: 'Sign in and allow access', detail: 'Sign in with your ScoutCode account when prompted so Gemini can run tools here.', imageCaption: 'Authorize Gemini to use Scout' },
    ],
  },
  {
    id: 'grok-web',
    name: 'Grok',
    logo: 'grok',
    url: 'https://grok.com',
    aliases: ['grok', 'xai'],
    steps: [
      { title: 'Open Grok integrations', detail: 'In Grok, open Settings → Integrations / Connectors and add a custom MCP server.', imageCaption: 'Grok → Settings → Custom connector' },
      { title: 'Paste your Scout MCP address', detail: 'Enter Scout as the name and paste the MCP endpoint URL below.', imageCaption: 'Configure Scout MCP endpoint in Grok' },
      { title: 'Sign in and allow access', detail: 'Approve the ScoutCode sign-in prompt and start chatting with your local project.', imageCaption: 'Confirm ScoutCode access' },
    ],
  },
  {
    id: 'claude-web',
    name: 'Claude',
    logo: 'claude',
    url: 'https://claude.ai',
    aliases: ['claude', 'anthropic'],
    steps: [
      { title: 'Open Claude Connectors', detail: 'In claude.ai, open Settings → Connectors and click Add custom connector.', imageCaption: 'claude.ai → Settings → Connectors' },
      { title: 'Add your Scout MCP URL', detail: 'Name it Scout, paste the MCP address below, and click Add.', imageCaption: 'Add custom connector modal in Claude' },
      { title: 'Connect your ScoutCode account', detail: 'Click Connect next to Scout, sign in to ScoutCode, and allow access to your workspace.', imageCaption: 'Authorize Claude connector with ScoutCode' },
    ],
  },
  {
    id: 'any',
    name: 'Any chatbot',
    logo: 'mcp',
    aliases: [],
    steps: [
      { title: 'Find custom MCP / connector settings', detail: 'Open your chatbot or client settings where remote MCP servers or custom connectors are added.', imageCaption: 'Custom MCP server settings in your client' },
      { title: 'Add the Scout MCP endpoint', detail: 'Create a new connector and paste the Scout MCP URL below.', imageCaption: 'Enter the remote MCP server URL' },
      { title: 'Sign in when prompted', detail: 'Sign in with your ScoutCode account over OAuth. For local apps like Cursor or Claude Desktop, see Settings → Chatbots.', imageCaption: 'Complete OAuth sign-in to link your project' },
    ],
  },
];

function TagPill({ tag }: { tag: Tag }) {
  return <span className={`ob-tag ${tag}`}>{tag === 'free' ? 'Free' : 'Paid'}</span>;
}

function Tile({ name, img, tag, linked, focused, onSelect }: { name: string; img: string; tag: Tag; linked: boolean; focused: boolean; onSelect: () => void }) {
  return (
    <button type="button" className={`provider-tile${focused ? ' focused' : ''}${linked ? ' linked' : ''}`} aria-pressed={focused} title={name} onClick={onSelect}>
      <img src={img} width={28} height={28} alt="" />
      <span className="ellipsis">{name}</span>
      <TagPill tag={tag} />
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

/** Whether an external chatbot has reached Scout through the MCP link. */
function botLinked(b: WebBot, link: View['link']) {
  if (!link.on) return false;
  if (b.id === 'any') return link.chatbots.length > 0;
  return link.chatbots.some((c) => b.aliases.some((a) => c.name.toLowerCase().includes(a)));
}

/** Step-by-step setup guide for connecting another chatbot over MCP, with a clear account gate when signed out. */
function BrowserSetup({ b }: { b: WebBot }) {
  const cfg = useView('cfg');
  const link = useView('link');
  const address = `${cfg.baseUrl}/mcp`;
  const targetName = b.id === 'any' ? 'another chatbot' : b.name;
  const tone = link.online ? 'on' : link.on ? 'wait' : 'off';
  const status = link.online ? 'Connected' : link.on ? 'Connecting…' : 'Off';

  return (
    <div className="stack">
      {!cfg.hasKey && (
        <div className="ob-account-gate stack">
          <div className="row">
            <span className="ob-gate-icon"><Icon name="lock" size={15} /></span>
            <div className="stack tight grow min0">
              <b className="strong">ScoutCode account required</b>
              <span className="tiny dim">Free · Nothing is spent on ScoutCode</span>
            </div>
          </div>
          <div className="small muted">
            Connecting {targetName} to your local project routes MCP tool calls through your ScoutCode account. Sign in first to turn on the bridge.
          </div>
          <BrowserEntry />
          <details className="ob-key">
            <summary className="tiny dim"><Icon name="key" size={12} />Or paste a ScoutCode API key</summary>
            <ScoutKeyEntry />
          </details>
        </div>
      )}

      <div className={`ob-link-bar row${!cfg.hasKey ? ' disabled' : ''}`}>
        <span className={`link-dot ${tone}`} aria-hidden />
        <div className="stack tight grow min0">
          <b className="strong small">MCP chatbot bridge ({status})</b>
          <span className="tiny dim ellipsis" title={link.cwd}>Folder: {link.cwd || 'your project folder'}</span>
        </div>
        <button
          type="button"
          className={`button compact${link.on ? '' : ' primary'}`}
          disabled={!cfg.hasKey || (!link.on && !link.cwd)}
          onClick={() => act('toggleLink', { mode: 'default' })}
        >
          {link.on ? 'Turn off' : 'Turn on'}
        </button>
      </div>

      <div className="ob-guide-head row">
        <span className="section-label grow">Step-by-step setup guide</span>
        {b.url && (
          <button type="button" className="button compact" onClick={() => openExternal(b.url!)}>
            <Icon name="browser" size={13} />Open {b.name}
          </button>
        )}
      </div>

      <ol className="ob-steps-list">
        {b.steps.map((step, i) => (
          <li key={step.title}>
            <b className="strong">{step.title}</b>
            <div className="small dim">{step.detail}</div>
            {i === 1 && (
              <div className="row address">
                <span className="mono small ellipsis grow" title={address}>{address}</span>
                <CopyButton text={address} />
              </div>
            )}
            <figure className="ob-guide-shot">
              <img src={mcpGuidePlaceholder} alt={step.imageCaption} />
              <figcaption className="tiny dim">{step.imageCaption}</figcaption>
            </figure>
          </li>
        ))}
      </ol>
    </div>
  );
}

// ---------- Step 1: Language (Apple-style searchable tall list) ----------

function LanguageStep({ onNext }: { onNext: () => void }) {
  const { cwd } = useView('chat');
  const coreLanguages = useCore((s) => s.statics?.languages);
  const { t, lang: uiLanguage } = useTranslation();
  const [reply, setReply] = useState(replyLanguageFor(uiLanguage));
  const [query, setQuery] = useState('');

  const languages = useMemo(() => {
    if (coreLanguages && coreLanguages.length > 0) {
      const map = new Map<string, string>(coreLanguages);
      for (const [id, label] of SUPPORTED_LANGUAGES) {
        if (!map.has(id)) map.set(id, label);
      }
      return [...map.entries()];
    }
    return SUPPORTED_LANGUAGES;
  }, [coreLanguages]);

  useEffect(() => {
    invoke<Prefs>('instructions_load', { cwd }).then((p) => {
      if (p.language !== replyLanguageFor(uiLanguage)) setReply(p.language);
    }).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const pick = (id: string) => {
    setUiLanguage(id);
    setReply(replyLanguageFor(id));
  };

  const handleContinue = () => {
    savePrefs(cwd, { language: reply }).catch(() => {});
    onNext();
  };

  const named = useMemo(() => {
    const q = query.trim().toLowerCase();
    return languages
      .filter(([id]) => id !== 'auto')
      .filter(([id, label]) => !q || id.toLowerCase().includes(q) || label.toLowerCase().includes(q));
  }, [languages, query]);

  return (
    <StepFrame
      title={t('lang_title')}
      sub={t('lang_sub')}
      next={{ label: t('btn_continue'), onClick: handleContinue }}
    >
      <div className="ob-lang-picker">
        <div className="ob-lang-search">
          <Icon name="search" size={14} />
          <input
            type="text"
            placeholder={t('lang_search_ph')}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={t('lang_search_aria')}
          />
          {query && (
            <button type="button" className="icon-button" style={{ width: 22, height: 22 }} aria-label={t('lang_clear_aria')} onClick={() => setQuery('')}>
              <Icon name="close" size={12} />
            </button>
          )}
        </div>

        <div className="ob-lang-list" role="radiogroup" aria-label={t('step_language')}>
          {named.length === 0 ? (
            <div className="ob-lang-empty small dim">{t('lang_no_matches')}</div>
          ) : named.map(([id, label]) => {
            const selected = id === uiLanguage;
            const full = isFull(id);
            return (
              <button
                key={id}
                type="button"
                role="radio"
                aria-checked={selected}
                className={`ob-lang-row${selected ? ' selected' : ''}`}
                onClick={() => pick(id)}
              >
                <div className="ob-lang-text min0 grow">
                  <span className="ob-lang-native ellipsis">{label}</span>
                  {label !== id && <span className="ob-lang-en small dim ellipsis">{id}</span>}
                </div>
                <span className={`ob-lang-tag ${full ? 'full' : 'partial'}`}>
                  {full ? t('lang_tag_full') : t('lang_tag_partial')}
                </span>
                <span className="ob-lang-check" aria-hidden>
                  {selected && <Icon name="check" size={14} />}
                </span>
              </button>
            );
          })}
        </div>

        <div className="ob-lang-foot">
          <div className="row">
            <Icon name="chat" size={14} />
            <span className="small grow">{t('lang_replies_in')}</span>
            <select className="input select" value={reply} onChange={(e) => setReply(e.target.value)}>
              {languages.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
            </select>
          </div>
          {!isFull(uiLanguage) && (
            <div className="tiny dim">
              {t('lang_partial_note', { lang: uiLanguage })}
            </div>
          )}
        </div>
      </div>
    </StepFrame>
  );
}

// ---------- Step 2: What Makes Scout Different ----------

const SPECIAL_FEATURE_KEYS: { icon: string; titleKey: TranslationKey; descKey: TranslationKey }[] = [
  { icon: 'memory', titleKey: 'special_memory_title', descKey: 'special_memory_desc' },
  { icon: 'bolt', titleKey: 'special_models_title', descKey: 'special_models_desc' },
  { icon: 'layers', titleKey: 'special_allai_title', descKey: 'special_allai_desc' },
  { icon: 'sliders', titleKey: 'special_custom_title', descKey: 'special_custom_desc' },
  { icon: 'board', titleKey: 'special_pm_title', descKey: 'special_pm_desc' },
  { icon: 'phone', titleKey: 'special_remote_title', descKey: 'special_remote_desc' },
];

function SpecialStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const { t } = useTranslation();
  return (
    <StepFrame
      title={t('special_title')}
      sub={t('special_sub')}
      wide
      back={onBack}
      next={{ label: t('btn_continue'), onClick: onNext }}
    >
      <div className="ob-special-grid">
        {SPECIAL_FEATURE_KEYS.map((f) => (
          <div key={f.titleKey} className="ob-special-card">
            <div className="ob-special-top row">
              <span className="ob-feature-icon"><Icon name={f.icon} size={16} /></span>
              <b className="strong">{t(f.titleKey)}</b>
            </div>
            <p className="small dim">{t(f.descKey)}</p>
          </div>
        ))}
      </div>
    </StepFrame>
  );
}

// ---------- Step 3: ScoutCode (before other logins, skippable) ----------

function ScoutLoginStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const cfg = useView('cfg');
  const { t } = useTranslation();
  const skip = () => {
    if (!cfg.hasKey) act('setGuest', { on: true });
    onNext();
  };
  return (
    <StepFrame
      title={t('scout_title')}
      sub={t('scout_sub')}
      wide
      back={onBack}
      next={cfg.hasKey ? { label: t('btn_continue'), onClick: onNext } : { label: t('btn_skip'), onClick: skip, ghost: true }}
    >
      {cfg.hasKey ? (
        <div className="ob-card narrow stack">
          <div className="row">
            <ScoutLogo size={32} />
            <div className="stack tight grow min0">
              <b className="strong">{cfg.username ? t('scout_signed_in_as', { name: cfg.username }) : t('scout_signed_in')}</b>
              <span className="tiny dim">{cfg.browserSignIn ? t('scout_credit_ready') : t('scout_api_key_ready')}</span>
            </div>
            <button type="button" className="button compact" onClick={() => act('signOut')}>{t('btn_sign_out')}</button>
          </div>
        </div>
      ) : (
        <div className="ob-choices">
          <div className="ob-choice recommended">
            <div className="ob-choice-head row">
              <ScoutLogo size={32} />
              <div className="stack tight grow min0">
                <b className="strong big">{t('scout_signin_title')}</b>
                <span className="tiny dim">{t('scout_signin_sub')}</span>
              </div>
              <span className="ob-choice-badge">{t('scout_recommended')}</span>
            </div>
            <ul className="ob-perks">
              <li className="ob-perk"><Icon name="credit" size={14} /><span>{t('scout_perk_credit')}</span></li>
              <li className="ob-perk"><Icon name="layers" size={14} /><span>{t('scout_perk_limits')}</span></li>
              <li className="ob-perk"><Icon name="gift" size={14} /><span>{t('scout_perk_rewards')}</span></li>
              <li className="ob-perk"><Icon name="phone" size={14} /><span>{t('scout_perk_sync')}</span></li>
            </ul>
            <div className="ob-choice-action stack">
              <BrowserEntry />
              <details className="ob-key">
                <summary className="small dim"><Icon name="key" size={13} />{t('scout_api_key_toggle')}</summary>
                <ScoutKeyEntry />
              </details>
            </div>
          </div>
          <div className="ob-choice">
            <div className="ob-choice-head row">
              <span className="ob-guest-icon"><Icon name="user" size={16} /></span>
              <div className="stack tight grow min0">
                <b className="strong big">{t('scout_guest_title')}</b>
                <span className="tiny dim">{t('scout_guest_sub')}</span>
              </div>
            </div>
            <ul className="ob-perks quiet">
              <li className="ob-perk"><Icon name="bolt" size={14} /><span>{t('scout_guest_p1')}</span></li>
              <li className="ob-perk"><Icon name="user" size={14} /><span>{t('scout_guest_p2')}</span></li>
              <li className="ob-perk"><Icon name="layers" size={14} /><span>{t('scout_guest_p3')}</span></li>
              <li className="ob-perk"><Icon name="key" size={14} /><span>{t('scout_guest_p4')}</span></li>
            </ul>
            <div className="ob-choice-action">
              <button type="button" className={`button wide${cfg.guest ? ' selected' : ''}`} onClick={skip}>
                {cfg.guest && <Icon name="check" size={14} />}{t('btn_use_free')}
              </button>
            </div>
          </div>
        </div>
      )}
      <div className="ob-fineprint">
        <CloudTerms />
        <span className="tiny dim">{t('scout_team_hint')}</span>
      </div>
    </StepFrame>
  );
}

// ---------- Step 4: Providers to Link ----------

function ProvidersStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const cfg = useView('cfg');
  const link = useView('link');
  const { t } = useTranslation();
  const [focused, setFocused] = useState<string | null>(null);
  const known = (p: Provider) => !['ovh', 'google'].includes(p.id) || core.get().statics.additionalProviders.some((a) => a.id === p.id);
  const shown = PROVIDERS.filter(known);
  const count = shown.filter((p) => p.linked(cfg)).length;
  const p = shown.find((x) => x.id === focused);
  const bot = WEB_BOTS.find((b) => b.id === focused);
  const pick = (x: Provider) => {
    setFocused(x.id);
    if (x.link && !x.linked(cfg)) x.link();
  };
  const handleNext = () => {
    if (count === 0 && !cfg.hasKey && !cfg.guest) act('setGuest', { on: true });
    onNext();
  };
  const groupLabel = (g: Group) => {
    if (g === 'open') return t('prov_group_open');
    if (g === 'login') return t('prov_group_login');
    return t('prov_group_setup');
  };
  const nextLabel = count === 0
    ? (cfg.hasKey ? t('btn_continue') : t('prov_continue_free'))
    : t('prov_continue_count', { count, s: count === 1 ? '' : 's' });

  return (
    <StepFrame
      title={t('prov_title')}
      sub={t('prov_sub')}
      wide
      back={onBack}
      next={{ label: nextLabel, onClick: handleNext }}
    >
      <div className="ob-providers">
        <div className="ob-provider-groups">
          {GROUPS.map(([group]) => (
            <div key={group} className="picker-group">
              <div className="picker-label">{groupLabel(group)}</div>
              <div className="picker-tiles">
                {shown.filter((x) => x.group === group).map((x) => (
                  <Tile key={x.id} name={x.name} img={logo(x.id)} tag={x.tag} linked={x.linked(cfg)} focused={focused === x.id} onSelect={() => pick(x)} />
                ))}
              </div>
            </div>
          ))}
          <div className="picker-group browser">
            <div className="picker-label row"><Icon name="chat" size={12} />{t('prov_chatbot_group')}</div>
            <div className="picker-tiles">
              {WEB_BOTS.map((b) => (
                <Tile key={b.id} name={b.name} img={logo(b.logo)} tag="free" linked={botLinked(b, link)} focused={focused === b.id} onSelect={() => setFocused(b.id)} />
              ))}
            </div>
          </div>
        </div>

        <div className="login-card stack setup-card">
          {p ? (
            <>
              <div className="row">
                <img src={logo(p.id)} width={22} height={22} alt="" />
                <b className="strong big grow ellipsis">{p.name}</b>
                <TagPill tag={p.tag} />
                {p.unlink && p.linked(cfg) && (
                  <button type="button" className="button compact" onClick={() => { p.unlink!(); setFocused(null); }}>{t('prov_btn_remove')}</button>
                )}
              </div>
              <Setup p={p} linked={p.linked(cfg)} />
            </>
          ) : bot ? (
            <>
              <div className="row">
                <img src={logo(bot.logo)} width={22} height={22} alt="" />
                <b className="strong big grow ellipsis">{bot.id === 'any' ? t('prov_via_any') : t('prov_via_chatbot', { name: bot.name })}</b>
                <TagPill tag="free" />
              </div>
              <div className="small dim">
                {t('prov_chatbot_desc', { name: bot.id === 'any' ? 'another chatbot' : bot.name })}
              </div>
              <BrowserSetup b={bot} />
            </>
          ) : (
            <div className="ob-empty">
              <Icon name="plus" size={20} />
              <span className="small dim">{t('prov_empty_title')}</span>
              <span className="tiny dim">{t('prov_empty_sub')}</span>
            </div>
          )}
        </div>
      </div>
    </StepFrame>
  );
}

// ---------- Step 5: Project Creation + Preloaded Project-Level Import ----------

export type DetectedChat = {
  id: string;
  source: 'claude' | 'codex';
  cwd: string;
  project: string;
  title: string;
  messages: number;
  updated: number;
  ago: string;
};

type Place = { name: string; path: string; icon: string };
export type Places = { home: string; places: Place[]; drives: Place[] };

type DetectedProject = {
  key: string;
  cwd: string;
  project: string;
  chatIds: string[];
  sources: ('claude' | 'codex')[];
  messages: number;
  ago: string;
};

function groupDetectedByProject(chats: DetectedChat[]): DetectedProject[] {
  const map = new Map<string, DetectedProject>();
  for (const c of chats) {
    const key = c.cwd.replace(/\\/g, '/').toLowerCase();
    const existing = map.get(key);
    if (existing) {
      existing.chatIds.push(c.id);
      if (!existing.sources.includes(c.source)) existing.sources.push(c.source);
      existing.messages += c.messages;
    } else {
      map.set(key, {
        key,
        cwd: c.cwd,
        project: c.project || c.cwd,
        chatIds: [c.id],
        sources: [c.source],
        messages: c.messages,
        ago: c.ago,
      });
    }
  }
  return [...map.values()];
}

function ProjectStep({
  places,
  detected,
  setDetected,
  onBack,
  onNext,
}: {
  places: Places | null;
  detected: DetectedChat[];
  setDetected: (next: DetectedChat[]) => void;
  onBack: () => void;
  onNext: () => void;
}) {
  const chat = useView('chat');
  const [projectName, setProjectName] = useState(chat.project);
  const [creatingIn, setCreatingIn] = useState<string | null>(null);
  const [newFolderName, setNewFolderName] = useState('');
  const [folderError, setFolderError] = useState<string | null>(null);

  // Default to Skip: no projects are pre-selected unless the user chooses them.
  const [selectedProjects, setSelectedProjects] = useState<Set<string>>(() => new Set());
  const [importing, setImporting] = useState(false);
  const [importedCount, setImportedCount] = useState<number | null>(null);

  const projects = useMemo(() => groupDetectedByProject(detected), [detected]);

  useEffect(() => {
    setProjectName(chat.project);
  }, [chat.project, chat.cwd]);

  const saveProjectName = (name: string) => {
    setProjectName(name);
    if (name.trim()) {
      act('renameProject', { path: chat.cwd, name: name.trim() });
    }
  };

  const createNewProjectFolder = async () => {
    const parent = creatingIn || chat.cwd;
    const trimmed = newFolderName.trim();
    if (!trimmed) return;
    setFolderError(null);
    try {
      const made = await invoke<string>('folder_create', { parent, name: trimmed });
      await invoke('add_project', { dir: made });
      setNewFolderName('');
      setCreatingIn(null);
    } catch (e) {
      setFolderError(String(e));
    }
  };

  const toggleProject = (key: string) => {
    const next = new Set(selectedProjects);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setSelectedProjects(next);
  };

  const selectedChatIds = useMemo(
    () => projects.filter((p) => selectedProjects.has(p.key)).flatMap((p) => p.chatIds),
    [projects, selectedProjects],
  );

  const importSelected = async () => {
    if (selectedChatIds.length === 0 || importing) return;
    setImporting(true);
    try {
      const count = await invoke<number>('import_external_chats', { ids: selectedChatIds });
      setImportedCount((prev) => (prev ?? 0) + count);
      const importedSet = new Set(selectedChatIds);
      setDetected(detected.filter((c) => !importedSet.has(c.id)));
      setSelectedProjects(new Set());
    } finally {
      setImporting(false);
    }
  };

  const handleContinue = async () => {
    if (projectName.trim() && projectName.trim() !== chat.project) {
      act('renameProject', { path: chat.cwd, name: projectName.trim() });
    }
    if (selectedChatIds.length > 0 && !importing) {
      await importSelected();
    }
    onNext();
  };

  const { t } = useTranslation();
  const nextLabel = selectedProjects.size > 0
    ? t('proj_next_import', { count: selectedProjects.size, s: selectedProjects.size === 1 ? '' : 's' })
    : projects.length > 0
      ? t('proj_next_skip')
      : t('btn_continue');

  return (
    <StepFrame
      title={t('proj_title')}
      sub={t('proj_sub')}
      back={onBack}
      next={{ label: nextLabel, onClick: handleContinue, disabled: importing }}
    >
      <div className="ob-card stack">
        <SectionLabel>{t('proj_active_label')}</SectionLabel>
        <div className="row wrap">
          <div className="grow min0">
            <div className="strong ellipsis" title={chat.cwd}>{chat.project}</div>
            <div className="tiny dim mono ellipsis" title={chat.cwd}>{chat.cwd}</div>
          </div>
          <button type="button" className="button primary" onClick={() => ui.set({ pickingFolder: true })}>
            <Icon name="folder" size={14} />{t('proj_browse')}
          </button>
          <button
            type="button"
            className="button"
            onClick={() => {
              setCreatingIn(creatingIn ? null : chat.cwd);
              setFolderError(null);
            }}
          >
            <Icon name="folderPlus" size={14} />{t('proj_new_subfolder')}
          </button>
        </div>

        <label className="stack tight">
          <span className="small muted">{t('proj_name_label')}</span>
          <input
            className="input"
            placeholder={t('proj_name_ph')}
            value={projectName}
            onChange={(e) => setProjectName(e.target.value)}
            onBlur={() => saveProjectName(projectName)}
          />
        </label>

        {creatingIn !== null && (
          <div className="ob-subcard stack">
            <div className="small strong">{t('proj_create_in', { path: creatingIn })}</div>
            <div className="row">
              <input
                className="input grow"
                autoFocus
                placeholder="new-project-name"
                value={newFolderName}
                onChange={(e) => setNewFolderName(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && createNewProjectFolder()}
              />
              <button type="button" className="button primary" disabled={!newFolderName.trim()} onClick={createNewProjectFolder}>{t('proj_create_btn')}</button>
              <button type="button" className="button ghost" onClick={() => setCreatingIn(null)}>{t('btn_cancel')}</button>
            </div>
            {folderError && <div className="small red">{folderError}</div>}
          </div>
        )}

        {places && places.places.length > 0 && (
          <div className="stack tight">
            <div className="tiny dim">{t('proj_quick_folders')}</div>
            <div className="row wrap">
              {places.places.map((pl) => (
                <button
                  key={pl.path}
                  type="button"
                  className="chip"
                  title={pl.path}
                  onClick={() => invoke('add_project', { dir: pl.path })}
                >
                  <Icon name={pl.icon} size={13} />{pl.name}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Summarized by project when Claude Code or Codex chats are detected on this PC */}
      {projects.length > 0 && (
        <div className="ob-card stack">
          <div className="row wrap">
            <div className="grow min0">
              <SectionLabel>
                {t('proj_detected_header', { chats: detected.length, s: detected.length === 1 ? '' : 's', projects: projects.length, p: projects.length === 1 ? '' : 's' })}
              </SectionLabel>
              <div className="small dim">
                {t('proj_detected_sub')}
              </div>
            </div>
            <div className="row">
              <button
                type="button"
                className="button ghost compact"
                onClick={() => setSelectedProjects(selectedProjects.size === projects.length ? new Set() : new Set(projects.map((p) => p.key)))}
              >
                {selectedProjects.size === projects.length ? t('proj_clear_all') : t('proj_select_all')}
              </button>
              {selectedProjects.size > 0 && (
                <button
                  type="button"
                  className="button primary compact"
                  disabled={importing}
                  onClick={importSelected}
                >
                  {importing ? <Spinner size={12} /> : null}
                  {t('proj_import_btn', { count: selectedProjects.size })}
                </button>
              )}
            </div>
          </div>

          <div className="ob-detected-list">
            {projects.map((p) => {
              const checked = selectedProjects.has(p.key);
              return (
                <div key={p.key} className={`ob-detected-row${checked ? ' selected' : ''}`}>
                  <label className="check grow min0">
                    <input type="checkbox" checked={checked} onChange={() => toggleProject(p.key)} />
                    <div className="ob-project-logos">
                      {p.sources.map((s) => <img key={s} src={logo(s)} width={16} height={16} alt={s} />)}
                    </div>
                    <div className="stack tight grow min0">
                      <div className="row">
                        <span className="strong small ellipsis">{p.project}</span>
                        <span className="ob-badge">{p.chatIds.length} chat{p.chatIds.length === 1 ? '' : 's'}</span>
                      </div>
                      <div className="tiny dim mono ellipsis" title={p.cwd}>
                        {p.cwd}
                      </div>
                    </div>
                  </label>
                  <button
                    type="button"
                    className="button ghost compact"
                    title={`Switch active project folder to ${p.cwd}`}
                    onClick={() => invoke('add_project', { dir: p.cwd }).catch(() => {})}
                  >
                    {t('proj_use_folder')}
                  </button>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {importedCount !== null && importedCount > 0 && (
        <Linked title={t('proj_imported_banner', { count: importedCount, s: importedCount === 1 ? '' : 's' })}>
          {t('proj_imported_ready')}
        </Linked>
      )}
    </StepFrame>
  );
}

// ---------- Step 6: Appearance ----------

function AppearanceStep({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const theme = useTheme();
  const { t } = useTranslation();
  const elements = ELEMENTS.find((e) => e.id === theme.elements)!.vars;
  return (
    <StepFrame
      title={t('app_title')}
      sub={t('app_sub')}
      wide
      back={onBack}
      next={{ label: t('btn_continue'), onClick: onNext }}
    >
      <div className="ob-appearance">
        <div className="ob-card stack">
          <SectionLabel>{t('app_bg')}</SectionLabel>
          <div className="swatches" role="radiogroup" aria-label={t('app_bg')}>
            {BACKGROUNDS.map((b) => (
              <button key={b.id} type="button" role="radio" aria-checked={b.id === theme.background} className={`swatch ${b.id === theme.background ? 'selected' : ''}`} onClick={() => setTheme({ background: b.id })}>
                <span className="swatch-preview">
                  <i style={{ width: '30%', background: b.vars.sidebar }} />
                  <i style={{ flex: 1, background: b.vars.bg, padding: 6 }}>
                    <i style={{ height: 7, width: '70%', borderRadius: 3, background: b.vars.surface }} />
                    <i style={{ height: 7, width: '40%', marginTop: 4, marginLeft: 'auto', borderRadius: 3, background: elements.primary }} />
                  </i>
                </span>
                {b.name}
              </button>
            ))}
          </div>

          <SectionLabel>{t('app_elements')}</SectionLabel>
          <div className="ob-dots" role="radiogroup" aria-label={t('app_elements')}>
            {ELEMENTS.map((e) => (
              <button key={e.id} type="button" role="radio" aria-checked={e.id === theme.elements} title={e.name} className={`ob-dot ${e.id === theme.elements ? 'selected' : ''}`} onClick={() => setTheme({ elements: e.id })}>
                <span className="swatch-dot" style={{ background: e.vars.primary }} />
                <span className="tiny">{e.name}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="ob-card stack">
          <SectionLabel>{t('app_material')}</SectionLabel>
          <div className="swatches materials" role="radiogroup" aria-label={t('app_material')}>
            {MATERIALS.map((m) => (
              <button key={m.id} type="button" role="radio" aria-checked={m.id === theme.material} title={m.note} className={`swatch ${m.id === theme.material ? 'selected' : ''}`} onClick={() => setTheme({ material: m.id })}>
                <span className={`swatch-preview material-preview ${m.id}`}>
                  <i style={{ width: '30%' }} />
                  <i style={{ flex: 1, padding: 6 }}>
                    <i style={{ height: 7, width: '70%', borderRadius: 3 }} />
                    <i style={{ height: 7, width: '40%', marginTop: 4, marginLeft: 'auto', borderRadius: 3, background: elements.primary }} />
                  </i>
                </span>
                {m.name}
              </button>
            ))}
          </div>
          <div className="tiny dim">{MATERIALS.find((m) => m.id === theme.material)!.note}</div>
          {getMaterialError() && <div className="small red">{getMaterialError()}</div>}

          <SectionLabel>{t('app_logo')}</SectionLabel>
          <div className="ob-logos" role="radiogroup" aria-label={t('app_logo')}>
            {LOGOS.map((l) => (
              <button key={l.id} type="button" role="radio" aria-checked={l.id === theme.logo} title={l.name} className={`ob-dot ${l.id === theme.logo ? 'selected' : ''}`} onClick={() => setTheme({ logo: l.id })}>
                {l.id === 'classic' ? <ScoutLogo size={28} id="classic" /> : <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(logoSvg(l))}`} width={28} height={28} alt="" />}
                <span className="tiny">{l.name}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </StepFrame>
  );
}

// ---------- Step 7: General Options (sectioned cards with icons) ----------

function OptionSection({ icon, title, desc, children }: { icon: string; title: string; desc?: string; children: ReactNode }) {
  return (
    <div className="ob-card ob-option-card stack">
      <div className="ob-option-head row">
        <span className="ob-option-icon"><Icon name={icon} size={15} /></span>
        <div className="stack tight grow min0">
          <b className="strong">{title}</b>
          {desc && <span className="tiny dim">{desc}</span>}
        </div>
      </div>
      <div className="ob-option-body stack">{children}</div>
    </div>
  );
}

function OptionRow({ icon, title, sub, children }: { icon?: string; title: string; sub: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      {icon && <span className="ob-row-icon"><Icon name={icon} size={15} /></span>}
      <div className="grow min0">
        <div className="setting-title">{title}</div>
        <div className="small dim">{sub}</div>
      </div>
      <div className="row setting-control">{children}</div>
    </div>
  );
}

function OptionsStep({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  const cfg = useView('cfg');
  const { cwd } = useView('chat');
  const sm = useView('scoutMote');
  const updater = useView('updater');
  const { t } = useTranslation();
  const [instructions, setInstructions] = useState<string | null>(null);
  const [savedInstructions, setSavedInstructions] = useState('');

  useEffect(() => {
    invoke<Prefs>('instructions_load', { cwd }).then((p) => { setInstructions(p.global); setSavedInstructions(p.global); }).catch(() => {});
  }, [cwd]);

  const saveInstructions = async () => {
    if (instructions === null || instructions === savedInstructions) return;
    await savePrefs(cwd, { global: instructions }).then(() => setSavedInstructions(instructions)).catch(() => {});
  };

  const flash = cfg.notifyFlash ?? true;
  const toast = cfg.notifyToast ?? false;
  const games = cfg.games;
  const setGames = (next: Partial<Games>) => {
    if (next.on === false || (next.place && next.place !== games.place)) closeGame();
    act('setGames', { games: { ...games, ...next } });
  };

  const finish = async () => {
    await saveInstructions();
    if (cfg.notifyFlash === null || cfg.notifyToast === null) {
      act('setNotify', { flash, toast });
    }
    if (cfg.loginNeeded) act('setGuest', { on: true });
    onDone();
  };

  return (
    <StepFrame
      title={t('opt_title')}
      sub={t('opt_sub')}
      wide
      back={onBack}
      next={{ label: t('opt_finish'), onClick: finish }}
    >
      <div className="ob-options-grid">
        <OptionSection icon="pencil" title={t('opt_instructions_title')} desc={t('opt_instructions_desc')}>
          <textarea
            className="input"
            rows={2}
            placeholder={t('opt_instructions_ph')}
            value={instructions ?? ''}
            disabled={instructions === null}
            onChange={(e) => setInstructions(e.target.value)}
            onBlur={saveInstructions}
          />
        </OptionSection>

        {cfg.hasKey && (
          <OptionSection icon="phone" title={t('opt_scoutmote_title')} desc={t('opt_scoutmote_desc')}>
            <OptionRow
              icon="globe"
              title={t('opt_scoutmote_enable')}
              sub={t('opt_scoutmote_enable_sub', { name: sm.name })}
            >
              <Toggle label={t('opt_scoutmote_title')} on={sm.on} onChange={(on) => act('setScoutMote', { on })} />
            </OptionRow>
          </OptionSection>
        )}

        <OptionSection icon="chat" title={t('opt_discord_title')} desc={t('opt_discord_desc')}>
          <OptionRow
            icon="user"
            title={t('opt_discord_show')}
            sub={t('opt_discord_show_sub')}
          >
            <Toggle label={t('opt_discord_show')} on={cfg.discordPresence} onChange={(on) => act('setDiscord', { presence: on, project: on && cfg.discordProject })} />
          </OptionRow>
          {cfg.discordPresence && (
            <OptionRow
              icon="folder"
              title={t('opt_discord_project')}
              sub={t('opt_discord_project_sub')}
            >
              <Toggle label={t('opt_discord_project')} on={cfg.discordProject} onChange={(on) => act('setDiscord', { presence: true, project: on })} />
            </OptionRow>
          )}
        </OptionSection>

        <OptionSection icon="game" title={t('opt_games_title')} desc={t('opt_games_desc')}>
          <OptionRow
            icon="game"
            title={t('opt_games_play')}
            sub={t('opt_games_play_sub')}
          >
            <Toggle label={t('opt_games_title')} on={games.on} onChange={(on) => setGames({ on })} />
          </OptionRow>
          {games.on && (
            <>
              <OptionRow
                icon="panel"
                title={t('opt_games_where')}
                sub={games.place === 'panel' ? t('opt_games_where_sub_panel') : t('opt_games_where_sub_float')}
              >
                <select className="input select" value={games.place} onChange={(e) => setGames({ place: e.target.value as Games['place'] })}>
                  <option value="float">{t('opt_games_float')}</option>
                  <option value="panel">{t('opt_games_panel')}</option>
                </select>
              </OptionRow>
              <OptionRow
                icon="bolt"
                title={t('opt_games_auto_open')}
                sub={t('opt_games_auto_open_sub')}
              >
                <Toggle label={t('opt_games_auto_open')} on={games.openOnRun} onChange={(openOnRun) => setGames({ openOnRun })} />
              </OptionRow>
              <OptionRow
                icon="check"
                title={t('opt_games_auto_close')}
                sub={t('opt_games_auto_close_sub')}
              >
                <Toggle label={t('opt_games_auto_close')} on={games.autoClose} onChange={(autoClose) => setGames({ autoClose })} />
              </OptionRow>
            </>
          )}
        </OptionSection>

        <OptionSection icon="bell" title={updater.enabled ? t('opt_notify_title_with_updates') : t('opt_notify_title')} desc={t('opt_notify_desc')}>
          <OptionRow
            icon="bell"
            title={t('opt_notify_flash')}
            sub={t('opt_notify_flash_sub')}
          >
            <Toggle label={t('opt_notify_flash')} on={flash} onChange={(on) => act('setNotify', { flash: on, toast })} />
          </OptionRow>
          <OptionRow
            icon="desktop"
            title={t('opt_notify_toast')}
            sub={t('opt_notify_toast_sub')}
          >
            <Toggle label={t('opt_notify_toast')} on={toast} onChange={(on) => act('setNotify', { flash, toast: on })} />
          </OptionRow>
          {updater.enabled && (
            <OptionRow
              icon="download"
              title={t('opt_update_auto')}
              sub={t('opt_update_auto_sub')}
            >
              <Toggle label={t('opt_update_auto')} on={cfg.autoUpdate} onChange={(on) => act('setAutoUpdate', { on })} />
            </OptionRow>
          )}
        </OptionSection>
      </div>
    </StepFrame>
  );
}

// ---------- Main Onboarding Container ----------

export function Onboarding({ onDone }: { onDone: () => void }) {
  // 0 is the welcome; 1..STEPS.length are the steps.
  const [step, setStep] = useState<number>(0);
  const { t } = useTranslation();

  // Preload folder places and detected Claude Code / Codex chats right at the start of onboarding
  // so they are ready by the time the user reaches the Project step.
  const [places, setPlaces] = useState<Places | null>(null);
  const [detected, setDetected] = useState<DetectedChat[]>([]);

  useEffect(() => {
    invoke<Places>('folder_places').then(setPlaces).catch(() => {});
    invoke<DetectedChat[]>('detect_external_chats').then(setDetected).catch(() => {});
  }, []);

  if (step === 0) return <WelcomeIntro onStart={() => setStep(1)} />;
  const current = STEPS[step - 1].id;
  const back = () => setStep(step - 1);
  const next = () => setStep(step + 1);

  return (
    <div className="ob-shell">
      <nav className="ob-steps" aria-label="Onboarding steps">
        {STEPS.map((s, i) => (
          <button
            key={s.id}
            type="button"
            className={`ob-step${i + 1 === step ? ' active' : ''}${i + 1 < step ? ' done' : ''}`}
            aria-current={i + 1 === step ? 'step' : undefined}
            onClick={() => setStep(i + 1)}
          >
            <span className="ob-step-num">{i + 1 < step ? <Icon name="check" size={10} /> : i + 1}</span>
            <span className="ob-step-label">{t(s.labelKey)}</span>
          </button>
        ))}
      </nav>

      <StepCtx.Provider value={{ index: step, total: STEPS.length }}>
        {current === 'language' && <LanguageStep onNext={next} />}
        {current === 'special' && <SpecialStep onBack={back} onNext={next} />}
        {current === 'scout' && <ScoutLoginStep onBack={back} onNext={next} />}
        {current === 'providers' && <ProvidersStep onBack={back} onNext={next} />}
        {current === 'project' && (
          <ProjectStep
            places={places}
            detected={detected}
            setDetected={setDetected}
            onBack={back}
            onNext={next}
          />
        )}
        {current === 'appearance' && <AppearanceStep onBack={back} onNext={next} />}
        {current === 'options' && <OptionsStep onBack={back} onDone={onDone} />}
      </StepCtx.Provider>
    </div>
  );
}
