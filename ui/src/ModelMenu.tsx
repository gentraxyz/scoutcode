import { useEffect, useMemo, useRef, useState } from 'react';
import { act, tokens, type Model, type ModelRef } from './api';
import { AdditionalOptions, BrowserEntry, CLAUDE_NOTE, CODEX_NOTE, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GUEST_NOTE, GrokEntry, GuestEntry, HERMES_NOTE, HERMES_WARNING, HermesEntry, KIRO_NOTE, KeyField, logo, NvidiaEntry, OPENCODE_NOTE, OPENCODE_WARNING, OpencodeEntry, ScoutKeyEntry, SignInEntry } from './Accounts';
import { ChatbotsPanel } from './Chatbots';
import { core, useCore, useView } from './store';
import { Icon, IconButton, Link, Popover, Star } from './widgets';
import scoutLogo from '../../assets/icon.png';

type Tab = 'starred' | 'scoutcode' | 'claude' | 'codex' | 'openrouter' | 'nvidia' | 'opencode' | 'kilo' | 'copilot' | 'devin' | 'kiro' | 'grok' | 'hermes' | 'additional' | 'chatbots';
const TABS: [Tab, string][] = [['starred', 'Starred'], ['chatbots', 'MCP chatbots'], ['scoutcode', 'ScoutCode'], ['claude', 'Claude Code'], ['codex', 'Codex'], ['openrouter', 'OpenRouter'], ['nvidia', 'NVIDIA NIM'], ['opencode', 'OpenCode'], ['kilo', 'Kilo Code'], ['copilot', 'GitHub Copilot'], ['devin', 'Devin'], ['kiro', 'Kiro'], ['grok', 'Grok Build'], ['hermes', 'Hermes'], ['additional', 'Additional Providers']];

/** One row per model and place it runs. A model both serve is listed under each, under the id that provider knows it by. */
type ModelRow = { key: string; provider: string; name: string; star: string; model: Model };

function allRows(models: Model[]): ModelRow[] {
  const rows: ModelRow[] = [];
  const add = (provider: (m: Model) => string | null, star: (m: Model) => string | null) => {
    for (const m of models) {
      const p = provider(m);
      const s = star(m);
      if (p && s) rows.push({ key: m.id, provider: p, name: m.label, star: s, model: m });
    }
  };
  add((m) => (m.hosted ? 'scoutcode' : null), (m) => m.id);
  add((m) => (m.claudeId ? 'claude' : null), (m) => m.id);
  add((m) => (m.codexId ? 'codex' : null), (m) => m.id);
  add((m) => (m.openrouterId ? 'openrouter' : null), (m) => m.openrouterId);
  add((m) => (m.nvidiaId ? 'nvidia' : null), (m) => m.nvidiaId);
  add((m) => (m.opencodeId ? 'opencode' : null), (m) => m.id);
  add((m) => (m.kiloId ? 'kilo' : null), (m) => m.kiloId);
  add((m) => (m.copilotId ? 'copilot' : null), (m) => m.id);
  add((m) => (m.devinId ? 'devin' : null), (m) => m.id);
  add((m) => (m.kiroId ? 'kiro' : null), (m) => m.id);
  add((m) => (m.grokId ? 'grok' : null), (m) => m.id);
  add((m) => (m.hermesId ? 'hermes' : null), (m) => m.id);
  add((m) => m.additional?.[0] ?? null, (m) => m.id);
  return rows;
}

/**
 * Whether a row's own provider charges nothing. A model ScoutCode serves can be free there and paid on OpenRouter, or
 * the other way round, so OpenRouter's row only looks at OpenRouter. NIM's list has no prices.
 */
function rowFree(r: ModelRow): boolean {
  const m = r.model;
  if (r.provider === 'kilo') return m.kiloFree;
  if (r.provider === 'nvidia') return false;
  if (r.provider === 'openrouter') return r.star.endsWith(':free') || (m.hosted ? m.openrouterFree : m.free);
  return m.free;
}

/** Rough USD for a typical coding request (10k tokens in, 1k out), the scale the price tiers use, so lists can go cheapest first. */
const TIER_USD = [0, 0.005, 0.03, 0.1, 0.5];

export function price(r: ModelRow): number {
  const m = r.model;
  if (r.key === 'auto') return -1;
  // NIM's list has no prices, like the row shows.
  if (r.provider === 'nvidia') return Infinity;
  if (r.provider === 'kilo') return m.kiloFree ? 0 : m.kiloTier ? TIER_USD[m.kiloTier] : Infinity;
  // A ScoutCode model's own price says nothing about what OpenRouter charges for it: only its OpenRouter tier does.
  const viaOpenrouter = r.provider === 'openrouter' && m.hosted;
  if (!viaOpenrouter) {
    const cost = m.cost ?? '';
    if (m.free || /^free$/i.test(cost)) return 0;
    const perRequest = /\$([\d.]+) \/ request/.exec(cost);
    const tokens = /\$([\d.]+) in · \$([\d.]+) out/.exec(cost);
    if (perRequest || tokens) return (perRequest ? Number(perRequest[1]) : 0) + (tokens ? (Number(tokens[1]) * 10_000 + Number(tokens[2]) * 1_000) / 1e6 : 0);
  } else if (rowFree(r)) return 0;
  const tier = viaOpenrouter ? m.openrouterTier : m.tier;
  // Unknown prices go last.
  return tier ? TIER_USD[tier] : Infinity;
}

/**
 * The Intelligence Index: how capable a model is for coding, from 1 (light) to 3 (frontier). An admin can set it on the
 * server (Desktop models page); otherwise it is read from the name so it
 * works for every provider's list. Small and cheap variants ("mini", "flash-lite", 8B…) are light; the newest flagship
 * families are frontier; the rest sit in between. Auto picks a model per request, so it has none.
 */
export function intelligence(m: Model): 1 | 2 | 3 | null {
  if (m.id === 'auto') return null;
  // An admin's setting on the server wins over the guess.
  if (m.intelligence === 1 || m.intelligence === 2 || m.intelligence === 3) return m.intelligence;
  const n = `${m.id} ${m.label}`.toLowerCase().replace(/[_\s]+/g, '-');
  if (/(^|[-/:])(mini|nano|lite|tiny|small)(?![a-z])|flash-lite|haiku-?3|gemma|phi-?\d|ministral|(^|[^\d.])(0\.\d|1|1\.5|2|3|4|7|8|9|12|14)b\b|\be[24]b\b/.test(n)) return 1;
  if (/opus|fable|sonnet-?(4[.-]5|4[.-]6|5)|gpt-?5|\bo3\b|\bo4\b|gemini-?(2\.5|3)[\w.-]*pro|grok-?[45](?![\w.-]*fast)|deepseek[\w.-]*(r1|v3\.[12]|v4)|kimi-?k2|qwen-?3[\w.-]*(max|coder|235b|480b)|glm-?(4\.[5-9]|5)|minimax-?m2|gpt-?oss-?120b|codex/.test(n)) return 3;
  return 2;
}

const INTEL_NAMES = ['', 'Light', 'Capable', 'Frontier'];

/** Three bars, filled up to the model's Intelligence Index: more bars, smarter model. */
export function IntelBars({ level }: { level: 1 | 2 | 3 | null }) {
  if (!level) return null;
  const tip = `Intelligence Index: ${INTEL_NAMES[level]} (${level} of 3)`;
  return (
    <span className="intel" title={tip} aria-label={tip} role="img">
      {[1, 2, 3].map((i) => <span key={i} className={`intel-bar ${i <= level ? 'on' : ''}`} style={{ height: 4 + i * 3 }} />)}
    </span>
  );
}

/** How the model list is ordered, picked in the menu and kept for next time. Starred models always come first. */
const SORTS = [
  ['cheap', 'Cheapest first'],
  ['pricey', 'Most expensive first'],
  ['name', 'Name (A–Z)'],
  ['smart', 'Smartest first'],
  ['context', 'Largest context'],
] as const;
type Sort = (typeof SORTS)[number][0];
const SORT_KEY = 'scout.modelSort';
const FREE_KEY = 'scout.modelFreeOnly';
/** Providers hidden from the rail with a right-click, kept on this PC. */
const HIDDEN_KEY = 'scout.hiddenProviders';
/** Set once the menu has shown its right-click tip, the first time it opened. */
const TIP_KEY = 'scout.modelMenuTipSeen';

function savedHidden(): Tab[] {
  try {
    const v = JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((t): t is Tab => TABS.some(([id]) => id === t)) : [];
  } catch {
    return [];
  }
}

function savedSort(): Sort {
  const s = localStorage.getItem(SORT_KEY);
  return SORTS.some(([id]) => id === s) ? (s as Sort) : 'cheap';
}

/** Orders two rows by `sort`. Models with no known price or context stay at the end either way. */
function compare(sort: Sort, a: ModelRow, b: ModelRow): number {
  const last = (x: number, y: number, dir: 1 | -1) => (x === y ? 0 : !Number.isFinite(x) ? 1 : !Number.isFinite(y) ? -1 : (x - y) * dir);
  switch (sort) {
    case 'pricey': return a.key === 'auto' ? -1 : b.key === 'auto' ? 1 : last(price(a), price(b), -1);
    case 'name': return a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true });
    case 'context': return last(a.model.context ?? Infinity, b.model.context ?? Infinity, -1);
    case 'smart': return a.key === 'auto' ? -1 : b.key === 'auto' ? 1 : (intelligence(b.model) ?? 0) - (intelligence(a.model) ?? 0) || last(price(a), price(b), 1);
    default: return last(price(a), price(b), 1);
  }
}

function runsOn(id: string): string {
  const a = core.get().statics.additionalProviders.find((p) => p.id === id);
  return a?.name ?? TABS.find(([t]) => t === id)?.[1] ?? id;
}

function RailIcon({ tab }: { tab: Tab }) {
  if (tab === 'starred') return <Star on={false} />;
  if (tab === 'additional') return <Icon name="plus" />;
  if (tab === 'chatbots') return <Icon name="chat" />;
  const file = { scoutcode: '', claude: 'claude', codex: 'codex', openrouter: 'openrouter', nvidia: 'nvidia', opencode: 'opencode', kilo: 'kilo', copilot: 'copilot', devin: 'devin', kiro: 'kiro', grok: 'grok', hermes: 'hermes' }[tab];
  if (!file) return <img className="logo" src={scoutLogo} width={16} height={16} alt="" />;
  return <img className="logo" src={logo(file)} width={16} height={16} alt="" />;
}

function ModelCard({ m, top, left }: { m: Model; top: number; left: number }) {
  const caps = [m.tools && 'Tools', m.vision && 'Vision'].filter(Boolean).join(' · ');
  const on = [m.hosted && 'ScoutCode', m.openrouterId && 'OpenRouter', m.nvidiaId && 'NVIDIA NIM', m.opencodeId && 'OpenCode', m.kiloId && 'Kilo Code', m.hermesId && 'Hermes', m.additional && runsOn(m.additional[0])].filter(Boolean).join(' or ');
  const stats: [string, string | null | undefined][] = [
    ['Smarts', intelligence(m) ? `${INTEL_NAMES[intelligence(m)!]} (${intelligence(m)} of 3)` : null],
    ['Context', m.context ? `${tokens(m.context)} tokens` : null], ['Cost', m.cost], ['Supports', caps], ['Runs on', on],
    ['Providers', m.providers.join(', ')], ['Tries', m.routesTo.join(' → ')],
  ];
  return (
    <div className="model-card" style={{ top, left }}>
      <div className="strong bold">{m.label}</div>
      <div className="mono small dim">{m.id}</div>
      {m.description && <div className="small muted">{m.description}</div>}
      {stats.some(([, v]) => v) && <hr />}
      {stats.filter(([, v]) => v).map(([k, v]) => <div key={k} className="stat-row"><span className="small dim">{k}</span><span className="small">{v}</span></div>)}
      {m.available === false && <div className="small red">Unavailable right now</div>}
    </div>
  );
}

function SignedOut({ tab }: { tab: Tab }) {
  if (tab === 'additional') return <AdditionalOptions />;
  const name = TABS.find(([t]) => t === tab)![1];
  return (
    <div className="stack">
      <b className="strong">{tab === 'opencode' ? 'Set up OpenCode' : `You must sign in to use ${name}`}</b>
      {tab === 'scoutcode' && (
        <>
          <div className="small dim">No key needed: uses your account's monthly desktop credit.</div>
          <BrowserEntry />
          <GuestEntry />
          <div className="small dim">Or use an API key:</div>
          <ScoutKeyEntry />
        </>
      )}
      {tab === 'openrouter' && (
        <>
          <div className="small dim">Saved on this PC. Models on OpenRouter bill your OpenRouter account.</div>
          <Link href="https://openrouter.ai/settings/keys" className="small">Make a key on openrouter.ai</Link>
          <KeyField hint="sk-or-…" button="Save OpenRouter key" onSave={(key) => act('addOpenrouterKey', { name: 'OpenRouter', key })} />
        </>
      )}
      {tab === 'nvidia' && (
        <>
          <div className="small dim">Saved on this PC. Models on NVIDIA NIM use your NVIDIA account.</div>
          <NvidiaEntry />
        </>
      )}
      {tab === 'opencode' && <><div className="small dim">{OPENCODE_NOTE}</div><OpencodeEntry /></>}
      {tab === 'copilot' && <><div className="small dim">{COPILOT_NOTE}</div><SignInEntry which="copilot" /></>}
      {tab === 'devin' && <><div className="small dim">{DEVIN_NOTE}</div><SignInEntry which="devin" /></>}
      {tab === 'kiro' && <><div className="small dim">{KIRO_NOTE}</div><SignInEntry which="kiro" /></>}
      {tab === 'claude' && <><div className="small dim">{CLAUDE_NOTE}</div><SignInEntry which="claude" /></>}
      {tab === 'codex' && <><div className="small dim">{CODEX_NOTE}</div><SignInEntry which="codex" /></>}
      {tab === 'grok' && <><div className="small dim">{GROK_NOTE}</div><GrokEntry /></>}
      {tab === 'hermes' && <><div className="small dim">{HERMES_NOTE}</div><HermesEntry /></>}
    </div>
  );
}

function Menu({ close, startTab, onPick }: { close: () => void; startTab: Tab; onPick?: (key: string, provider: string) => void }) {
  const models = useCore((s) => s.models);
  const cfg = useView('cfg');
  const starred = useView('starred');
  const current = useView('model');
  const loading = useView('modelsLoading');
  const additionalErrors = useView('additionalErrors');
  const kiroError = useView('kiroModelsError');
  const codexError = useView('codexModelsError');
  const claudeError = useView('claudeModelsError');
  const opencodeError = useView('opencodeModelsError');
  const grokError = useView('grokModelsError');
  const hermesError = useView('hermesModelsError');
  const link = useView('link');
  const { kiloWarning, additionalProviders } = useCore((s) => s.statics);
  const [tab, setTab] = useState<Tab>(startTab);
  const [search, setSearch] = useState('');
  const [sort, setSortState] = useState<Sort>(savedSort);
  const [freeOnly, setFreeOnlyState] = useState(() => localStorage.getItem(FREE_KEY) === '1');
  const setSort = (s: Sort) => { setSortState(s); localStorage.setItem(SORT_KEY, s); };
  const setFreeOnly = (on: boolean) => { setFreeOnlyState(on); localStorage.setItem(FREE_KEY, on ? '1' : '0'); };
  const [adding, setAdding] = useState(false);
  const [hidden, setHiddenState] = useState<Tab[]>(savedHidden);
  const setHidden = (h: Tab[]) => { setHiddenState(h); localStorage.setItem(HIDDEN_KEY, JSON.stringify(h)); };
  // A provider's right-click menu, or with tab 'starred' the list of hidden ones; placed inside the menu.
  const [ctx, setCtx] = useState<{ tab: Tab; x: number; y: number } | null>(null);
  const [tip, setTip] = useState(() => localStorage.getItem(TIP_KEY) !== '1');
  const dismissTip = () => setTip(false);
  // Shown the first time the menu opens, and not again.
  useEffect(() => localStorage.setItem(TIP_KEY, '1'), []);
  const menuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!ctx) return;
    const down = (e: MouseEvent) => { if (!(e.target as HTMLElement).closest('.rail-menu')) setCtx(null); };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setCtx(null); } };
    document.addEventListener('mousedown', down);
    window.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', down); window.removeEventListener('keydown', key, true); };
  }, [ctx]);
  const [hover, setHover] = useState<{ m: Model; top: number; left: number } | null>(null);
  // A team's member sees what the team runs, and the providers on this PC it allows.
  const team = cfg.team;
  const teamRuns = (t: Tab) => !!team && (team.providers.includes(t) || (t === 'additional' && additionalProviders.some((a) => team.providers.includes(a.id))));
  const shownTab = (t: Tab) => !team || t === 'starred' || t === 'chatbots' || teamRuns(t) || team.acp.includes(t);
  const signedIn = (t: Tab) => teamRuns(t) || ({ scoutcode: cfg.hasKey || cfg.guest, openrouter: cfg.openrouterKeys.length > 0, nvidia: cfg.nvidiaKey !== null, opencode: cfg.opencode, copilot: cfg.copilot, devin: cfg.devin, kiro: cfg.kiro, codex: cfg.codex, claude: cfg.claude, grok: cfg.grokSignedIn || cfg.grokKey !== null, hermes: cfg.hermesSignedIn || cfg.hermesKey !== null, additional: cfg.additional.length > 0, starred: true, kilo: true, chatbots: link.on })[t];
  const q = search.trim().toLowerCase();
  const rows = useMemo(() => {
    const all = allRows(models);
    const pos = (r: ModelRow) => { const i = starred.indexOf(r.star); return i < 0 ? Infinity : i; };
    let list: ModelRow[];
    const hiddenRow = (r: ModelRow) => hidden.includes(r.provider as Tab) || (hidden.includes('additional') && additionalProviders.some((a) => a.id === r.provider));
    if (q) list = all.filter((r) => !hiddenRow(r) && [r.key, r.name, r.star].some((s) => s.toLowerCase().includes(q)));
    else if (tab === 'starred') list = all.filter((r) => starred.includes(r.star));
    else if (tab === 'additional') list = all.filter((r) => additionalProviders.some((a) => a.id === r.provider));
    else list = all.filter((r) => r.provider === tab && (tab !== 'scoutcode' || signedIn('scoutcode') || r.model.free));
    if (freeOnly) list = list.filter((r) => r.key === 'auto' || rowFree(r));
    // Starred first, in the order they were starred; then the sort picked. Ties keep their order.
    return [...list].sort((a, b) => (pos(a) !== pos(b) ? pos(a) - pos(b) : compare(sort, a, b)));
  }, [models, q, tab, starred, additionalProviders, sort, freeOnly, hidden]);
  // ScoutCode always lists its free models; signing in only adds the rest and a banner suggests it.
  const signIn = !q && tab !== 'scoutcode' && (!signedIn(tab) || (tab === 'additional' && adding));
  // Chatbots bring their own model, so the tab shows the link instead of a list.
  const chatbots = !q && tab === 'chatbots';
  const empty = loading > 0 ? 'Loading models…' : q ? 'No models match.' : tab === 'starred' ? 'Star a model to keep it here.' : 'No models here yet.';
  return (
    <div className="model-menu" ref={menuRef}>
      <div className="rail">
        {TABS.filter(([t]) => shownTab(t) && !hidden.includes(t)).map(([t, name]) => (
          <button
            key={t}
            type="button"
            title={signedIn(t) ? name : `${name} (not connected)`}
            aria-label={signedIn(t) ? name : `${name} (not connected)`}
            className={`rail-tab ${tab === t && !q ? 'on' : ''} ${signedIn(t) ? '' : 'off'}`}
            onClick={() => { setTab(t); setSearch(''); setAdding(false); }}
            onContextMenu={(e) => {
              e.preventDefault();
              if (t === 'starred') return;
              const box = menuRef.current!.getBoundingClientRect();
              setCtx({ tab: t, x: e.clientX - box.left, y: e.clientY - box.top });
              if (tip) dismissTip();
            }}
          >
            <RailIcon tab={t} />
          </button>
        ))}
        {hidden.length > 0 && (
          <button type="button" className="rail-tab" title={`Show hidden providers (${hidden.length})`} aria-label={`Show hidden providers (${hidden.length})`} onClick={(e) => {
            const box = menuRef.current!.getBoundingClientRect();
            const b = e.currentTarget.getBoundingClientRect();
            setCtx({ tab: 'starred', x: b.right - box.left + 4, y: b.top - box.top });
          }}>
            <Icon name="more" />
          </button>
        )}
      </div>
      {ctx && (
        <div className="rail-menu" style={{ left: ctx.x, top: Math.max(0, Math.min(ctx.y, 470 - 48 - (ctx.tab === 'starred' ? hidden.length : 2) * 32)) }}>
          {ctx.tab === 'starred' ? hidden.map((t) => (
            <button key={t} type="button" className="menu-item" onClick={() => { setHidden(hidden.filter((h) => h !== t)); setCtx(null); }}>
              <RailIcon tab={t} /><span className="grow ellipsis">Show {TABS.find(([id]) => id === t)![1]}</span>
            </button>
          )) : (() => {
            const t = ctx.tab;
            const name = TABS.find(([id]) => id === t)![1];
            return (
              <>
                <div className="tiny dim rail-menu-head">{name}</div>
                {!signedIn(t) && (
                  <button type="button" className="menu-item" onClick={() => { setTab(t); setSearch(''); setAdding(false); setCtx(null); }}>
                    <Icon name="user" /><span className="grow">{t === 'opencode' ? 'Set up OpenCode' : t === 'additional' ? 'Add a provider' : t === 'chatbots' ? 'Connect a chatbot' : `Sign in to ${name}`}</span>
                  </button>
                )}
                <button type="button" className="menu-item" onClick={() => {
                  setHidden([...hidden, t]);
                  if (tab === t) setTab('starred');
                  setCtx(null);
                }}>
                  <Icon name="close" /><span className="grow">Hide</span>
                </button>
              </>
            );
          })()}
        </div>
      )}
      <div className="model-column">
        <input className="input" autoFocus placeholder="Search models" value={search} onChange={(e) => setSearch(e.target.value)} />
        {tip && (
          <div className="card-note menu-tip">
            <div className="row">
              <b className="strong grow small">Tip</b>
              <IconButton icon="close" tip="Got it" size={22} onClick={dismissTip} />
            </div>
            <div className="small dim">Right-click a provider on the left to hide it, or to sign in if it needs an account. The bars by each model are its Intelligence Index: the more bars, the smarter.</div>
          </div>
        )}
        {!chatbots && !signIn && (
          <div className="model-sort">
            <select className="input select compact" aria-label="Sort models" title="Sort models" value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
              {SORTS.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
            </select>
            <button type="button" className={`chip ${freeOnly ? 'on' : ''}`} aria-pressed={freeOnly} title="Show only models that cost nothing here" onClick={() => setFreeOnly(!freeOnly)}>Free only</button>
          </div>
        )}
        {tab === 'kilo' && <div className="tiny dim">{kiloWarning}</div>}
        {tab === 'scoutcode' && !q && !signedIn('scoutcode') && (
          <div className="card-note">
            <div className="small"><b className="strong">Sign in for more free requests.</b> <span className="dim">Your account's monthly desktop credit pays for the rest.</span></div>
            <BrowserEntry />
          </div>
        )}
        {tab === 'scoutcode' && cfg.guest && <div className="tiny dim">{GUEST_NOTE} Sign in from Settings &gt; Account for more.</div>}
        {tab === 'kiro' && cfg.kiro && kiroError && <div className="tiny red">{kiroError}</div>}
        {tab === 'codex' && cfg.codex && codexError && <div className="tiny red">{codexError}</div>}
        {tab === 'claude' && cfg.claude && claudeError && <div className="tiny red">{claudeError}</div>}
        {tab === 'grok' && signedIn('grok') && grokError && <div className="tiny red">{grokError}</div>}
        {tab === 'hermes' && signedIn('hermes') && (
          <>
            <div className="tiny dim">{HERMES_WARNING}</div>
            {hermesError && <div className="tiny red">{hermesError}</div>}
          </>
        )}
        {tab === 'opencode' && cfg.opencode && (
          <>
            <div className="tiny dim">{OPENCODE_WARNING}</div>
            {opencodeError && <div className="tiny red">{opencodeError}</div>}
            {!cfg.opencodeSignedIn && <SignInEntry which="opencode" />}
          </>
        )}
        {tab === 'additional' && !q && (
          <>
            <div className="row">
              <b className="strong grow">Additional Providers</b>
              {!team && cfg.additional.length > 0 && (adding
                ? <button type="button" className="button" onClick={() => setAdding(false)}>Back</button>
                : cfg.additional.length < additionalProviders.length && <button type="button" className="button" title="Add another provider" onClick={() => setAdding(true)}>+</button>)}
            </div>
            {additionalErrors.map((e) => <div key={e} className="tiny red">{e}</div>)}
          </>
        )}
        <div className="model-list" onMouseLeave={() => setHover(null)}>
          {chatbots ? <ChatbotsPanel close={close} /> : signIn ? <SignedOut tab={tab} /> : rows.map((r) => {
            const m = r.model;
            // NIM's list has no prices, and what another provider charges says nothing about it.
            const free = rowFree(r);
            const tier = r.provider === 'nvidia' ? null : r.provider === 'kilo' ? m.kiloTier : r.provider === 'openrouter' && m.hosted ? m.openrouterTier : m.tier;
            const selected = current.key === r.key && current.provider === r.provider;
            const on = starred.includes(r.star);
            return (
              <div
                key={`${r.provider}:${r.key}`}
                className={`model-row ${selected ? 'selected' : ''}`}
                onMouseEnter={(e) => {
                  const row = e.currentTarget.getBoundingClientRect();
                  const menu = e.currentTarget.closest('.popover')!.getBoundingClientRect();
                  const left = menu.right + 8 + 284 <= window.innerWidth ? menu.right + 8 : menu.left - 8 - 284;
                  setHover({ m, top: Math.max(8, Math.min(row.top, window.innerHeight - 240)), left });
                }}
                onClick={() => { if (onPick) onPick(r.key, r.provider); else act('chooseModel', { key: r.key, provider: r.provider }); close(); }}
              >
                <button type="button" className={`star ${on ? 'on' : ''}`} title={on ? 'Unstar' : 'Star'} onClick={(e) => { e.stopPropagation(); act('toggleStar', { id: r.star }); }}><Star on={on} /></button>
                <span className="grow ellipsis">{r.name}</span>
                <IntelBars level={intelligence(m)} />
                {free ? <span className="badge">Free</span> : tier ? <span className="tiny dim">{'$'.repeat(tier)}</span> : null}
                {logo(r.provider) && <img className="logo row-logo" src={logo(r.provider)} width={13} height={13} alt="" />}
              </div>
            );
          })}
          {!signIn && !chatbots && rows.length === 0 && <div className="small dim pad">{empty}</div>}
        </div>
      </div>
      {hover && <ModelCard {...hover} />}
    </div>
  );
}

/** A button that opens the model menu and hands back the pick, without choosing it itself. */
export function ModelPicker({ label, onPick }: { label: string; onPick: (key: string, provider: string) => void }) {
  return (
    <Popover side="top" className="model-popover" trigger={(_, toggle) => <button type="button" className="button" onClick={toggle}>{label}</button>}>
      {(close) => <Menu close={close} startTab="scoutcode" onPick={onPick} />}
    </Popover>
  );
}

/** The model, as a menu with search and where it runs. */
export function ModelChip() {
  const current = useView('model');
  const models = useCore((s) => s.models);
  const { additionalProviders } = useCore((s) => s.statics);
  const label = models.find((m) => m.id === current.key)?.label ?? current.short;
  const shown = label.length > 26 ? `${label.slice(0, 25)}…` : label;
  const onAdditional = additionalProviders.some((a) => a.id === current.provider);
  const hint = current.provider === 'scoutcode' ? 'Model' : `Model · running on ${runsOn(current.provider)}`;
  const start: Tab = onAdditional ? 'additional' : TABS.some(([t]) => t === current.provider) ? (current.provider as Tab) : 'scoutcode';
  const [key, setKey] = useState(0);
  return (
    <Popover align="end" side="top" className="model-popover" onOpen={() => setKey(key + 1)} trigger={(open, toggle) => (
      <button type="button" className={`chip ${open ? 'open' : ''}`} title={hint} onClick={toggle}>{shown}<Icon name="chevronDown" size={12} /></button>
    )}>
      {(close) => <Menu key={key} close={close} startTab={start} />}
    </Popover>
  );
}


/** A model's name and, off ScoutCode, where it runs. */
export function useModelName(m: ModelRef): string {
  const models = useCore((s) => s.models);
  const label = models.find((x) => x.id === m.key)?.label ?? m.key;
  return m.provider === 'scoutcode' ? label : `${label} · ${runsOn(m.provider)}`;
}

function FallbackRow({ m, n, onRemove }: { m: ModelRef; n: number; onRemove: () => void }) {
  const name = useModelName(m);
  return (
    <div className="fallback-row">
      <span className="dim small">{n}.</span>
      {logo(m.provider) && <img className="logo row-logo" src={logo(m.provider)} width={13} height={13} alt="" />}
      <span className="grow ellipsis">{name}</span>
      <IconButton icon="close" tip="Remove" size={22} onClick={onRemove} />
    </div>
  );
}

/** Fallback models in the order they're tried, with a picker to add up to `max`. */
export function FallbackList({ value, onChange, max = 3 }: { value: ModelRef[]; onChange: (v: ModelRef[]) => void; max?: number }) {
  const add = (key: string, provider: string) => {
    if (!value.some((m) => m.key === key && m.provider === provider)) onChange([...value, { key, provider }]);
  };
  return (
    <div className="fallback-list">
      {value.map((m, i) => <FallbackRow key={`${m.provider}:${m.key}`} m={m} n={i + 1} onRemove={() => onChange(value.filter((_, j) => j !== i))} />)}
      {value.length < max && <div><ModelPicker label={value.length ? 'Add another' : 'Add a fallback model'} onPick={add} /></div>}
    </div>
  );
}
