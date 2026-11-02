import { useEffect, useState, type ReactNode } from 'react';
import { act, invoke, money, openExternal, parseUsd, shortDate, usd, type Fallbacks, type McpServer, type ModelRef, type UsageInfo } from './api';
import { FallbackList, ModelPicker, useModelName } from './ModelMenu';
import { AdditionalKey, BrowserEntry, CLAUDE_NOTE, CODEX_NOTE, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GUEST_NOTE, GrokEntry, GrokKeyEntry, HERMES_NOTE, HermesEntry, HermesKeyEntry, KIRO_NOTE, KeyField, NvidiaEntry, logo, OPENCODE_NOTE, OpencodeEntry, ScoutKeyEntry, SignInEntry } from './Accounts';
import { ui, useCore, useUi, useView, type SettingsTab } from './store';
import { CodeBlock, CopyButton, Icon, IconButton, Link, Modal, SectionLabel, Spinner, Toggle } from './widgets';

const TABS: { id: SettingsTab; title: string; icon: string; keywords: string }[] = [
  { id: 'instructions', title: 'Instructions', icon: 'chat', keywords: 'instructions project global rules preferences language russian english responses' },
  { id: 'memory', title: 'Memory', icon: 'file', keywords: 'memory remember project user profile facts context' },
  { id: 'notifications', title: 'Notifications', icon: 'bell', keywords: 'notifications alerts taskbar flash toast windows attention' },
  { id: 'discord', title: 'Discord', icon: 'chat', keywords: 'discord rich presence profile status privacy share activity' },
  { id: 'fallbacks', title: 'Fallbacks', icon: 'refresh', keywords: 'fallbacks fallback models provider fails failed error retry backup' },
  { id: 'usage', title: 'Usage', icon: 'chart', keywords: 'usage tokens cost spend daily monthly limits balance banked reset chart history' },
  { id: 'account', title: 'Account', icon: 'user', keywords: 'account scoutcode sign in sign out login key username billing' },
  { id: 'providers', title: 'Providers', icon: 'user', keywords: 'providers link unlink connect sign in sign out key claude code codex chatgpt anthropic openai copilot github openrouter nvidia opencode grok xai hermes nous devin kiro kilo models' },
  { id: 'skills', title: 'Skills', icon: 'file', keywords: 'skills upload directory skills.sh install instructions @ mention' },
  { id: 'mcp', title: 'MCP servers', icon: 'terminal', keywords: 'mcp servers custom tools connect add command url http stdio integrations' },
  { id: 'chatbots', title: 'Chatbots', icon: 'chat', keywords: 'chatbots mcp connector link claude chatgpt cursor tools server' },
  { id: 'updates', title: 'Updates', icon: 'download', keywords: 'updates version upgrade download restart release notes about' },
];

function Heading({ title, sub }: { title: string; sub: string }) {
  return <div className="settings-heading"><h2>{title}</h2><p className="small dim">{sub}</p></div>;
}

function SettingRow({ title, sub, children }: { title: string; sub?: string; children?: ReactNode }) {
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
      <SettingRow title="Response language" sub="Auto follows the conversation. Choose a language to keep replies and progress updates in that language.">
        <select className="input select" value={draft.language} onChange={(e) => setDraft({ ...draft, language: e.target.value })}>
          {languages.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
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
    </>
  );
}

function Notifications() {
  const cfg = useView('cfg');
  const flash = cfg.notifyFlash ?? true;
  const toast = cfg.notifyToast ?? false;
  return (
    <>
      <Heading title="Notifications" sub="When Scout finishes, or needs an answer, while its window is in the background." />
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
      <Heading title="Discord" sub="Shows on your Discord profile that you're using Scout. Off unless you turn it on." />
      <SettingRow title="Show Scout on Discord" sub="Your profile shows \“Using Scout\” while the app is open. Nothing else is sent."><Toggle label="Show Scout on Discord" on={presence} onChange={(on) => act('setDiscord', { presence: on, project: on && project })} /></SettingRow>
      <SettingRow title="Show the folder's name" sub="Adds \“Working on <folder>\”, using only the folder's name, never its path, chats or files. Needs the setting above."><Toggle label="Show the folder's name" on={project} onChange={(on) => act('setDiscord', { presence, project: on })} /></SettingRow>
    </>
  );
}

function SingleModelFallbacks({ m, list, onChange, onRemove }: { m: ModelRef; list: ModelRef[]; onChange: (l: ModelRef[]) => void; onRemove: () => void }) {
  const name = useModelName(m);
  return (
    <div className="stack">
      <div className="row"><b className="strong grow ellipsis">{name}</b><IconButton icon="close" tip={`Remove ${name}'s own list`} size={24} onClick={onRemove} /></div>
      <FallbackList value={list} onChange={onChange} />
      {list.length === 0 && <div className="small dim">No fallbacks: this model just shows its errors.</div>}
    </div>
  );
}

function FallbacksPage() {
  const { fallbacks: f } = useView('cfg');
  const save = (next: Partial<Fallbacks>) => act('setFallbacks', { fallbacks: { ...f, ...next } });
  const same = (a: ModelRef, b: ModelRef) => a.key === b.key && a.provider === b.provider;
  return (
    <>
      <Heading title="Fallbacks" sub="When a model fails, Scout carries the turn on with the next model in its list, then goes back to the model you chose for your next message." />
      <SettingRow title="Ask when a model fails" sub="The next time a model with no fallbacks fails, Scout offers to set some up. Off, it just shows the error.">
        <Toggle label="Ask when a model fails" on={!f.chosen} onChange={(on) => save({ chosen: !on })} />
      </SettingRow>
      <SectionLabel>Every model</SectionLabel>
      <p className="small dim">Tried for any model without a list of its own below.</p>
      <FallbackList value={f.all} onChange={(all) => save({ all })} />
      <SectionLabel>Single models</SectionLabel>
      <p className="small dim">A model here uses its own list instead.</p>
      {f.models.map(([m, list], i) => (
        <SingleModelFallbacks key={`${m.provider}:${m.key}`} m={m} list={list}
          onChange={(l) => save({ models: f.models.map((x, j) => (j === i ? [m, l] : x)) })}
          onRemove={() => save({ models: f.models.filter((_, j) => j !== i) })} />
      ))}
      <div><ModelPicker label="Add a model" onPick={(key, provider) => { const m = { key, provider }; if (!f.models.some(([x]) => same(x, m))) save({ models: [...f.models, [m, []]] }); }} /></div>
    </>
  );
}

// ---------- usage ----------

function Tile({ label, value, sub }: { label: string; value: string; sub: string }) {
  return <div className="tile"><div className="tiny dim">{label}</div><div className="tile-value">{value}</div><div className="tiny dim">{sub}</div></div>;
}

const shortTokens = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n));
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
          {m.username}   {usd(m.spentThisMonth)} spent, {usd(m.harnessSpentThisMonth)} desktop, {usd(m.balance)} balance, {usd(m.banked)} banked
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
        <input className="input field-180" placeholder={`Desktop credit (now ${usd(planCli)})`} value={cli} onChange={(e) => setCli(e.target.value)} />
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

function UsageLimits() {
  const { hasKey } = useView('cfg');
  const { info, busy, message } = useView('usage');
  if (!hasKey) return <p className="muted">Sign in to ScoutCode to see your monthly limits.</p>;
  const plan = (p: { limit: number; left: number } | null): [string, string] => (p ? [usd(p.left), `left of ${usd(p.limit)}`] : ['Unlimited', 'Requests are free']);
  return (
    <div className="stack">
      {info ? (
        <>
          <div className="tiles four">
            <Tile label="Desktop credit (browser sign-in)" value={plan(info.cli)[0]} sub={plan(info.cli)[1]} />
            <Tile label="Money (API keys)" value={plan(info.money)[0]} sub={plan(info.money)[1]} />
            <Tile label="Banked" value={usd(info.bankedMoney.reduce((n, b) => n + b.amount, 0))} sub="Spent next, runs out" />
            <Tile label="Balance" value={usd(info.balance)} sub="Spent last, doesn't reset" />
          </div>
          <div className="tiny dim">Resets {shortDate(info.resetsAt.slice(0, 10))} UTC</div>
          {info.bankedMoney.map((b, i) => <SettingRow key={i} title={`${usd(b.amount)} banked`} sub={`Expires ${shortDate(b.expiresAt.slice(0, 10))} UTC`} />)}
          {info.bankedResets.map((r, i) => (
            <SettingRow key={i} title="Banked reset" sub={`Clears your spending this month. Expires ${shortDate(r.expiresAt.slice(0, 10))} UTC.`}>
              {/* The one that runs out soonest goes first, so only the first row has the button. */}
              {i === 0 && <button type="button" className="button primary" disabled={busy} onClick={() => act('usageChange', { body: { action: 'use-banked-reset' }, said: 'Your usage this month is back to zero.' })}>Use</button>}
            </SettingRow>
          ))}
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
      <Heading title="Usage" sub="What you have left this month, and the tokens Scout used each day on this PC. Days and months are UTC." />
      <UsageLimits />
      {log && (
        <div className="stack usage-log">
          <div className="tiles three">
            {([['Today', log.today], ['Last 7 days', log.week], ['Last 30 days', log.month]] as const).map(([label, [t, c]]) => <Tile key={label} label={label} value={`${shortTokens(t)} tokens`} sub={`$${c.toFixed(2)}`} />)}
          </div>
          <div className="usage-chart">
            {log.days.map((d) => (
              <div key={d.day} className="usage-bar" title={`${shortDate(d.day)}\n${d.tokens.toLocaleString()} tokens\n$${d.cost.toFixed(2)}\n${d.requests} requests`}>
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

// ---------- account ----------

function Account() {
  const cfg = useView('cfg');
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
          <SettingRow title={cfg.browserSignIn ? 'Browser sign-in' : 'API key'} sub={cfg.browserSignIn ? 'Uses your monthly desktop credit, then any banked money and balance.' : "Pays from your account's money."} />
          <SettingRow title="Account page" sub="Keys, balance and billing on scoutcode.dev."><button type="button" className="button" onClick={account}>Open</button></SettingRow>
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
      <p className="small dim">Tried from the top. If one fails, the next one is used. Saved on this PC. Models on OpenRouter bill your OpenRouter account.</p>
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

type OwnWhich = 'copilot' | 'devin' | 'kiro' | 'codex' | 'claude';

function OwnAccount({ which, title, note, who, planText, planUrl }: { which: OwnWhich; title: string; note: string; who: string | null; planText: string; planUrl: string }) {
  const rowTitle = which === 'copilot' ? 'GitHub account' : `${title} account`;
  return (
    <>
      <p className="small dim">{note}</p>
      {who ? (
        <SettingRow title={rowTitle} sub={who}>
          <button type="button" className="button" onClick={() => act('signOutOf', { which, uninstall: false })}>Sign out</button>
          <button type="button" className="button" onClick={() => act('signOutOf', { which, uninstall: true })}>Sign out and uninstall</button>
        </SettingRow>
      ) : <div className="field-420"><SignInEntry which={which} /></div>}
      <Link href={planUrl} className="small">{planText}</Link>
    </>
  );
}

type ProviderEntry = { id: string; name: string; linked: boolean; status: string; body: ReactNode };

function ProviderRow({ p, open, onToggle }: { p: ProviderEntry; open: boolean; onToggle: () => void }) {
  return (
    <div className={`provider-row ${open ? 'open' : ''}`}>
      <button type="button" className="provider-head" aria-expanded={open} onClick={onToggle}>
        <img className="logo" src={logo(p.id)} width={22} height={22} alt="" />
        <div className="grow min0"><div className="setting-title">{p.name}</div><div className="small dim ellipsis">{p.status}</div></div>
        {p.linked && <span className="badge">Linked</span>}
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
  const { plans, kiloWarning, additionalProviders } = useCore((s) => s.statics);
  const [open, setOpen] = useState<string | null>(null);
  const own = (which: OwnWhich, name: string, note: string, on: boolean, planText: string, who = 'Signed in'): ProviderEntry => ({
    id: which, name, linked: on, status: login[which] ? 'Signing in…' : on ? who : 'Not linked',
    body: <OwnAccount which={which} title={name} note={note} who={on ? who : null} planText={planText} planUrl={plans[which]} />,
  });
  const keyRow = (title: string, masked: string, onRemove: () => void) => <SettingRow title={title} sub={masked}><button type="button" className="button" onClick={onRemove}>Unlink</button></SettingRow>;
  const all: ProviderEntry[] = [
    own('claude', 'Claude Code', CLAUDE_NOTE, cfg.claude, "See Claude's plans"),
    own('codex', 'Codex', CODEX_NOTE, cfg.codex, "See Codex in ChatGPT's plans"),
    own('copilot', 'GitHub Copilot', COPILOT_NOTE, cfg.copilot, 'No Copilot plan yet? Turn on the free one', cfg.copilotUser ?? 'Signed in'),
    {
      id: 'openrouter', name: 'OpenRouter', linked: cfg.openrouterKeys.length > 0,
      status: cfg.openrouterKeys.length ? `${cfg.openrouterKeys.length} key${cfg.openrouterKeys.length === 1 ? '' : 's'}` : 'Not linked',
      body: <OpenRouterKeys />,
    },
    {
      id: 'nvidia', name: 'NVIDIA NIM', linked: !!cfg.nvidiaKey, status: cfg.nvidiaKey ?? 'Not linked',
      body: (
        <>
          <p className="small dim">Saved on this PC. Models on NVIDIA NIM use your NVIDIA account.</p>
          {cfg.nvidiaKey ? keyRow('NVIDIA key', cfg.nvidiaKey, () => act('saveNvidiaKey', { key: null })) : <div className="field-420"><NvidiaEntry /></div>}
        </>
      ),
    },
    {
      id: 'opencode', name: 'OpenCode', linked: cfg.opencode,
      status: login.opencode ? 'Signing in…' : !cfg.opencode ? 'Not linked' : cfg.opencodeSignedIn ? 'Signed in' : 'Free models only',
      body: (
        <>
          <p className="small dim">{OPENCODE_NOTE}</p>
          {cfg.opencode ? (
            <>
              <SettingRow title="OpenCode account" sub={cfg.opencodeSignedIn ? 'Signed in' : 'Not signed in: free models only'}>
                {cfg.opencodeSignedIn && <button type="button" className="button" onClick={() => act('signOutOf', { which: 'opencode', uninstall: false })}>Sign out</button>}
                <button type="button" className="button" onClick={() => act('setOpencode', { on: false, uninstall: false })}>Turn off</button>
                <button type="button" className="button" onClick={() => act('setOpencode', { on: false, uninstall: true })}>Turn off and uninstall</button>
              </SettingRow>
              {!cfg.opencodeSignedIn && <div className="field-420"><SignInEntry which="opencode" /></div>}
            </>
          ) : <div className="field-420"><OpencodeEntry /></div>}
          <Link href={plans.opencode} className="small">Open the OpenCode console</Link>
        </>
      ),
    },
    {
      id: 'grok', name: 'Grok Build', linked: cfg.grokSignedIn || !!cfg.grokKey,
      status: login.grok ? 'Signing in…' : cfg.grokSignedIn ? 'Signed in' : cfg.grokKey ?? 'Not linked',
      body: (
        <>
          <p className="small dim">{GROK_NOTE}</p>
          {cfg.grokSignedIn || cfg.grokKey ? (
            <>
              <SettingRow title="Grok account" sub={cfg.grokSignedIn ? 'Signed in' : 'Not signed in'}>
                {cfg.grokSignedIn && <button type="button" className="button" onClick={() => act('signOutOf', { which: 'grok', uninstall: false })}>Sign out</button>}
                <button type="button" className="button" onClick={() => act('signOutOf', { which: 'grok', uninstall: true })}>Remove and uninstall</button>
              </SettingRow>
              {!cfg.grokSignedIn && <div className="field-420"><SignInEntry which="grok" /></div>}
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
      status: login.hermes ? 'Signing in…' : cfg.hermesSignedIn ? 'Signed in' : cfg.hermesKey ?? 'Not linked',
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
    own('devin', 'Devin', DEVIN_NOTE, cfg.devin, "See Devin's plans"),
    own('kiro', 'Kiro', KIRO_NOTE, cfg.kiro, "See Kiro's plans"),
    ...additionalProviders.map((a): ProviderEntry => {
      const l = cfg.additional.find((x) => x.id === a.id);
      return {
        id: a.id, name: a.name, linked: !!l, status: l ? l.masked ?? 'Free, no key' : 'Not linked',
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
  const rest = all.filter((p) => !p.linked);
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
    case 'connecting': return 'Connecting…';
    case 'ready': return `Connected · ${s.tools.length} tool${s.tools.length === 1 ? '' : 's'}`;
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
              {field("Headers, one per line. Signing in with OAuth isn't supported, so use a token.", 'headers', 'Authorization: Bearer <token>', 2)}
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
  const [mode, setMode] = useState('default');
  const canLink = cfg.hasKey && !!link.cwd;
  const status = link.status || (!cfg.hasKey ? 'This goes through your ScoutCode account, so sign in first.' : 'Off.');
  const address = `${cfg.baseUrl}/mcp`;
  const snippet = JSON.stringify({ mcpServers: { scout: { command: exe, args: ['mcp', '--cwd', link.cwd] } } }, null, 2);
  return (
    <>
      <Heading title="Chatbots" sub="Let a chatbot use Scout's tools on this computer over MCP: its model does the thinking, and it can reach the files and shell here, in the folder you choose. Nothing is spent on ScoutCode." />
      <SectionLabel>Web chatbots, like claude.ai and ChatGPT</SectionLabel>
      <SettingRow title="Folder" sub={link.cwd || 'Open a project first.'} />
      <SettingRow title="What it may do without asking" sub="The rest waits for your OK here. Turn the link off and on to change this.">
        <select className="input select" disabled={link.on} value={mode} onChange={(e) => setMode(e.target.value)}>
          {modes.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
        </select>
      </SettingRow>
      <SettingRow title="Link" sub={status}><button type="button" className="button" disabled={!link.on && !canLink} onClick={() => act('toggleLink', { mode })}>{link.on ? 'Turn off' : 'Turn on'}</button></SettingRow>
      {mode === 'yolo' && <p className="small red">Yolo lets the chatbot run any command on this computer without asking.</p>}
      <SettingRow title="Connector address" sub={address}><CopyButton text={address} /></SettingRow>
      <p className="small dim">Add this to the chatbot as a connector and sign in when it asks. Unlink a chatbot from your account page.</p>
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

// ---------- updates ----------

export function restartBlocker(running: boolean, dirty: boolean): string | null {
  if (running) return 'Scout is working. You can restart when it finishes.';
  if (dirty) return 'Save the file open in the editor first.';
  return null;
}

function Updates() {
  const { enabled, version, status } = useView('updater');
  const { autoUpdate } = useView('cfg');
  const { anyRunning } = useView('run');
  const dirty = useUi((u) => u.editorDirty);
  const title = `Scout ${version}`;
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

/** Settings: a large panel in the middle of the window, sections on the left and the open one on the right. */
export function Settings() {
  const tab = useUi((u) => u.settings);
  const [search, setSearch] = useState('');
  if (!tab) return null;
  const q = search.trim().toLowerCase().split(/\s+/).filter(Boolean);
  // Every word typed has to appear in the section's title or keywords, in any order.
  const shown = TABS.filter((t) => q.every((w) => `${t.title.toLowerCase()} ${t.keywords}`.includes(w)));
  const close = () => { ui.set({ settings: null }); setSearch(''); };
  return (
    <Modal onClose={close} className="settings">
      <div className="settings-title"><b className="brand grow">Settings</b><IconButton icon="close" tip="Close (Esc)" size={30} onClick={close} /></div>
      <div className="settings-body">
        <nav className="settings-nav">
          <div className="side-search"><Icon name="search" /><input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} /></div>
          <SectionLabel>Preferences</SectionLabel>
          {shown.map((t) => (
            <button key={t.id} type="button" className={`settings-link ${t.id === tab ? 'selected' : ''}`} onClick={() => ui.set({ settings: t.id })}>
              <Icon name={t.icon} />{t.title}
            </button>
          ))}
          {shown.length === 0 && <div className="small dim">Nothing matches.</div>}
        </nav>
        <div className="settings-page" key={tab}>
          {tab === 'instructions' && <Instructions />}
          {tab === 'memory' && <Memory />}
          {tab === 'notifications' && <Notifications />}
          {tab === 'discord' && <Discord />}
          {tab === 'fallbacks' && <FallbacksPage />}
          {tab === 'usage' && <Usage />}
          {tab === 'account' && <Account />}
          {tab === 'providers' && <Providers />}
          {tab === 'skills' && <Skills />}
          {tab === 'mcp' && <Mcp />}
          {tab === 'chatbots' && <Chatbots />}
          {tab === 'updates' && <Updates />}
        </div>
      </div>
    </Modal>
  );
}

