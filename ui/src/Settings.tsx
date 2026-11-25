import { useEffect, useState, type ReactNode } from 'react';
import { accountLabel, act, invoke, money, restartBlocker, num, openExternal, parseUsd, setExplore, shortDate, usd, DISCORD_INVITE_URL, type AccountProvider, type Games, type McpServer, type ModelRef, type Music, type UsageInfo } from './api';
import { closeGame } from './Games';
import { closeMusic } from './Music';
import { AgentRouterPanel, ModelList } from './ModelMenu';
import { ProviderQuotas } from './Quota';
import { AdditionalKey, CUSTOM_NEW, CUSTOM_NOTE, CustomProviderDetails, CustomProviderForm, BrowserEntry, CLAUDE_NOTE, CODEX_NOTE, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GUEST_NOTE, GrokEntry, GuestEntry, GrokKeyEntry, HERMES_NOTE, HermesEntry, HermesKeyEntry, KIRO_NOTE, ANTIGRAVITY_NOTE, VIBE_NOTE, KeyField, NvidiaEntry, logo, OPENCODE_NOTE, OpencodeEntry, providerStatus, ScoutKeyEntry, SignInEntry } from './Accounts';
import { ui, useCore, useUi, useView, type SettingsTab } from './store';
import { BACKGROUNDS, ELEMENTS, MATERIALS, getMaterialError, setTheme, useTheme } from './theme';
import { LOGOS, ScoutLogo, logoSvg } from './logo';
import { ClaudeSetup } from './ClaudeSetup';
import { LocalSetup, localStatus } from './Local';
import { HIDEABLE, showAllUi, showUi, useHidden } from './Hideable';
import { isFull, setUiLanguage, useUiLanguage, useTranslation, SUPPORTED_LANGUAGES } from './i18n';
import { CodeBlock, CopyButton, Icon, Link, SectionLabel, Select, Spinner, Toggle } from './widgets';
import { DreamEntry, DreamingNow } from './Dreams';
import { ErrorBoundary } from './ErrorBoundary';

const TABS: { id: SettingsTab; title: string; icon: string; group: string; keywords: string }[] = [
  { id: 'appearance', title: 'Appearance', icon: 'palette', group: 'General', keywords: 'appearance theme themes colors colours background dark black midnight forest mocha plum buttons look style language interface translation logo material glass mica hidden games game 2048 chess flappy bird scout dash runner play fun waiting music mp3 soundcloud audio song player' },
  { id: 'preferences', title: 'Preferences', icon: 'sliders', group: 'General', keywords: 'preferences notifications alerts taskbar flash toast windows attention discord rich presence profile status share activity explore explorer sub-agent subagent search research free qwen save plan tokens limits cheaper scoutmote remote control browser web phone away online computer pc privacy analytics telemetry usage data tracking posthog statistics opt out close closing quit exit confirm ask' },
  { id: 'instructions', title: 'Instructions', icon: 'chat', group: 'General', keywords: 'instructions project global rules preferences language russian english responses' },
  { id: 'memory', title: 'Memory', icon: 'memory', group: 'General', keywords: 'memory remember project user profile facts context dreaming dream dreams idle away sleep tidy learn recent chats free model log undo' },
  { id: 'providers', title: 'Providers', icon: 'layers', group: 'Models', keywords: 'providers link unlink connect sign in sign out key claude code codex chatgpt anthropic openai copilot github openrouter nvidia opencode grok xai hermes nous devin kiro antigravity google gemini mistral vibe devstral kilo models local llama llama.cpp gguf gpu offload offline ollama tools small model' },
  { id: 'router', title: 'Custom agents', icon: 'route', group: 'Models', keywords: 'custom agents agent router routing route fallback fallbacks backup fails failed error retry jev rate limit 429 provider models intelligence tier' },
  { id: 'images', title: 'Image generation', icon: 'sparkle', group: 'Models', keywords: 'image images generation generate generator pictures art icons illustrations openrouter openai key byok gpt image gemini' },
  { id: 'browser', title: 'Browser', icon: 'globe', group: 'Tools', keywords: 'browser built-in web browse website page click forms localhost test agent computer use webview automation' },
  { id: 'skills', title: 'Skills', icon: 'file', group: 'Tools', keywords: 'skills upload directory skills.sh install instructions @ mention' },
  { id: 'mcp', title: 'MCP servers', icon: 'terminal', group: 'Tools', keywords: 'mcp servers custom tools connect add command url http stdio integrations' },
  { id: 'chatbots', title: 'Chatbots', icon: 'chat', group: 'Tools', keywords: 'chatbots mcp connector link claude chatgpt cursor tools server' },
  { id: 'account', title: 'Account', icon: 'user', group: 'Scout', keywords: 'account scoutcode sign in sign out login key username billing rewards daily streak claim referral referrals invite code bonus credit' },
  { id: 'updates', title: 'Updates', icon: 'download', group: 'Scout', keywords: 'updates version upgrade download restart release notes about' },
  { id: 'logs', title: 'Logs', icon: 'files', group: 'Scout', keywords: 'logs log crash crashed crashes error errors bug report debug diagnostics support copy save session' },
];

function Heading({ title, sub }: { title: string; sub: string }) {
  return <div className="settings-heading"><h2>{title}</h2><p className="small dim">{sub}</p></div>;
}

/** The head of one part of a page that holds several, such as Preferences. */
function Part({ title, sub, icon }: { title: string; sub: string; icon?: string }) {
  return <div className="settings-part"><h3>{icon && <Icon name={icon} size={17} />}{title}</h3><p className="small dim">{sub}</p></div>;
}

function SettingRow({ title, sub, children }: { title: string; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="grow min0"><div className="setting-title">{title}</div>{sub && <div className="small dim pre-wrap">{sub}</div>}</div>
      {children && <div className="row setting-control">{children}</div>}
    </div>
  );
}

function Message({ m }: { m: [boolean, string] | null }) {
  return m ? <div className={`small ${m[0] ? 'muted' : 'red'}`}>{m[1]}</div> : null;
}

// ---------- instructions and memory ----------

const SAVED = 'Saved. Applies from your next message, including in existing chats.';

function Instructions() {
  const { cwd } = useView('chat');
  const languages = useCore((s) => s.statics.languages);
  const [saved, setSaved] = useState({ global: '', project: '', language: 'auto' });
  const [draft, setDraft] = useState(saved);
  const [message, setMessage] = useState<[boolean, string] | null>(null);
  const [broken, setBroken] = useState(false);
  const load = () => invoke<typeof saved>('instructions_load', { cwd }).then((p) => { setSaved(p); setDraft(p); setBroken(false); setMessage(null); }, (e) => { setBroken(true); setMessage([false, String(e)]); });
  useEffect(() => { load(); }, [cwd]); // eslint-disable-line
  const dirty = !broken && JSON.stringify(saved) !== JSON.stringify(draft);
  return (
    <>
      <Heading title="Instructions" sub="Save preferences across chats. Scout includes these rules in its context again after a long conversation is summarized." />
      <SettingRow title="AI replies in" sub="Auto follows the conversation. Choose a language to keep replies and progress updates in that language, separately from the interface language.">
        <Select className="input select" value={draft.language} onChange={(e) => setDraft({ ...draft, language: e.target.value })}>
          {languages.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </Select>
      </SettingRow>
      <SectionLabel>All projects</SectionLabel>
      <p className="small dim">General preferences for every desktop chat on this computer.</p>
      <textarea className="input" rows={5} placeholder="For example: Keep explanations concise and explain changes before running commands." value={draft.global} onChange={(e) => setDraft({ ...draft, global: e.target.value })} />
      <SectionLabel>This project</SectionLabel>
      <p className="tiny dim">{cwd}</p>
      <p className="small dim">Applies to every chat in this folder. Specific project rules override general preferences. Existing SCOUT.md and AGENTS.md files also apply.</p>
      <textarea className="input" rows={6} placeholder="For example: Use pnpm, keep the existing component style, and run pnpm test after changes." value={draft.project} onChange={(e) => setDraft({ ...draft, project: e.target.value })} />
      <div className="row save-row">
        <button type="button" className="button primary" disabled={!dirty} onClick={() => invoke('instructions_save', { cwd, ...draft }).then(() => { setSaved(draft); setMessage([true, SAVED]); }, (e) => setMessage([false, String(e)]))}>Save instructions</button>
        <button type="button" className="button" onClick={load}>Reload saved</button>
        {dirty && <span className="small dim">Unsaved changes</span>}
      </div>
      <Message m={message} />
    </>
  );
}

function Memory() {
  const { cwd } = useView('chat');
  const [paths, setPaths] = useState(['', '']);
  const [saved, setSaved] = useState(['', '']);
  const [drafts, setDrafts] = useState(['', '']);
  const [message, setMessage] = useState<[boolean, string] | null>(null);
  const load = () => invoke<{ path: string; text: string }[]>('memory_load', { cwd }).then((m) => {
    setPaths(m.map((x) => x.path));
    setSaved(m.map((x) => x.text));
    setDrafts(m.map((x) => x.text));
    setMessage(null);
  });
  useEffect(() => { load(); }, [cwd]); // eslint-disable-line
  const about = [
    ['This project', 'Facts about this repo. You can commit this file to share it with your team, so keep it free of anything personal.', 'For example: ## Commands\n- cargo test -p core'],
    ['You', 'Facts about you, for every project on this computer. Never committed.', 'For example: ## Style\n- Short answers, code first'],
  ];
  const dirty = saved[0] !== drafts[0] || saved[1] !== drafts[1];
  return (
    <>
      <Heading title="Memory" sub="What Scout has learned, included in every chat. Scout keeps these up to date itself, asking first as your permission mode says. You can edit them here too." />
      {about.map(([title, note, hint], i) => (
        <div key={title} className="stack">
          <SectionLabel>{title}</SectionLabel>
          <p className="tiny dim">{paths[i]}</p>
          <p className="small dim">{note}</p>
          <textarea className="input mono" rows={10} placeholder={hint} value={drafts[i]} onChange={(e) => setDrafts(drafts.map((d, k) => (k === i ? e.target.value : d)))} />
        </div>
      ))}
      <div className="row save-row">
        <button type="button" className="button primary" disabled={!dirty} onClick={() => invoke('memory_save', { cwd, saved, drafts }).then(() => { setSaved(drafts); setMessage([true, SAVED]); }, (e) => setMessage([false, String(e)]))}>Save memory</button>
        <button type="button" className="button" onClick={load}>Reload saved</button>
        {dirty && <span className="small dim">Unsaved changes</span>}
      </div>
      <Message m={message} />
      <Dreaming />
    </>
  );
}

function Dreaming() {
  const cfg = useView('cfg');
  const d = useView('dreaming');
  const on = d.on === true;
  const next = d.last ? new Date(d.last + 24 * 60 * 60_000) : null;
  return (
    <>
      <Part title="Dreaming" sub="While you're away, Scout reads your last two days of chats and tidies your memory: it keeps what will still be true in a month, like your stack, style, commands and decisions, and never what you're in the middle of." />
      <SettingRow title="Dream while I'm away" sub="Starts after 20 minutes without using Scout, at most once a day, while no chat is working. Uses ScoutCode's free Dreaming model, and each request counts toward your free requests.">
        <Toggle label="Dream while I'm away" on={on} onChange={(v) => act('setDreaming', { on: v })} />
      </SettingRow>
      {!cfg.hasKey && !cfg.guest && <div className="small red">Dreaming runs on ScoutCode: sign in, or use its free models without an account, in Settings &gt; Account.</div>}
      {!d.model && <div className="small dim">ScoutCode hasn't set a Dreaming model yet, so Scout won't dream for now.</div>}
      <SettingRow title="Dream now" sub={d.last ? `Last dream ${new Date(d.last).toLocaleString()}.${on && next && next.getTime() > Date.now() ? ` Next one after ${next.toLocaleString()}, once you're away.` : ''}` : 'No dreams yet.'}>
        <button type="button" className="button" disabled={d.running || !d.model} onClick={() => act('dreamNow')}>{d.running ? 'Dreaming…' : 'Dream now'}</button>
      </SettingRow>
      <DreamingNow />
      {d.message && <Message m={[d.message.ok, d.message.text]} />}
      <SectionLabel>Dream Log</SectionLabel>
      {d.log.length === 0 && <p className="small dim">What each dream changed shows up here, with an Undo.</p>}
      <div className="stack">{d.log.map((x) => <DreamEntry key={x.id} d={x} />)}</div>
    </>
  );
}

function Notifications() {
  const cfg = useView('cfg');
  const flash = cfg.notifyFlash ?? true;
  const toast = cfg.notifyToast ?? false;
  return (
    <>
      <Part title="Notifications" sub="When Scout finishes, or needs an answer, while its window is in the background." />
      <SettingRow title="Flash the taskbar button" sub="Scout's button blinks until you switch back to it."><Toggle label="Flash the taskbar button" on={flash} onChange={(on) => act('setNotify', { flash: on, toast })} /></SettingRow>
      <SettingRow title="Windows notification" sub="A notification in the corner of the screen says what Scout needs."><Toggle label="Windows notification" on={toast} onChange={(on) => act('setNotify', { flash, toast: on })} /></SettingRow>
      <SettingRow title="Try it" sub="Shows a notification now, whatever is chosen above."><button type="button" className="button" onClick={() => act('testNotification')}>Send a test</button></SettingRow>
    </>
  );
}

function Discord() {
  const cfg = useView('cfg');
  const presence = cfg.discordPresence;
  const project = cfg.discordProject;
  return (
    <>
      <Part icon="discord" title="Discord" sub="Shows on your Discord profile that you're using Scout." />
      <SettingRow
        title="Scout Discord server"
        sub={
          <>
            Join our community at{' '}
            <a
              className="link"
              href={DISCORD_INVITE_URL}
              onClick={(e) => {
                e.preventDefault();
                openExternal(DISCORD_INVITE_URL);
              }}
            >
              discord.gg/qJ8kjUntMV
            </a>{' '}
            to chat with the team, get support, and link your account for a $0.25 balance reward.
          </>
        }
      >
        <button
          type="button"
          className="button"
          onClick={() => openExternal(DISCORD_INVITE_URL)}
        >
          Join Discord
        </button>
      </SettingRow>
      <SettingRow title="Show Scout on Discord" sub="Your profile shows \“Using Scout\” while the app is open. Nothing else is sent."><Toggle label="Show Scout on Discord" on={presence} onChange={(on) => act('setDiscord', { presence: on, project: on && project })} /></SettingRow>
      <SettingRow title="Show the folder's name" sub="Adds \“Working on <folder>\”, using only the folder's name, never its path, chats or files. Needs the setting above."><Toggle label="Show the folder's name" on={project} onChange={(on) => act('setDiscord', { presence, project: on })} /></SettingRow>
    </>
  );
}

/** Colors, picked from short lists of ones that go together. */
function Appearance() {
  const theme = useTheme();
  const elements = ELEMENTS.find((e) => e.id === theme.elements)!.vars;
  const coreLanguages = useCore((s) => s.statics?.languages);
  const uiLanguage = useUiLanguage();
  const { t } = useTranslation();
  const languages = (coreLanguages && coreLanguages.length > 0 ? (() => {
    const map = new Map<string, string>(coreLanguages);
    for (const [id, label] of SUPPORTED_LANGUAGES) {
      if (!map.has(id)) map.set(id, label);
    }
    return [...map.entries()];
  })() : SUPPORTED_LANGUAGES).filter(([id]) => id !== 'auto');
  return (
    <>
      <Heading title={t('settings_appearance')} sub={t('settings_appearance_sub')} />
      <SettingRow title={t('settings_interface_lang')} sub={isFull(uiLanguage) ? t('settings_interface_lang_full') : t('settings_interface_lang_partial')}>
        <Select
          className="input select"
          value={uiLanguage}
          // Only the interface: the language Scout replies in is its own setting, under Instructions.
          onChange={(e) => setUiLanguage(e.target.value)}
        >
          <optgroup label={t('settings_optgroup_full')}>
            {languages.filter(([id]) => isFull(id)).map(([id, label]) => <option key={id} value={id}>{label}</option>)}
          </optgroup>
          {languages.some(([id]) => !isFull(id)) && (
            <optgroup label={t('settings_optgroup_partial')}>
              {languages.filter(([id]) => !isFull(id)).map(([id, label]) => <option key={id} value={id}>{label}</option>)}
            </optgroup>
          )}
        </Select>
      </SettingRow>
      <SectionLabel>Background</SectionLabel>
      <div className="swatches" role="radiogroup" aria-label="Background">
        {BACKGROUNDS.map((b) => (
          <button key={b.id} type="button" role="radio" aria-checked={b.id === theme.background} className={`swatch ${b.id === theme.background ? 'selected' : ''}`} onClick={() => setTheme({ background: b.id })}>
            <span className="swatch-preview">
              <i style={{ width: '30%', background: b.vars.sidebar }} />
              <i style={{ flex: 1, background: b.vars.bg, padding: 7 }}>
                <i style={{ height: 8, width: '70%', borderRadius: 3, background: b.vars.surface }} />
                <i style={{ height: 8, width: '40%', marginTop: 5, marginLeft: 'auto', borderRadius: 3, background: elements.primary }} />
              </i>
            </span>
            {b.name}
          </button>
        ))}
      </div>
      <SectionLabel>Buttons and highlights</SectionLabel>
      <div className="swatches" role="radiogroup" aria-label="Buttons and highlights">
        {ELEMENTS.map((e) => (
          <button key={e.id} type="button" role="radio" aria-checked={e.id === theme.elements} className={`swatch ${e.id === theme.elements ? 'selected' : ''}`} onClick={() => setTheme({ elements: e.id })}>
            <span className="swatch-dot" style={{ background: e.vars.primary }} />
            {e.name}
          </button>
        ))}
      </div>
      <SectionLabel>Window material</SectionLabel>
      <div className="swatches" role="radiogroup" aria-label="Window material">
        {MATERIALS.map((m) => (
          <button key={m.id} type="button" role="radio" aria-checked={m.id === theme.material} title={m.note} className={`swatch ${m.id === theme.material ? 'selected' : ''}`} onClick={() => setTheme({ material: m.id })}>
            <span className={`swatch-preview material-preview ${m.id}`}>
              <i style={{ width: '30%' }} />
              <i style={{ flex: 1, padding: 7 }}>
                <i style={{ height: 8, width: '70%', borderRadius: 3 }} />
                <i style={{ height: 8, width: '40%', marginTop: 5, marginLeft: 'auto', borderRadius: 3, background: elements.primary }} />
              </i>
            </span>
            {m.name}
          </button>
        ))}
      </div>
      <p className="small dim">{MATERIALS.find((m) => m.id === theme.material)!.note}</p>
      {getMaterialError() && <p className="small red">{getMaterialError()}</p>}
      <SectionLabel>Logo</SectionLabel>
      <div className="swatches logos" role="radiogroup" aria-label="Logo">
        {LOGOS.map((l) => (
          <button key={l.id} type="button" role="radio" aria-checked={l.id === theme.logo} className={`swatch logo-swatch ${l.id === theme.logo ? 'selected' : ''}`} onClick={() => setTheme({ logo: l.id })}>
            {l.id === 'classic' ? <ScoutLogo size={40} id="classic" /> : <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(logoSvg(l))}`} width={40} height={40} alt="" />}
            {l.name}
          </button>
        ))}
      </div>
      <p className="small dim">Shows in the sidebar, on the start screen and on the taskbar.</p>
      <Extras />
      <HiddenItems />
      <SectionLabel>Setup</SectionLabel>
      <SettingRow title="Run setup again" sub="Walk through the first-run screens again. Your providers and settings stay as they are.">
        <button type="button" className="button" onClick={() => ui.set({ settings: null, onboarding: true, onboardingMode: 'full' })}>Start</button>
      </SettingRow>
      <div className="row"><button type="button" className="button ghost compact" onClick={() => setTheme({ background: 'graphite', elements: 'gray', material: 'solid', logo: 'classic' })}>Reset to default</button></div>
    </>
  );
}

/** What was hidden with a right-click on the top bar or the sidebar, to bring back. */
function HiddenItems() {
  const hidden = useHidden();
  return (
    <>
      <SectionLabel>Hidden items</SectionLabel>
      {hidden.length === 0 ? <p className="small dim">Right-click a button in the top bar, or New chat or Search chats in the sidebar, to hide it.</p> : (
        <>
          {HIDEABLE.filter(([id]) => hidden.includes(id)).map(([id, name, where]) => (
            <SettingRow key={id} title={name} sub={where}><button type="button" className="button" onClick={() => showUi(id)}>Show</button></SettingRow>
          ))}
          {hidden.length > 1 && <div className="row"><button type="button" className="button ghost compact" onClick={showAllUi}>Show all</button></div>}
        </>
      )}
    </>
  );
}

// ---------- explore ----------

/** Providers explore can call itself. Claude Code, Codex and the other apps run their own agent, so they can't. */
const EXPLORE_RUNS_ON = new Set(['scoutcode', 'openrouter', 'nvidia', 'kilo', 'local']);

function ExplorePage() {
  const { explore } = useView('cfg');
  const { additionalProviders } = useCore((s) => s.statics);
  const [skipped, setSkipped] = useState('');
  const runsHere = (p: string) => EXPLORE_RUNS_ON.has(p) || additionalProviders.some((a) => a.id === p);
  const models: ModelRef[] = explore.models.map(({ key, provider }) => ({ key, provider }));
  const change = (next: ModelRef[]) => {
    const ok = next.filter((m) => runsHere(m.provider));
    setSkipped(ok.length < next.length ? 'That model runs inside another app’s own agent, so explore can’t use it. Pick a ScoutCode, OpenRouter, NVIDIA, Kilo, local or additional provider’s model.' : '');
    setExplore(explore.on, ok);
  };
  return (
    <>
      <Part title="Explore" sub="Makes paid plans last longer. When a task needs a lot of searching and reading, a read-only explore agent does it on a free model, then hands back its findings and the files that matter, each checked to exist. Your paid model only reads the files it needs." />
      <SettingRow title="Explore agents" sub="Scout’s own sub-agents run on explore’s models, and Claude Code and Codex get an explore tool they’re told to use for any task that needs 5 or more files. Explore only reads: it can’t change files or run commands.">
        <Toggle label="Explore agents" on={explore.on} onChange={(on) => setExplore(on, models)} />
      </SettingRow>
      <SettingRow title="Models" sub={models.length ? 'Tried in this order: when one fails or hits a limit, the next carries the work on.' : 'Qwen 3.8 27B first, then ScoutCode’s other free models when it’s busy. Add your own to use them instead.'}>
        {models.length > 0 && <button type="button" className="button ghost compact" onClick={() => change([])}>Use the default</button>}
      </SettingRow>
      <ModelList value={models} onChange={change} max={3} />
      <Message m={skipped ? [false, skipped] : null} />
    </>
  );
}


type ImageModel = { id: string; name: string; cost: string };
const IMAGE_PROVIDERS: [string, string][] = [['openrouter', 'OpenRouter'], ['openai', 'OpenAI']];

/** Image generation, on the user's own key with a provider that makes images. */
function Images() {
  const cfg = useView('cfg');
  const on = cfg.experimentalImages;
  const { provider, model, key } = cfg.image;
  const [models, setModels] = useState<ImageModel[] | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    setModels(null);
    invoke<ImageModel[]>('image_models', { provider }).then((m) => { setModels(m); setError(''); }, (e) => setError(String(e)));
  }, [provider]);
  const set = (next: { on?: boolean; provider?: string; model?: string | null }) => act('setImages', { on, provider, model, ...next });
  const picked = model ?? '';
  const openrouterKey = cfg.openrouterKeys[0];
  const hasKey = provider === 'openai' ? !!key : !!openrouterKey;
  return (
    <>
      <Heading title="Image generation" sub="Scout can make images, such as icons, illustrations and placeholder art, and save them in your project. They're made on your own key with the provider you pick here, which bills you for each one. Scout asks before making one unless you've allowed edits." />
      <SettingRow title="Image generation" sub={on && !hasKey ? 'Add a key below before Scout can make images.' : 'Gives Scout a tool for making images. Applies from your next message.'}>
        <Toggle label="Image generation" on={on} onChange={(v) => set({ on: v })} />
      </SettingRow>
      <SettingRow title="Provider" sub="Whose key and image models to use.">
        <Select className="input select" value={provider} onChange={(e) => set({ provider: e.target.value, model: null })}>
          {IMAGE_PROVIDERS.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
        </Select>
      </SettingRow>
      {provider === 'openai' ? (
        key ? (
          <SettingRow title="OpenAI key" sub={<span className="mono">{key}</span>}>
            <button type="button" className="button" onClick={() => act('saveImageKey', { key: null })}>Remove</button>
          </SettingRow>
        ) : (
          <div className="stack">
            <div className="setting-title">OpenAI key</div>
            <div className="small dim">Saved on this PC and only used for images. <Link href="https://platform.openai.com/api-keys">Make a key on platform.openai.com</Link></div>
            <KeyField hint="sk-…" button="Save OpenAI key" onSave={(k) => act('saveImageKey', { key: k })} />
          </div>
        )
      ) : openrouterKey ? (
        <SettingRow title="OpenRouter key" sub={<>Uses your saved OpenRouter key <span className="mono">{openrouterKey.masked}</span>. Change it in Providers.</>}>
          <button type="button" className="button" onClick={() => ui.set({ settings: 'providers' })}>Providers</button>
        </SettingRow>
      ) : (
        <div className="stack">
          <div className="setting-title">OpenRouter key</div>
          <div className="small dim">Saved on this PC. It also adds OpenRouter's models to the model menu. <Link href="https://openrouter.ai/settings/keys">Make a key on openrouter.ai</Link></div>
          <KeyField hint="sk-or-…" button="Save OpenRouter key" onSave={(k) => act('addOpenrouterKey', { name: 'OpenRouter', key: k })} />
        </div>
      )}
      <SettingRow title="Image model" sub={error ? `Could not load the image models: ${error}` : 'Which of the provider’s image models makes them.'}>
        {models === null && !error ? <Spinner /> : (
          <Select className="input select" value={picked} onChange={(e) => set({ model: e.target.value || null })}>
            <option value="">Default ({provider === 'openai' ? 'GPT Image 1' : 'Gemini 2.5 Flash Image'})</option>
            {models?.map((m) => <option key={m.id} value={m.id}>{m.name}{m.cost ? ` · ${m.cost}` : ''}</option>)}
            {picked && models && !models.some((m) => m.id === picked) && <option value={picked}>{picked}</option>}
          </Select>
        )}
      </SettingRow>
    </>
  );
}

/** The built-in browser in the side panel, and whether Scout's agent may use it. */
function BrowserSettings() {
  const { browserAgent } = useView('cfg');
  return (
    <>
      <Heading title="Browser" sub="Scout has a web browser of its own, in the side panel (the globe in the top bar). You can browse in it yourself, and sign-ins stay between restarts." />
      <SettingRow title="Let Scout use the browser" sub="Scout can then open sites, click, fill in forms and read pages, such as to test the web app you're building. You watch it in the panel and can take over at any time. Before it submits, buys, sends or deletes something, it asks you, unless the chat has full access. Applies from your next message.">
        <Toggle label="Let Scout use the browser" on={browserAgent} onChange={(on) => act('setBrowserAgent', { on })} />
      </SettingRow>
      <SettingRow title="Open the browser" sub="Shows it in the side panel.">
        <button type="button" className="button" onClick={() => ui.set({ settings: null, page: 'chat', dockOpen: true, dockTab: 'browser' })}>Open</button>
      </SettingRow>
    </>
  );
}

/** Games and music: extras for while Scout works, set with how the window looks. */
function Extras() {
  const cfg = useView('cfg');
  const games = cfg.games;
  const setGames = (next: Partial<Games>) => {
    // A game that's open where it no longer belongs closes, rather than jumping elsewhere.
    if (next.on === false || (next.place && next.place !== games.place)) closeGame();
    act('setGames', { games: { ...games, ...next } });
  };
  const music = cfg.music;
  const setMusic = (next: Partial<Music>) => {
    if (next.on === false) closeMusic(true);
    act('setMusic', { music: { ...music, ...next } });
  };
  return (
    <>
      <SectionLabel>Games and music</SectionLabel>
      <SettingRow title="Games" sub="Something to play while Scout works: 2048, chess against Scout, Flappy Scout, where you flap a little bird through gaps between editor panes, or Scout Dash, an endless three-lane run. Open them with the gamepad at the top of the window and pick one. A game stays open until you close it, and keeps your progress when you do.">
        <Toggle label="Games" on={games.on} onChange={(on) => setGames({ on })} />
      </SettingRow>
      {games.on && (
        <SettingRow title="Where it shows" sub={games.place === 'panel' ? 'A Games tab in the panel on the right.' : 'A card over the chat. Drag it by its bar to wherever you like, and it stays there.'}>
          {games.place === 'float' && games.pos && <button type="button" className="button ghost compact" onClick={() => setGames({ pos: null })}>Reset position</button>}
          <Select className="input select" value={games.place} onChange={(e) => setGames({ place: e.target.value as Games['place'] })}>
            <option value="float">Floating card</option>
            <option value="panel">Side panel</option>
          </Select>
        </SettingRow>
      )}
      <SettingRow title="Music" sub="Listen to your own MP3s or SoundCloud while Scout works. Open it with the music button at the top of the window. Music keeps playing when you close the card.">
        <Toggle label="Music" on={music.on} onChange={(on) => setMusic({ on })} />
      </SettingRow>
    </>
  );
}

function AgentRouterPage() {
  return (
    <>
      <Heading
        title="Custom agents"
        sub="Make as many custom agents as you like, each its own route of providers and models (for example Opus 4.6 → Haiku 4.5 → DeepSeek), and pick one like a model. When a model fails or hits a rate limit, the next one carries the turn on. With Jev on, it rates each prompt's difficulty and picks the matching intelligence tier."
      />
      <AgentRouterPanel />
    </>
  );
}

// ---------- usage ----------

function Tile({ label, value, sub }: { label: string; value: string; sub: string }) {
  return <div className="tile"><div className="tiny dim">{label}</div><div className="tile-value">{value}</div><div className="tiny dim">{sub}</div></div>;
}

const shortTokens = (v: number) => {
  const n = num(v);
  return n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n);
};
/** "Nov 5" for an ISO time, or "soon" when the server didn't send one. */
const isoDay = (iso: unknown) => (typeof iso === 'string' && iso.length >= 10 ? shortDate(iso.slice(0, 10)) : 'soon');
/** "14:30", local time, for when a 5-hour window resets. */
const clock = (iso: string) => new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const validDays = (s: string) => { const d = Number(s.trim()); return Number.isInteger(d) && d >= 1 && d <= 3650 ? d : null; };

/** Admin controls: anyone's limits, giving money, and resetting everyone. */
function UsageAdmin({ info, busy }: { info: UsageInfo; busy: boolean }) {
  const [user, setUser] = useState(info.username);
  const [search, setSearch] = useState('');
  const [mode, setMode] = useState('default');
  const [monthly, setMonthly] = useState('');
  const [cli, setCli] = useState('');
  const [everyone, setEveryone] = useState(false);
  const [banked, setBanked] = useState(false);
  const [amount, setAmount] = useState('');
  const [expiry, setExpiry] = useState('30');
  const [resetDays, setResetDays] = useState('30');
  const [confirm, setConfirm] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  // Puts the chosen user's saved limits in the form.
  useEffect(() => {
    const u = info.users.find((m) => m.username === user);
    if (!u) return;
    setMode(u.mode);
    setMonthly(u.limits?.monthly != null ? money(u.limits.monthly) : '');
    setCli(u.cliLimit != null ? money(u.cliLimit) : '');
  }, [user, info]);
  const change = (body: unknown, said: string) => {
    setConfirm(null);
    setMessage(null);
    act('usageChange', { body, said });
  };
  const q = search.trim().toLowerCase();
  const found = q ? info.users.filter((m) => m.username.includes(q)) : [];
  const planCli = info.users.find((m) => m.username === user)?.cliMonthly ?? 0;
  const saveLimits = () => {
    const m = parseUsd(monthly);
    const c = cli.trim() ? parseUsd(cli) : null;
    if (mode === 'custom' && m === null) return setMessage('Money per month is an amount in USD, like 5 (0 = none).');
    if (cli.trim() && c === null) return setMessage('Desktop credit is an amount in USD, like 5. Leave it empty for what the plan gets.');
    change({ action: 'set-limits', username: user, limits: mode === 'custom' ? { monthly: m } : mode, cliLimit: c }, `Saved the limits for ${user}.`);
  };
  const give = () => {
    const a = parseUsd(amount);
    const d = validDays(expiry);
    if (a === null || a <= 0) return setMessage('The amount is in USD and more than $0, like 5.');
    if (banked && d === null) return setMessage('The expiry is a whole number of days, from 1 to 3650.');
    if (everyone && confirm !== 'give') return setConfirm('give');
    const body: Record<string, unknown> = { action: banked ? 'give-banked' : 'give-balance', amount: a, expiresInDays: d };
    if (everyone) body.everyone = true; else body.username = user;
    change(body, `Gave ${everyone ? 'everyone' : user} ${usd(a)} of ${banked ? 'banked balance' : 'balance'}.`);
    setAmount('');
  };
  return (
    <fieldset className="admin" disabled={busy}>
      <SectionLabel>Admin</SectionLabel>
      <b className="strong">Changing {user}</b>
      <input className="input field-260" placeholder="Find a user" value={search} onChange={(e) => setSearch(e.target.value)} />
      {found.slice(0, 6).map((m) => (
        <button key={m.username} type="button" className={`user-pick ${m.username === user ? 'selected' : ''}`} onClick={() => { setUser(m.username); setSearch(''); }}>
          {m.username}   {usd(m.spentThisMonth)} spent, {usd(m.harnessSpentThisMonth)} desktop this week, {usd(m.balance)} balance, {usd(m.banked)} banked
        </button>
      ))}
      {q && found.length === 0 && <div className="small dim">No matching users.</div>}
      {found.length > 6 && <div className="small dim">{found.length - 6} more. Keep typing to narrow it down.</div>}

      <div className="small muted">Limits per month</div>
      <div className="row">
        {[['default', 'Default'], ['custom', 'Custom'], ['unlimited', 'Unlimited']].map(([id, label]) => <label key={id} className="check"><input type="radio" checked={mode === id} onChange={() => setMode(id)} />{label}</label>)}
      </div>
      <div className="row wrap">
        {mode === 'custom' && <input className="input field-180" placeholder="Money per month, USD" value={monthly} onChange={(e) => setMonthly(e.target.value)} />}
        <input className="input field-180" placeholder={`Desktop credit / week (now ${usd(planCli)})`} value={cli} onChange={(e) => setCli(e.target.value)} />
        <button type="button" className="button primary" onClick={saveLimits}>Save limits</button>
      </div>

      <div className="small muted">Give money</div>
      <div className="row wrap">
        <label className="check"><input type="radio" checked={!banked} onChange={() => setBanked(false)} />Balance</label>
        <label className="check"><input type="radio" checked={banked} onChange={() => setBanked(true)} />Banked balance</label>
        <span className="gap" />
        <label className="check"><input type="radio" checked={!everyone} onChange={() => { setEveryone(false); setConfirm(null); }} />To {user}</label>
        <label className="check"><input type="radio" checked={everyone} onChange={() => { setEveryone(true); setConfirm(null); }} />To everyone</label>
      </div>
      <div className="row wrap">
        <input className="input field-140" placeholder="Amount, USD" value={amount} onChange={(e) => setAmount(e.target.value)} />
        {banked && <input className="input field-140" placeholder="Expires in days" value={expiry} onChange={(e) => setExpiry(e.target.value)} />}
        <button type="button" className="button primary" onClick={give}>{confirm === 'give' ? 'Confirm: give to everyone' : 'Give'}</button>
      </div>

      <SettingRow title="Reset usage" sub="Everyone's spending this month and free requests today go back to zero. Banked money and balances stay.">
        <button type="button" className="button" onClick={() => (confirm === 'reset' ? change({ action: 'reset-all' }, "Everyone's usage is back to zero.") : setConfirm('reset'))}>{confirm === 'reset' ? 'Confirm: reset everyone' : 'Reset usage'}</button>
      </SettingRow>
      <SettingRow title="Banked Reset" sub="Everyone gets one reset of their own to use when they want, before it expires.">
        <button type="button" className="button" onClick={() => {
          const d = validDays(resetDays);
          if (d === null) return setMessage('The expiry is a whole number of days, from 1 to 3650.');
          if (confirm === 'banked') change({ action: 'give-banked-reset', everyone: true, expiresInDays: d }, `Everyone got a banked reset that expires in ${d} days.`);
          else setConfirm('banked');
        }}>{confirm === 'banked' ? 'Confirm: give to everyone' : 'Banked Reset'}</button>
        <input className="input field-70" placeholder="Days" value={resetDays} onChange={(e) => setResetDays(e.target.value)} />
        <span className="small dim">Expires in days</span>
      </SettingRow>
      {message && <div className="small red">{message}</div>}
    </fieldset>
  );
}

function RewardsSection({ info, busy }: { info: UsageInfo; busy: boolean }) {
  const { baseUrl } = useView('cfg');
  const [code, setCode] = useState('');
  const dr = info.dailyReward;
  const ref = info.referrals;
  const referred = Array.isArray(ref?.referred) ? ref.referred : [];
  const dl = info.discordLink;
  if (!dr && !ref && !dl) return null;
  const site = baseUrl.replace(/\/v1\/?$/, '');
  const redeem = () => {
    const trimmed = code.trim();
    if (!trimmed || busy) return;
    act('usageChange', {
      body: { action: 'redeem-referral', code: trimmed },
      said: 'Redeemed referral code.',
    });
    setCode('');
  };
  return (
    <>
      {dr && (
        <SettingRow
          title={dr.streak > 0 ? `Daily login reward · ${dr.streak}-day streak` : 'Daily login reward'}
          sub={
            dr.canClaim
              ? `Day ${dr.nextDay}: claim ${usd(dr.amount)} to your balance. Days 1–3 earn $0.10/day, days 4–7 earn $0.20/day, and day 8+ earns $0.30/day. Missing a UTC day resets the streak.`
              : `Claimed ${usd(dr.claimedToday ?? dr.amount)} today. Day ${dr.nextDay} (${usd(dr.amount)}) unlocks at 00:00 UTC. Days 1–3: $0.10/day · Days 4–7: $0.20/day · Day 8+: $0.30/day.`
          }
        >
          <button
            type="button"
            className="button primary"
            disabled={busy || !dr.canClaim}
            onClick={() => act('usageChange', { body: { action: 'claim-daily-reward' }, said: `Claimed ${usd(dr.amount)} daily login reward (day ${dr.nextDay}).` })}
          >
            {dr.canClaim ? `Claim ${usd(dr.amount)}` : 'Claimed today'}
          </button>
        </SettingRow>
      )}
      {ref && (
        <>
          <SettingRow
            title={`${ref.partner ? 'Partner Program' : 'Referrals'} (${referred.length} referred${num(ref.earned) > 0 ? ` · ${usd(ref.earned)} earned` : ''})`}
            sub={`Your referral code is "${ref.code}". Refer as many friends as you like: they get ${usd(ref.credit)} in ScoutCode credit that expires in ${ref.expiryDays} days, and you get ${usd(ref.bonus)} in balance once they sign in to Scout and send their first prompt.`}
          >
            <CopyButton text={ref.code} label="Copy code" />
            <CopyButton text={`${site}/signup?ref=${encodeURIComponent(ref.code)}`} label="Copy link" />
          </SettingRow>
          {referred.map((r) => (
            <SettingRow
              key={r.username}
              title={`Referred ${r.username}`}
              sub={`${isoDay(r.createdAt)} · ${r.rewardedAt ? `+${usd(r.payout)} added to your balance` : 'Waiting for their first prompt in Scout'}`}
            />
          ))}
          {ref.referredBy ? (
            <SettingRow title={`Referred by ${ref.referredBy}`} sub={`You received ${usd(ref.receivedCredit)} in ScoutCode credit for ${ref.expiryDays} days.`} />
          ) : (
            ref.canRedeem !== false && (
              <SettingRow title="Redeem a referral code" sub={`Enter a friend's username or referral link to get ${usd(ref.credit)} in ScoutCode credit that expires in ${ref.expiryDays} days.`}>
                <input
                  className="input field-180"
                  placeholder="Friend's username"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && redeem()}
                />
                <button type="button" className="button primary" disabled={busy || !code.trim()} onClick={redeem}>
                  Redeem +{usd(ref.credit)}
                </button>
              </SettingRow>
            )
          )}
        </>
      )}
      {dl && (
        <>
          {dl.linked ? (
            <SettingRow
              title="Discord account linked"
              sub={
                <>
                  Linked to Discord tag &ldquo;{dl.discordTag || 'linked'}&rdquo;
                  {dl.linkedAt ? ` on ${shortDate(dl.linkedAt.slice(0, 10))}` : ''}. Earned {usd(dl.reward ?? 0.25)} balance reward. Chat with us on{' '}
                  <a
                    className="link"
                    href={DISCORD_INVITE_URL}
                    onClick={(e) => {
                      e.preventDefault();
                      openExternal(DISCORD_INVITE_URL);
                    }}
                  >
                    Discord
                  </a>.
                </>
              }
            >
              <button
                type="button"
                className="button"
                onClick={() => openExternal(DISCORD_INVITE_URL)}
              >
                Open Discord
              </button>
              <span className="small muted">Linked (+{usd(dl.reward ?? 0.25)})</span>
            </SettingRow>
          ) : (
            <SettingRow
              title="Link Discord account · Earn $0.25"
              sub={
                dl.code ? (
                  <>
                    Your link code is &ldquo;{dl.code}&rdquo; (expires in 15 minutes). Run &ldquo;/link code:{dl.code}&rdquo; on{' '}
                    <a
                      className="link"
                      href={DISCORD_INVITE_URL}
                      onClick={(e) => {
                        e.preventDefault();
                        openExternal(DISCORD_INVITE_URL);
                      }}
                    >
                      our Discord server
                    </a>{' '}
                    to earn {usd(dl.reward ?? 0.25)} balance.
                  </>
                ) : (
                  <>
                    Link your Discord account once to earn {usd(dl.reward ?? 0.25)} added to your permanent balance. Generate a code and run /link on{' '}
                    <a
                      className="link"
                      href={DISCORD_INVITE_URL}
                      onClick={(e) => {
                        e.preventDefault();
                        openExternal(DISCORD_INVITE_URL);
                      }}
                    >
                      our Discord server
                    </a>.
                  </>
                )
              }
            >
              {dl.code ? (
                <>
                  <CopyButton text={dl.code} label="Copy code" />
                  <CopyButton text={`/link ${dl.code}`} label="Copy /link" />
                  <button
                    type="button"
                    className="button"
                    disabled={busy}
                    onClick={() =>
                      act('usageChange', {
                        body: { action: 'generate-discord-code' },
                        said: 'Generated new Discord linking code.',
                      })
                    }
                  >
                    New code
                  </button>
                  <button
                    type="button"
                    className="button"
                    onClick={() => openExternal(DISCORD_INVITE_URL)}
                  >
                    Open Discord
                  </button>
                </>
              ) : (
                <>
                  <button
                    type="button"
                    className="button primary"
                    disabled={busy}
                    onClick={() =>
                      act('usageChange', {
                        body: { action: 'generate-discord-code' },
                        said: 'Generated Discord linking code.',
                      })
                    }
                  >
                    Get link code
                  </button>
                  <button
                    type="button"
                    className="button"
                    onClick={() => openExternal(DISCORD_INVITE_URL)}
                  >
                    Join Discord
                  </button>
                </>
              )}
            </SettingRow>
          )}
        </>
      )}
    </>
  );
}

function UsageLimits() {
  const { hasKey } = useView('cfg');
  const { info, busy, message } = useView('usage');
  if (!hasKey) return <p className="muted">Sign in to ScoutCode to see your credit and limits.</p>;
  const plan = (p: { limit: number; left: number } | null | undefined): [string, string] => (p ? [usd(p.left), `left of ${usd(p.limit)}`] : ['Unlimited', 'Requests are free']);
  const bankedMoney = Array.isArray(info?.bankedMoney) ? info.bankedMoney : [];
  const bankedResets = Array.isArray(info?.bankedResets) ? info.bankedResets : [];
  return (
    <div className="stack">
      {info ? (
        <>
          <div className="tiles three">
            <Tile label="Desktop credit this week (browser sign-in)" value={plan(info.cli)[0]} sub={plan(info.cli)[1]} />
            {info.cliWindow && (
              <Tile
                label="This 5-hour window"
                value={usd(info.cliWindow.left)}
                sub={`left of ${usd(info.cliWindow.limit)}${info.cliWindow.resetsAt ? `, resets ${clock(info.cliWindow.resetsAt)}` : ''}`}
              />
            )}
            <Tile label="Money (API keys)" value={plan(info.money)[0]} sub={plan(info.money)[1]} />
          </div>
          <div className="tiles three">
            <Tile label="Banked" value={usd(bankedMoney.reduce((n, b) => n + num(b.amount), 0))} sub="Spent next, runs out" />
            <Tile label="Balance" value={usd(info.balance)} sub="Spent last, doesn't reset" />
          </div>
          <div className="tiny dim">
            {info.cliResetsAt && `Desktop credit resets ${isoDay(info.cliResetsAt)} UTC. `}{info.resetsAt && <>Money resets {isoDay(info.resetsAt)} UTC.</>}
          </div>
          {bankedMoney.map((b, i) => <SettingRow key={i} title={`${usd(b.amount)} banked`} sub={`Expires ${isoDay(b.expiresAt)} UTC`} />)}
          {bankedResets.map((r, i) => (
            <SettingRow key={i} title="Banked reset" sub={`Clears your spending this month. Expires ${isoDay(r.expiresAt)} UTC.`}>
              {/* The one that runs out soonest goes first, so only the first row has the button. */}
              {i === 0 && <button type="button" className="button primary" disabled={busy} onClick={() => act('usageChange', { body: { action: 'use-banked-reset' }, said: 'Your usage this month is back to zero.' })}>Use</button>}
            </SettingRow>
          ))}
          <RewardsSection info={info} busy={busy} />
          {info.admin && <UsageAdmin info={info} busy={busy} />}
        </>
      ) : busy && <Spinner size={16} />}
      {message && <div className={message[0] ? 'green' : 'red'}>{message[1]}</div>}
    </div>
  );
}

type UsageLog = { today: [number, number]; week: [number, number]; month: [number, number]; days: { day: string; tokens: number; cost: number; requests: number }[]; seededOn: string | null };

function Usage() {
  const [log, setLog] = useState<UsageLog | null>(null);
  useEffect(() => { invoke<UsageLog>('usage_log').then(setLog); act('usageRefresh'); }, []);
  const max = Math.max(1, ...(log?.days ?? []).map((d) => d.tokens));
  return (
    <>
      <Heading title="Usage" sub="What you have left this month, each provider's limits, and the tokens Scout used each day on this PC. Days and months are UTC." />
      <UsageLimits />
      <ProviderQuotas />
      {log && (
        <div className="stack usage-log">
          <div className="tiles three">
            {([['Today', log.today], ['Last 7 days', log.week], ['Last 30 days', log.month]] as const).map(([label, [t, c]]) => <Tile key={label} label={label} value={`${shortTokens(t)} tokens`} sub={usd(c)} />)}
          </div>
          <div className="usage-chart">
            {log.days.map((d) => (
              <div key={d.day} className="usage-bar" title={`${shortDate(d.day)}\n${num(d.tokens).toLocaleString()} tokens\n${usd(d.cost)}\n${d.requests} requests`}>
                <span style={{ height: `${d.tokens ? Math.max(2, (d.tokens / max) * 100) : 0}%` }} />
              </div>
            ))}
          </div>
          <div className="row tiny dim"><span className="grow">{log.days[0] && shortDate(log.days[0].day)}</span><span>{log.days.length > 0 && shortDate(log.days[log.days.length - 1].day)}</span></div>
          <div className="tiny dim">{log.seededOn ? `Days before ${shortDate(log.seededOn)} are estimated from saved chats: each chat counts on the day it was last used.` : "Counted from each request as it's answered."}</div>
        </div>
      )}
    </>
  );
}

/** Usage as a page of its own, off the side rail. */
export function UsagePage() {
  return <div className="usage-page"><div className="settings-page"><Usage /></div></div>;
}

// ---------- account ----------

function Account() {
  const cfg = useView('cfg');
  const { info, busy, message } = useView('usage');
  useEffect(() => { if (cfg.hasKey) act('usageRefresh'); }, [cfg.hasKey]);
  const name = cfg.username ?? (cfg.hasKey ? 'Unknown until the model list loads' : 'Not signed in');
  const account = () => openExternal(`${cfg.baseUrl.replace(/\/v1$/, '')}/account`);
  return (
    <>
      <Heading title="Account" sub="The ScoutCode account this app uses. Other providers' accounts and keys are under Providers." />
      {cfg.guest ? (
        <>
          <SettingRow title="Without an account" sub={GUEST_NOTE}><button type="button" className="button" onClick={() => { act('setGuest', { on: false }); ui.set({ settings: null }); }}>Stop</button></SettingRow>
          <div className="field-420 stack">
            <BrowserEntry />
            <div className="small dim">Or use an API key:</div>
            <ScoutKeyEntry />
          </div>
        </>
      ) : (
        <>
          <SettingRow title="Signed in as" sub={name} />
          {cfg.team && <SettingRow title="Team" sub={`${cfg.team.name}, Scout Enterprise (${cfg.team.role === 'admin' ? 'admin' : 'member'}). Code review is free.`} />}
          <SettingRow title={cfg.browserSignIn ? 'Browser sign-in' : 'API key'} sub={cfg.browserSignIn ? 'Uses your weekly desktop credit, then any banked money and balance.' : "Pays from your account's money."} />
          <SettingRow title="Account page" sub="Keys, balance and billing on scoutcode.dev."><button type="button" className="button" onClick={account}>Open</button></SettingRow>
          {info && <RewardsSection info={info} busy={busy} />}
          {message && <div className={`small ${message[0] ? 'green' : 'red'}`}>{message[1]}</div>}
          <SettingRow title="Sign out" sub="Forgets the login on this PC."><button type="button" className="button" onClick={() => { act('signOut'); ui.set({ settings: null }); }}>Sign out</button></SettingRow>
        </>
      )}
    </>
  );
}

// ---------- providers ----------

function OpenRouterKeys() {
  const { openrouterKeys: keys } = useView('cfg');
  const [name, setName] = useState('');
  return (
    <>
      <p className="small dim">Tried from the top. If one fails, the next one is used. Saved on this PC. Models on OpenRouter bill your OpenRouter account, and can only be used in a custom agent: add them to a route under Custom agents.</p>
      {keys.map((k, i) => (
        <SettingRow key={i} title={`${i + 1}. ${k.name}`} sub={k.masked}>
          {i > 0 && <button type="button" className="button" onClick={() => act('editOpenrouterKey', { at: i, op: 'up' })}>Up</button>}
          {i + 1 < keys.length && <button type="button" className="button" onClick={() => act('editOpenrouterKey', { at: i, op: 'down' })}>Down</button>}
          <button type="button" className="button" onClick={() => act('editOpenrouterKey', { at: i, op: 'remove' })}>Unlink</button>
        </SettingRow>
      ))}
      <div className="field-420 stack">
        <input className="input" placeholder="Name, e.g. Work" value={name} onChange={(e) => setName(e.target.value)} />
        <KeyField hint="sk-or-…" button={keys.length ? 'Add another key' : 'Link with a key'} onSave={(key) => { act('addOpenrouterKey', { name: name.trim(), key }); setName(''); }} />
      </div>
    </>
  );
}

type OwnWhich = 'copilot' | 'devin' | 'kiro' | 'antigravity' | 'vibe' | 'codex' | 'claude';

function OwnAccount({ which, title, note, who, planText, planUrl }: { which: OwnWhich; title: string; note: string; who: string | null; planText: string; planUrl: string }) {
  const rowTitle = which === 'copilot' ? 'GitHub account' : which === 'antigravity' ? 'Google account' : which === 'vibe' ? 'Mistral account' : `${title} account`;
  return (
    <>
      <p className="small dim">{note}</p>
      {who ? (
        <SettingRow title={rowTitle} sub={who}>
          <button type="button" className="button" onClick={() => act('signOutOf', { which, uninstall: false })}>Sign out</button>
          <button type="button" className="button" onClick={() => act('signOutOf', { which, uninstall: true })}>Sign out and uninstall</button>
        </SettingRow>
      ) : <div className="field-420"><SignInEntry which={which} /></div>}
      {who && (which === 'codex' || which === 'claude' || which === 'copilot') && <AgentAccountList which={which} />}
      <Link href={planUrl} className="small">{planText}</Link>
    </>
  );
}

/** Codex's or Claude Code's accounts on this PC: which one runs, and adding another. */
function AgentAccountList({ which }: { which: AccountProvider }) {
  const p = useView('cfg').agentAccounts[which];
  const busy = !!useView('login')[which];
  return (
    <>
      {p.list.length >= 2 && p.list.map((a, i) => (
        <SettingRow key={a.id} title={accountLabel(a, i)} sub={a.id === p.active ? 'In use' : 'Saved on this PC'}>
          {a.id !== p.active && <button type="button" className="button" disabled={busy} onClick={() => act('switchAccount', { which, id: a.id })}>Switch</button>}
          <button type="button" className="button" disabled={busy} onClick={() => act('removeAccount', { which, id: a.id })}>{a.id === p.active ? 'Sign out' : 'Remove'}</button>
        </SettingRow>
      ))}
      {busy
        ? <div className="field-420"><SignInEntry which={which} /></div>
        : (
          <SettingRow title="Add another account" sub={p.list.length >= 2 ? 'Pick which account runs from the model menu, or here.' : 'Sign in to a second account, then switch between them from the model menu.'}>
            <button type="button" className="button" onClick={() => act('addAccount', { which })}>Add</button>
          </SettingRow>
        )}
    </>
  );
}

type ProviderEntry = { id: string; name: string; linked: boolean; status: string; body: ReactNode };

function ProviderRow({ p, open, onToggle }: { p: ProviderEntry; open: boolean; onToggle: () => void }) {
  const unlinkedStatus = !p.linked ? providerStatus(p.id) : null;
  return (
    <div className={`provider-row ${open ? 'open' : ''}`}>
      <button type="button" className="provider-head" aria-expanded={open} onClick={onToggle}>
        <img className="logo" src={logo(p.id)} width={22} height={22} alt="" />
        <div className="grow min0">
          <div className="setting-title">{p.name}</div>
          {p.linked && <div className="small dim ellipsis">{p.status}</div>}
        </div>
        {p.linked ? <span className="badge">Linked</span> : <span className={`badge ${unlinkedStatus === 'Paid' ? 'paid' : ''}`}>{unlinkedStatus}</span>}
        <Icon name={open ? 'chevronDown' : 'chevronRight'} />
      </button>
      {open && <div className="provider-body stack">{p.body}</div>}
    </div>
  );
}

/** Every provider of models besides ScoutCode: what's linked, and linking or unlinking each one. */
function Providers() {
  const cfg = useView('cfg');
  const login = useView('login');
  const local = useView('local');
  const { plans, kiloWarning, additionalProviders } = useCore((s) => s.statics);
  const [open, setOpen] = useState<string | null>(null);
  const own = (which: OwnWhich, name: string, note: string, on: boolean, planText: string, who = 'Signed in'): ProviderEntry => ({
    id: which, name, linked: on, status: login[which] ? 'Signing in…' : on ? who : providerStatus(which),
    body: <OwnAccount which={which} title={name} note={note} who={on ? who : null} planText={planText} planUrl={plans[which]} />,
  });
  // The account in use, by its email once known, with how many there are.
  const activeAccount = (w: AccountProvider) => {
    const p = cfg.agentAccounts[w];
    const at = p.list.findIndex((a) => a.id === p.active);
    const name = p.list[at]?.name ?? 'Signed in';
    return p.list.length >= 2 ? `${name} (${at + 1} of ${p.list.length} accounts)` : name;
  };
  const keyRow = (title: string, masked: string, onRemove: () => void) => <SettingRow title={title} sub={masked}><button type="button" className="button" onClick={onRemove}>Unlink</button></SettingRow>;
  const all: ProviderEntry[] = [
    {
      id: 'scoutcode', name: 'ScoutCode', linked: cfg.hasKey || cfg.guest,
      status: cfg.hasKey ? 'Signed in' : cfg.guest ? 'Free models only' : providerStatus('scoutcode'),
      body: (
        <>
          <p className="small dim">{GUEST_NOTE}</p>
          {cfg.hasKey ? (
            <SettingRow title="ScoutCode account" sub={cfg.username ?? 'Signed in'}>
              <button type="button" className="button" onClick={() => act('signOut')}>Sign out</button>
            </SettingRow>
          ) : (
            <div className="field-420 stack">
              <BrowserEntry />
              <GuestEntry />
              <div className="small dim">Or use an API key:</div>
              <ScoutKeyEntry />
            </div>
          )}
        </>
      ),
    },
    own('claude', 'Claude Code', CLAUDE_NOTE, cfg.claude, "See Claude's plans", activeAccount('claude')),
    own('codex', 'Codex', CODEX_NOTE, cfg.codex, "See Codex in ChatGPT's plans", activeAccount('codex')),
    own('copilot', 'GitHub Copilot', COPILOT_NOTE, cfg.copilot, 'No Copilot plan yet? Turn on the free one', activeAccount('copilot')),
    {
      id: 'openrouter', name: 'OpenRouter', linked: cfg.openrouterKeys.length > 0,
      status: cfg.openrouterKeys.length ? `${cfg.openrouterKeys.length} key${cfg.openrouterKeys.length === 1 ? '' : 's'}` : providerStatus('openrouter'),
      body: <OpenRouterKeys />,
    },
    {
      id: 'nvidia', name: 'NVIDIA NIM', linked: !!cfg.nvidiaKey, status: cfg.nvidiaKey ?? providerStatus('nvidia'),
      body: (
        <>
          <p className="small dim">Saved on this PC. Models on NVIDIA NIM use your NVIDIA account.</p>
          {cfg.nvidiaKey ? keyRow('NVIDIA key', cfg.nvidiaKey, () => act('saveNvidiaKey', { key: null })) : <div className="field-420"><NvidiaEntry /></div>}
        </>
      ),
    },
    {
      id: 'opencode', name: 'OpenCode', linked: cfg.opencode,
      status: login.opencode ? 'Signing in…' : !cfg.opencode ? providerStatus('opencode') : cfg.opencodeSignedIn ? 'Signed in' : 'Free models only',
      body: (
        <>
          <p className="small dim">{OPENCODE_NOTE}</p>
          {cfg.opencode ? (
            <>
              <SettingRow title="OpenCode account" sub={cfg.opencodeSignedIn ? activeAccount('opencode') : 'Not signed in: free models only'}>
                {cfg.opencodeSignedIn && <button type="button" className="button" onClick={() => act('signOutOf', { which: 'opencode', uninstall: false })}>Sign out</button>}
                <button type="button" className="button" onClick={() => act('setOpencode', { on: false, uninstall: false })}>Turn off</button>
                <button type="button" className="button" onClick={() => act('setOpencode', { on: false, uninstall: true })}>Turn off and uninstall</button>
              </SettingRow>
              {!cfg.opencodeSignedIn && <div className="field-420"><SignInEntry which="opencode" /></div>}
              {cfg.opencodeSignedIn && <AgentAccountList which="opencode" />}
            </>
          ) : <div className="field-420"><OpencodeEntry /></div>}
          <Link href={plans.opencode} className="small">Open the OpenCode console</Link>
        </>
      ),
    },
    {
      id: 'grok', name: 'Grok Build', linked: cfg.grokSignedIn || !!cfg.grokKey,
      status: login.grok ? 'Signing in…' : cfg.grokSignedIn ? 'Signed in' : cfg.grokKey ?? providerStatus('grok'),
      body: (
        <>
          <p className="small dim">{GROK_NOTE}</p>
          {cfg.grokSignedIn || cfg.grokKey ? (
            <>
              <SettingRow title="Grok account" sub={cfg.grokSignedIn ? activeAccount('grok') : 'Not signed in'}>
                {cfg.grokSignedIn && <button type="button" className="button" onClick={() => act('signOutOf', { which: 'grok', uninstall: false })}>Sign out</button>}
                <button type="button" className="button" onClick={() => act('signOutOf', { which: 'grok', uninstall: true })}>Remove and uninstall</button>
              </SettingRow>
              {!cfg.grokSignedIn && <div className="field-420"><SignInEntry which="grok" /></div>}
              {cfg.grokSignedIn && <AgentAccountList which="grok" />}
              {cfg.grokKey
                ? keyRow('xAI key', cfg.grokSignedIn ? `${cfg.grokKey}: used when signed out` : cfg.grokKey, () => act('saveGrokKey', { key: null }))
                : !login.grok && <div className="field-420"><GrokKeyEntry /></div>}
            </>
          ) : <div className="field-420"><GrokEntry /></div>}
          <Link href={plans.grok} className="small">Open the xAI console</Link>
        </>
      ),
    },
    {
      id: 'hermes', name: 'Hermes', linked: cfg.hermesSignedIn || !!cfg.hermesKey,
      status: login.hermes ? 'Signing in…' : cfg.hermesSignedIn ? 'Signed in' : cfg.hermesKey ?? providerStatus('hermes'),
      body: (
        <>
          <p className="small dim">{HERMES_NOTE}</p>
          {cfg.hermesSignedIn || cfg.hermesKey ? (
            <>
              {cfg.hermesSignedIn
                ? <SettingRow title="Nous Portal account" sub="Signed in"><button type="button" className="button" onClick={() => act('signOutOf', { which: 'hermes', uninstall: false })}>Sign out</button></SettingRow>
                : <div className="field-420"><SignInEntry which="hermes" /></div>}
              {cfg.hermesKey
                ? keyRow('Nous Portal key', cfg.hermesSignedIn ? `${cfg.hermesKey}: used when signed out` : cfg.hermesKey, () => act('saveHermesKey', { key: null }))
                : !login.hermes && <div className="field-420"><HermesKeyEntry /></div>}
            </>
          ) : <div className="field-420"><HermesEntry /></div>}
          <Link href={plans.hermes} className="small">Open Nous Portal</Link>
        </>
      ),
    },
    {
      id: 'local', name: 'Local models (llama.cpp)', linked: cfg.local.models.length > 0,
      status: localStatus(cfg.local, local),
      body: <LocalSetup mode="settings" />,
    },
    own('devin', 'Devin', DEVIN_NOTE, cfg.devin, "See Devin's plans"),
    own('kiro', 'Kiro', KIRO_NOTE, cfg.kiro, "See Kiro's plans"),
    own('antigravity', 'Antigravity', ANTIGRAVITY_NOTE, cfg.antigravity, 'Open Antigravity'),
    own('vibe', 'Mistral Vibe', VIBE_NOTE, cfg.vibe, 'Open the Mistral console'),
    ...additionalProviders.map((a): ProviderEntry => {
      const l = cfg.additional.find((x) => x.id === a.id);
      if (a.custom) {
        return { id: a.id, name: a.name, linked: true, status: a.note, body: <CustomProviderDetails p={a.custom} masked={l?.masked ?? null} /> };
      }
      return {
        id: a.id, name: a.name, linked: !!l, status: l ? l.masked ?? 'Free, no key' : providerStatus(a.id),
        body: (
          <>
            <p className="small dim">{a.note}</p>
            {l ? (
              <>
                {keyRow(l.masked ? `${a.name} key` : 'Free, no key', l.masked ?? 'Add a key below for higher limits.', () => act('unlinkAdditional', { id: a.id }))}
                {/* Linked without a key, so one is only for higher limits. */}
                {!l.masked && <div className="field-420"><AdditionalKey id={a.id} /></div>}
              </>
            ) : a.needsKey
              ? <div className="field-420"><AdditionalKey id={a.id} /></div>
              : <div className="field-420"><button type="button" className="button wide" onClick={() => act('linkAdditional', { id: a.id, key: null })}>Use {a.name} for free</button></div>}
          </>
        ),
      };
    }),
    {
      // Its free models need nothing, so it's always linked; a key only adds the paid ones.
      id: 'kilo', name: 'Kilo Code', linked: true, status: cfg.kiloKey ? `Free and paid models, key ${cfg.kiloKey}` : 'Free models, no key',
      body: (
        <>
          <p className="small dim">Its free models need no account or key, so they're always in the model menu. {kiloWarning}</p>
          {cfg.kiloKey ? keyRow('Kilo key', cfg.kiloKey, () => act('removeKiloKey')) : (
            <div className="field-420 stack">
              <p className="small dim">Optional: a key from your Kilo account, for its paid models.</p>
              <KeyField hint="Kilo API key" button="Save Kilo key" onSave={(key) => act('saveKilo', { enabled: true, key })} />
            </div>
          )}
          <div><button type="button" className="button" onClick={() => act('checkKilo')}>Test connection</button></div>
          {login.kiloCheck && <div className={`small ${login.kiloCheck.ok ? 'muted' : 'red'}`}>{login.kiloCheck.message}</div>}
        </>
      ),
    },
  ];
  const list = (ps: ProviderEntry[]) => <div className="provider-list">{ps.map((p) => <ProviderRow key={p.id} p={p} open={open === p.id} onToggle={() => setOpen(open === p.id ? null : p.id)} />)}</div>;
  if (cfg.team) {
    // A Scout Enterprise team's member: the team's providers, whose keys stay on the server, and the ones on this PC it allows.
    const names: Record<string, string> = { scoutcode: 'ScoutCode Cloud', openrouter: 'OpenRouter', nvidia: 'NVIDIA NIM', kilo: 'Kilo Code', hermes: 'Hermes', ...Object.fromEntries(additionalProviders.map((a) => [a.id, a.name])) };
    const allowed = all.filter((p) => cfg.team!.acp.includes(p.id));
    return (
      <>
        <Heading title="Providers" sub={`${cfg.team.name} manages your providers and models. To add one, ask a team admin.`} />
        <SectionLabel>From your team</SectionLabel>
        {cfg.team.providers.length
          ? cfg.team.providers.map((id) => <SettingRow key={id} title={names[id] ?? id} sub="Through ScoutCode with your team's key, which isn't kept on this PC." />)
          : <p className="small dim">Your team hasn't turned on any providers yet.</p>}
        {allowed.length > 0 && <><SectionLabel>Sign in on this PC</SectionLabel><p className="small dim">Your team allows these. They run on this PC with your own account.</p>{list(allowed)}</>}
      </>
    );
  }
  const rest = [
    ...all.filter((p) => !p.linked),
    { id: CUSTOM_NEW, name: 'Your own provider', linked: false, status: '', body: <><p className="small dim">{CUSTOM_NOTE}</p><div className="field-420"><CustomProviderForm onDone={() => setOpen(null)} /></div></> },
  ];
  return (
    <>
      <Heading title="Providers" sub="Link your own accounts and keys to add their models to the model menu. Unlinking one takes its models out of the menu; your chats stay." />
      <SectionLabel>Linked</SectionLabel>
      {list(all.filter((p) => p.linked))}
      {rest.length > 0 && <><SectionLabel>Not linked</SectionLabel>{list(rest)}</>}
    </>
  );
}

// ---------- skills ----------

type Listing = { key: string; name: string; source: string; id: string; installs: number; url: string };

function Skills() {
  const skills = useView('skills');
  const [message, setMessage] = useState<[boolean, string] | null>(null);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Listing[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);
  const search = () => {
    setSearching(true);
    invoke<Listing[]>('skill_search', { query }).then((r) => { setResults(r); setMessage(null); }, (e) => setMessage([false, String(e)])).finally(() => setSearching(false));
  };
  useEffect(search, []); // eslint-disable-line
  const upload = (folder: boolean) => invoke<string[] | null>('skill_upload', { folder }).then((names) => {
    if (names) setMessage([true, `Added. Type ${names.map((n) => `@${n}`).join(' or ')} in a message to use ${names.length === 1 ? 'it' : 'one'}.`]);
  }, (e) => setMessage([false, String(e)]));
  return (
    <>
      <Heading title="Skills" sub="Instructions Scout follows when you ask for them. Type @ and the skill's name in a message to send it along. Skills are written by other people and can tell Scout to run things, so read one before you use it. Scout's permissions still apply." />
      <div className="row">
        <button type="button" className="button primary" onClick={() => upload(true)}>Upload a folder</button>
        <button type="button" className="button" onClick={() => upload(false)}>Upload a file</button>
      </div>
      <Message m={message} />
      <SectionLabel>Installed</SectionLabel>
      {skills.length === 0 && <p className="small dim">None yet. Upload one, or install one from the directory below.</p>}
      {skills.map((s) => (
        <SettingRow key={s.name} title={`@${s.name}`} sub={s.about || (s.source ? `From ${s.source}` : 'No description')}>
          <button type="button" className="button" onClick={() => invoke('reveal', { path: `${s.dir}\\SKILL.md` })}>Show files</button>
          <button type="button" className="button ghost" onClick={() => invoke('skill_remove', { name: s.name }).then(() => setMessage(null), (e) => setMessage([false, String(e)]))}>Remove</button>
        </SettingRow>
      ))}
      <SectionLabel>Directory</SectionLabel>
      <p className="small dim">Skills from <Link href="https://skills.sh">skills.sh</Link>. Open a skill's page to read it and its security audits first.</p>
      <div className="row">
        <input className="input field-280" placeholder="Search skills" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && search()} />
        <button type="button" className="button" disabled={searching} onClick={search}>Search</button>
        {searching && <Spinner />}
      </div>
      {results?.length === 0 && !searching && !message && <p className="small dim">Nothing matches.</p>}
      {results?.map((l) => {
        const have = skills.some((s) => s.source === l.key);
        return (
          <SettingRow key={l.key} title={l.name} sub={`${l.source} · ${l.installs} installs`}>
            {installing === l.key ? <Spinner /> : (
              <button type="button" className="button" disabled={installing !== null} onClick={() => {
                setInstalling(l.key);
                setMessage(null);
                invoke<string>('skill_install', { source: l.source, id: l.id, name: l.name, installs: l.installs })
                  .then((name) => setMessage([true, `Installed. Type @${name} in a message to use it.`]), (e) => setMessage([false, String(e)]))
                  .finally(() => setInstalling(null));
              }}>{have ? 'Update' : 'Install'}</button>
            )}
            <Link href={l.url} className="small">Read it</Link>
          </SettingRow>
        );
      })}
    </>
  );
}

// ---------- MCP ----------

type McpFormState = { editing: string | null; name: string; remote: boolean; command: string; args: string; env: string; url: string; headers: string };
const EMPTY_MCP: McpFormState = { editing: null, name: '', remote: false, command: '', args: '', env: '', url: '', headers: '' };

function McpStateLine(s: McpServer): string {
  switch (s.state) {
    case 'off': return 'Off';
    case 'approval': return 'Waiting for your OK before it connects';
    case 'connecting': return 'Connecting…';
    case 'ready': return `Connected${s.signedIn ? ' · signed in' : ''} · ${s.tools.length} tool${s.tools.length === 1 ? '' : 's'}`;
    case 'signIn': return 'Needs you to sign in';
    case 'signingIn': return 'Waiting for you to sign in, in your browser…';
    case 'failed': return `Couldn't connect: ${s.error}`;
  }
}

function Mcp() {
  const { error, servers } = useView('mcp');
  const [form, setForm] = useState<McpFormState | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const field = (label: string, key: keyof McpFormState, hint: string, rows = 1) => (
    <label className="stack">
      <span className="small muted">{label}</span>
      <textarea className="input" rows={rows} placeholder={hint} value={String(form![key] ?? '')} onChange={(e) => setForm({ ...form!, [key]: e.target.value })} />
    </label>
  );
  return (
    <>
      <Heading title="MCP servers" sub="Give Scout more tools by connecting it to MCP servers, on this computer or remote. Scout asks before each call until you choose to always allow a tool. A server runs with your access to this computer, so only add ones you trust. Claude Code, Codex, Devin, Kiro, OpenCode and Grok Build use their own tools, so they don't get these." />
      {error && <div className="small red">{error}</div>}
      {servers.length === 0 && <p className="small dim">None yet.</p>}
      {servers.map((s) => (
        <div key={s.name} className="stack">
          <SettingRow title={s.name} sub={`${s.summary}\n${McpStateLine(s)}`}>
            {s.state === 'approval' && <button type="button" className="button primary" onClick={() => act('mcpAllow', { name: s.name })}>Allow</button>}
            {s.state === 'signIn' && <button type="button" className="button primary" onClick={() => act('mcpSignIn', { name: s.name })}>Sign in</button>}
            {s.state === 'signingIn' && <button type="button" className="button" onClick={() => act('mcpCancelSignIn')}>Cancel sign-in</button>}
            {s.state === 'failed' && s.form.remote && !/^authorization\s*:/im.test(s.form.headers) && <button type="button" className="button" onClick={() => act('mcpSignIn', { name: s.name })}>Sign in</button>}
            {s.signedIn && s.state !== 'signingIn' && <button type="button" className="button ghost" onClick={() => act('mcpSignOut', { name: s.name })}>Sign out</button>}
            <button type="button" className="button" onClick={() => { setFormError(null); setForm({ ...s.form, editing: s.name, name: s.name }); }}>Edit</button>
            <button type="button" className="button ghost" onClick={() => act('mcpRemove', { name: s.name })}>Remove</button>
            <Toggle label={`Turn ${s.name} on or off`} on={!s.disabled} onChange={(on) => act('mcpSetEnabled', { name: s.name, on })} />
          </SettingRow>
          {s.tools.length > 0 && <div className="tiny dim">{s.tools.join(', ')}</div>}
        </div>
      ))}
      <div className="row save-row">
        {!form && <button type="button" className="button primary" onClick={() => { setFormError(null); setForm(EMPTY_MCP); }}>Add a server</button>}
        <button type="button" className="button" onClick={() => act('mcpReconnect')}>Reconnect all</button>
        <button type="button" className="button" onClick={() => invoke('mcp_show_file')}>Show mcp.json</button>
      </div>
      <p className="small dim">mcp.json uses the same mcpServers format as other apps, so you can paste a server's setup in, then press Reconnect all.</p>
      {form && (
        <div className="stack">
          <SectionLabel>{form.editing ? 'Change server' : 'New server'}</SectionLabel>
          {field('Name', 'name', 'github')}
          <div className="row">
            <button type="button" className={`button ${form.remote ? 'ghost' : ''}`} onClick={() => setForm({ ...form, remote: false })}>Runs on this computer</button>
            <button type="button" className={`button ${form.remote ? '' : 'ghost'}`} onClick={() => setForm({ ...form, remote: true })}>Remote (URL)</button>
          </div>
          {form.remote ? (
            <>
              {field('URL (Streamable HTTP)', 'url', 'https://example.com/mcp')}
              {field('Headers, one per line (optional). Leave out Authorization and Scout signs in through your browser if the server asks.', 'headers', 'Authorization: Bearer <token>', 2)}
            </>
          ) : (
            <>
              {field('Command', 'command', 'npx')}
              {field('Arguments', 'args', '-y @modelcontextprotocol/server-everything')}
              {field('Environment variables, one per line', 'env', 'API_KEY=...', 2)}
            </>
          )}
          <div className="row">
            <button type="button" className="button primary" onClick={() => invoke('mcp_save', { form }).then(() => setForm(null), (e) => setFormError(String(e)))}>Save and connect</button>
            <button type="button" className="button ghost" onClick={() => setForm(null)}>Cancel</button>
          </div>
          {formError && <div className="small red">{formError}</div>}
        </div>
      )}
    </>
  );
}

// ---------- chatbots ----------

function Chatbots() {
  const link = useView('link');
  const cfg = useView('cfg');
  const { modes, exe } = useCore((s) => s.statics);
  const [mode, setMode] = useState(link.mode ?? 'default');
  const canLink = cfg.hasKey && !!link.cwd;
  const status = link.status || (!cfg.hasKey ? 'This goes through your ScoutCode account, so sign in first.' : 'Off.');
  const address = `${cfg.baseUrl}/mcp`;
  const snippet = JSON.stringify({ mcpServers: { scout: { command: exe, args: ['mcp', '--cwd', link.cwd] } } }, null, 2);
  return (
    <>
      <Heading title="Chatbots" sub="Let a chatbot use Scout's tools on this computer over MCP: its model does the thinking, and it can reach the files and shell here, in the folder you choose. Nothing is spent on ScoutCode." />
      <SectionLabel>Web chatbots, like claude.ai and ChatGPT</SectionLabel>
      <SettingRow title="Folder" sub={link.cwd || 'Open a project first.'} />
      <SettingRow title="What it may do without asking" sub="The rest waits for your OK here. Turn the link off and on to change this. The link is on unless you turn it off, comes back on when Scout starts, and follows the project you have open.">
        <Select className="input select" disabled={link.on} value={mode} onChange={(e) => setMode(e.target.value)}>
          {modes.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
        </Select>
      </SettingRow>
      <SettingRow title="Link" sub={status}><button type="button" className="button" disabled={!link.on && !canLink} onClick={() => act('toggleLink', { mode })}>{link.on ? 'Turn off' : 'Turn on'}</button></SettingRow>
      {mode === 'yolo' && <p className="small red">Yolo lets the chatbot run any command on this computer without asking.</p>}
      <SettingRow title="Connector address" sub={address}><CopyButton text={address} /></SettingRow>
      <p className="small dim">Add this to the chatbot as a connector and sign in when it asks. Unlink a chatbot from your account page.</p>
      <ClaudeSetup address={address} disabled={!cfg.hasKey} />
      <ClaudeSetup address={address} disabled={!cfg.hasKey} bot="gemini" />
      {link.log.length > 0 && (
        <div className="stack">
          <div className="small dim">Latest calls</div>
          {[...link.log].reverse().map((c, i) => <div key={i} className="mono small code-text ellipsis">{`${c.tool} ${c.label}`.split('\n')[0].slice(0, 100)}</div>)}
        </div>
      )}
      <SectionLabel>Chatbot apps on this computer, like Claude Desktop and Cursor</SectionLabel>
      <p className="small dim">Add this to the app's MCP settings. It's read-only and can't ask you anything. To allow more, add "--mode" and acceptEdits, auto or yolo to args.</p>
      <CodeBlock code={snippet} />
    </>
  );
}

function ScoutMote() {
  const sm = useView('scoutMote');
  const cfg = useView('cfg');
  const site = cfg.baseUrl.replace(/\/v1\/?$/, '');
  const status = !cfg.hasKey ? 'ScoutMote goes through your ScoutCode account, so sign in first.' : sm.status || (sm.on ? 'Connecting…' : 'Off.');
  return (
    <>
      <Part title="ScoutMote" sub="Use Scout on this computer from any browser: start chats, watch them work, answer what it asks and stop it. It works while Scout is open here and this PC is online." />
      <SettingRow title="Control this PC from the web" sub={status}>
        <Toggle label="Control this PC from the web" on={sm.on} onChange={(on) => act('setScoutMote', { on })} />
      </SettingRow>
      <SettingRow title="Shows up as" sub={sm.name} />
      {sm.on && cfg.hasKey && <SettingRow title="Open ScoutMote" sub={`Sign in at ${site} with the same account.`}><button type="button" className="button" onClick={() => openExternal(site)}>Open</button></SettingRow>}
      <p className="small dim">Prompts from the web run in this chat's permission mode, like ones typed here. What the mode doesn't allow waits for an OK, which you can give here or in the browser. Only your projects can be opened, and nothing else on your account can reach this PC.</p>
    </>
  );
}

// ---------- updates ----------

function Privacy() {
  const { usageAnalytics } = useView('cfg');
  return (
    <>
      <Part title="Privacy" sub="What Scout sends to ScoutCode about how the app is used." />
      <SettingRow
        title="Send usage analytics"
        sub="Scout's version, when it opens, and which provider and model each message ran on, so we know what to improve. Never your messages, files, folders or keys."
      >
        <Toggle label="Send usage analytics" on={usageAnalytics} onChange={(on) => act('setAnalytics', { on })} />
      </SettingRow>
    </>
  );
}

function Updates() {
  const { enabled, version, commit, status } = useView('updater');
  const { autoUpdate } = useView('cfg');
  const { anyRunning } = useView('run');
  const dirty = useUi((u) => u.editorDirty);
  // The commit it was built from, so this copy can be checked against the source.
  const built = commit && commit !== 'unknown' ? ` (${commit.slice(0, 7)}${commit.endsWith('-dirty') ? ', uncommitted changes' : ''})` : '';
  const title = `Scout ${version}${built}`;
  if (!enabled) return (
    <>
      <Heading title="Updates" sub="Scout checks for new versions when it starts and every few hours. They're signed by ScoutCode and checked before they're installed." />
      <SettingRow title={title} sub="This copy doesn't update itself. Only the installed app does, not a development build." />
    </>
  );
  const blocker = restartBlocker(anyRunning, dirty);
  let line = '';
  let notes: { summary: string; changelog: string } | null = null;
  switch (status.state) {
    case 'idle': line = "Hasn't checked yet."; break;
    case 'checking': line = 'Checking for updates…'; break;
    case 'upToDate': line = "You're on the latest version."; break;
    case 'available': line = `Version ${status.version} is available.`; notes = status; break;
    case 'downloading': line = `Downloading version ${status.version}: ${Math.floor((status.got * 100) / Math.max(1, status.total))}%`; break;
    case 'ready': line = `Version ${status.version} is ready. It installs when you restart or close Scout.`; notes = status; break;
    case 'failed': line = status.error; break;
  }
  return (
    <>
      <Heading title="Updates" sub="Scout checks for new versions when it starts and every few hours. They're signed by ScoutCode and checked before they're installed." />
      <SettingRow title={title} sub={line}>
        {status.state === 'ready' && <button type="button" className="button primary" disabled={!!blocker} title={blocker ?? undefined} onClick={() => invoke('restart_to_update')}>Restart to update</button>}
        {status.state === 'available' && <button type="button" className="button primary" onClick={() => act('downloadUpdate')}>Download</button>}
        {(status.state === 'checking' || status.state === 'downloading') && <Spinner size={16} />}
        {(status.state === 'idle' || status.state === 'upToDate' || status.state === 'failed') && <button type="button" className="button" onClick={() => act('checkUpdates')}>Check now</button>}
      </SettingRow>
      <SettingRow title="Download automatically" sub="New versions download in the background and wait for your next restart."><Toggle label="Download automatically" on={autoUpdate} onChange={(on) => act('setAutoUpdate', { on })} /></SettingRow>
      {notes?.summary.trim() && (
        <div className="stack">
          <SectionLabel>Update summary</SectionLabel>
          <div className="strong">{notes.summary}</div>
        </div>
      )}
      {notes?.changelog.trim() && (
        <div className="stack">
          <SectionLabel>Changelog</SectionLabel>
          <div className="muted pre-wrap">{notes.changelog}</div>
        </div>
      )}
    </>
  );
}

type LogEntry = { name: string; modified: number; bytes: number; current: boolean; closed: boolean };

function sizeLabel(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** One session's log: copy it, or save it as a file. */
function LogRow({ entry, n }: { entry: LogEntry; n: number }) {
  const [copied, setCopied] = useState<'idle' | 'busy' | 'done'>('idle');
  const [error, setError] = useState('');
  const when = new Date(entry.modified).toLocaleString();
  const title = entry.current ? 'This session' : n === 1 ? 'Last session' : `${n} sessions ago`;
  const state = entry.current ? `Running now · ${sizeLabel(entry.bytes)}` : `${entry.closed ? 'Closed normally' : "Didn't close normally (crashed, froze or was ended)"} · last written ${when} · ${sizeLabel(entry.bytes)}`;
  const copy = async () => {
    setCopied('busy');
    setError('');
    try {
      await navigator.clipboard.writeText(await invoke<string>('logs_read', { name: entry.name }));
      setCopied('done');
      setTimeout(() => setCopied('idle'), 1500);
    } catch (e) {
      setError(String(e));
      setCopied('idle');
    }
  };
  const save = () => { setError(''); invoke('logs_save', { name: entry.name }).catch((e) => setError(String(e))); };
  return (
    <SettingRow title={title} sub={<>{state}{error && <><br /><span className="red">{error}</span></>}</>}>
      <button type="button" className="button" onClick={copy} disabled={copied === 'busy'}><Icon name="copy" />{copied === 'done' ? 'Copied' : 'Copy'}</button>
      <button type="button" className="button" onClick={save}><Icon name="download" />Save…</button>
    </SettingRow>
  );
}

function Logs() {
  const [logs, setLogs] = useState<LogEntry[] | null>(null);
  const load = () => { invoke<LogEntry[]>('logs_list').then(setLogs, () => setLogs([])); };
  useEffect(load, []);
  const crashed = logs?.some((l) => !l.current && !l.closed);
  return (
    <>
      <Heading title="Logs" sub="Scout keeps a log of each of its last three sessions: what it did, what failed, and any crash. Keys and tokens are taken out. Copy or save one to send it to us on Discord when something went wrong." />
      {crashed && <p className="small">A recent session didn't close normally. Its log ends just before it stopped, which is what we need to see.</p>}
      {logs === null && <Spinner size={16} />}
      {logs?.length === 0 && <p className="small dim">No logs yet.</p>}
      {logs?.map((l, i) => <LogRow key={l.name} entry={l} n={logs[0]?.current ? i : i + 1} />)}
      <div className="row">
        <button type="button" className="button ghost" onClick={load}><Icon name="refresh" />Refresh</button>
        <button type="button" className="button ghost" onClick={() => invoke('logs_open_folder')}><Icon name="folderOpen" />Show in folder</button>
      </div>
    </>
  );
}

/** The smaller choices about how Scout behaves, on one page. */
function Closing() {
  const { confirmClose } = useView('cfg');
  return (
    <>
      <Part title="Closing" sub="What happens when you close Scout's window." />
      <SettingRow title="Ask before closing" sub="Closing the window asks first, and says if a chat is still running.">
        <Toggle label="Ask before closing" on={confirmClose} onChange={(on) => act('setConfirmClose', { on })} />
      </SettingRow>
    </>
  );
}

function Preferences() {
  return (
    <>
      <Heading title="Preferences" sub="How Scout lets you know, what it shares, and the helpers that run beside it." />
      <Notifications />
      <Closing />
      <Discord />
      <ExplorePage />
      <ScoutMote />
      <Privacy />
    </>
  );
}

const GROUPS = ['General', 'Models', 'Tools', 'Scout'];

/** Settings: the whole window, with its sections grouped down the left and the open one on the right. */
export function Settings({ windowButtons }: { windowButtons?: ReactNode }) {
  const tab = useUi((u) => u.settings);
  const [search, setSearch] = useState('');
  const close = () => { ui.set({ settings: null }); setSearch(''); };
  useEffect(() => {
    if (!tab) return;
    const key = (e: KeyboardEvent) => {
      // A dialog opened from here closes first.
      if (e.key !== 'Escape' || e.defaultPrevented || document.querySelector('.modal-back')) return;
      e.preventDefault();
      close();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [!!tab]); // eslint-disable-line
  if (!tab) return null;
  const q = search.trim().toLowerCase().split(/\s+/).filter(Boolean);
  // Every word typed has to appear in the section's title or keywords, in any order.
  const shown = TABS.filter((t) => q.every((w) => `${t.title.toLowerCase()} ${t.keywords}`.includes(w)));
  const current = TABS.find((t) => t.id === tab);
  return (
    <div className="settings settings-screen" role="dialog" aria-modal aria-label="Settings">
      <header className="settings-bar" data-tauri-drag-region>
        <button type="button" className="settings-back" onClick={close} title="Back to Scout (Esc)"><Icon name="back" size={15} />Back</button>
        <b className="chat-title" data-tauri-drag-region>Settings{current ? ` · ${current.title}` : ''}</b>
        <span className="grow" data-tauri-drag-region />
        {windowButtons}
      </header>
      <div className="settings-body">
        <nav className="settings-nav">
          <div className="side-search"><Icon name="search" /><input placeholder="Search settings" value={search} onChange={(e) => setSearch(e.target.value)} /></div>
          {GROUPS.map((g) => {
            const tabs = shown.filter((t) => t.group === g);
            if (!tabs.length) return null;
            return (
              <div key={g} className="settings-group">
                <SectionLabel>{g}</SectionLabel>
                {tabs.map((t) => (
                  <button key={t.id} type="button" className={`settings-link ${t.id === tab ? 'selected' : ''}`} onClick={() => ui.set({ settings: t.id })}>
                    <Icon name={t.icon} />{t.title}
                  </button>
                ))}
              </div>
            );
          })}
          {shown.length === 0 && <div className="small dim">Nothing matches.</div>}
        </nav>
        <div className="settings-scroll">
          <div className="settings-page" key={tab}>
            <ErrorBoundary what={current ? `The ${current.title} page` : 'This page'}>
            {tab === 'appearance' && <Appearance />}
            {tab === 'preferences' && <Preferences />}
            {tab === 'instructions' && <Instructions />}
            {tab === 'memory' && <Memory />}
            {tab === 'providers' && <Providers />}
            {tab === 'router' && <AgentRouterPage />}
            {tab === 'images' && <Images />}
            {tab === 'browser' && <BrowserSettings />}
            {tab === 'skills' && <Skills />}
            {tab === 'mcp' && <Mcp />}
            {tab === 'chatbots' && <Chatbots />}
            {tab === 'account' && <Account />}
            {tab === 'updates' && <Updates />}
            {tab === 'logs' && <Logs />}
            </ErrorBoundary>
          </div>
        </div>
      </div>
    </div>
  );
}
