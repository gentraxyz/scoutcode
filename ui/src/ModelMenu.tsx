import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { act, tokens, type AgentRouteStep, type AgentRouter, type Model, type ModelRef } from './api';
import { AdditionalOptions, BrowserEntry, COPILOT_NOTE, DEVIN_NOTE, GUEST_NOTE, GuestEntry, HERMES_NOTE, HERMES_WARNING, HermesEntry, KIRO_NOTE, KeyField, logo, NvidiaEntry, OPENCODE_NOTE, OPENCODE_WARNING, OpencodeEntry, providerStatus, ScoutKeyEntry, SignInEntry } from './Accounts';
import { chatbotConnected, ChatbotPage, CHATBOT_ONLY_PAGES, isBotPage } from './Chatbots';
import { DeepSeekWebPanel, useDeepSeek } from './DeepSeek';
import { core, useCore, useView } from './store';
import { Icon, IconButton, Link, Popover, Star, Switch } from './widgets';
import scoutLogo from '../../assets/icon.png';

type Tab = 'starred' | 'router' | 'scoutcode' | 'claude' | 'codex' | 'gemini' | 'openrouter' | 'nvidia' | 'opencode' | 'kilo' | 'copilot' | 'devin' | 'kiro' | 'grok' | 'hermes' | 'deepseek-web' | 'additional' | 'chatbots';
// ChatGPT and Claude are Codex and Claude Code too: their pages hold both the chatbot and its models.
const TABS: [Tab, string][] = [['starred', 'Starred'], ['router', 'Agent Router'], ['codex', 'ChatGPT'], ['claude', 'Claude'], ['gemini', 'Gemini'], ['grok', 'Grok'], ['chatbots', 'Other chatbots'], ['scoutcode', 'ScoutCode'], ['openrouter', 'OpenRouter'], ['nvidia', 'NVIDIA NIM'], ['opencode', 'OpenCode'], ['kilo', 'Kilo Code'], ['copilot', 'GitHub Copilot'], ['devin', 'Devin'], ['kiro', 'Kiro'],['hermes', 'Hermes'], ['deepseek-web', 'DeepSeek Web'], ['additional', 'Additional Providers']];

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
  add((m) => (m.id.startsWith('deepseek-web/') ? 'deepseek-web' : null), (m) => m.id);
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
  if (tab === 'router') return <Icon name="route" />;
  if (tab === 'additional') return <Icon name="plus" />;
  if (tab === 'chatbots') return <Icon name="chat" />;
  const file = { scoutcode: 'scoutcode', claude: 'claude', codex: 'codex', gemini: 'gemini', openrouter: 'openrouter', nvidia: 'nvidia', opencode: 'opencode', kilo: 'kilo', copilot: 'copilot', devin: 'devin', kiro: 'kiro', grok: 'grok', hermes: 'hermes', 'deepseek-web': 'deepseek-web' }[tab];
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
  const status = providerStatus(tab);
  return (
    <div className="stack">
      <div className="row">
        <b className="strong grow">{tab === 'opencode' ? 'Set up OpenCode' : `You must sign in to use ${name}`}</b>
        <span className={`badge ${status === 'Paid' ? 'paid' : ''}`}>{status}</span>
      </div>
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
      {tab === 'hermes' && <><div className="small dim">{HERMES_NOTE}</div><HermesEntry /></>}
      {tab === 'deepseek-web' && <DeepSeekWebPanel />}
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
  const deepseek = useDeepSeek();
  const teamRuns = (t: Tab) => !!team && (team.providers.includes(t) || (t === 'additional' && additionalProviders.some((a) => team.providers.includes(a.id))));
  const shownTab = (t: Tab) => (!onPick || (t !== 'router' && !CHATBOT_ONLY_PAGES.includes(t))) && (!team || t === 'starred' || t === 'router' || CHATBOT_ONLY_PAGES.includes(t) || teamRuns(t) || team.acp.includes(t));
  const signedIn = (t: Tab) => teamRuns(t) || ({ scoutcode: cfg.hasKey || cfg.guest, openrouter: cfg.openrouterKeys.length > 0, nvidia: cfg.nvidiaKey !== null, opencode: cfg.opencode, copilot: cfg.copilot, devin: cfg.devin, kiro: cfg.kiro, codex: cfg.codex, claude: cfg.claude, grok: cfg.grokSignedIn || cfg.grokKey !== null, hermes: cfg.hermesSignedIn || cfg.hermesKey !== null, 'deepseek-web': !!deepseek?.enabled, additional: cfg.additional.length > 0, starred: true, router: true, kilo: true, chatbots: link.on, gemini: chatbotConnected('gemini', link.chatbots) })[t];
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
  // A chatbot's page shows its own sign-in (ChatbotPage), so it never gets the plain sign-in screen.
  const signIn = !q && !isBotPage(tab) && tab !== 'scoutcode' && (!signedIn(tab) || (tab === 'additional' && adding));
  // A chatbot's page holds its link; ChatGPT and Claude also list their models under it. Chatbots bring their own model.
  const botPage = !q && isBotPage(tab);
  const chatbotOnly = !q && CHATBOT_ONLY_PAGES.includes(tab);
  const routerTab = !q && tab === 'router';
  // Whether the model list shows: not for the sign-in, the router, or a chatbot-only page.
  const listed = !routerTab && !signIn && !chatbotOnly && !(botPage && !signedIn(tab));
  const empty = loading > 0 ? 'Loading models…' : q ? 'No models match.' : tab === 'starred' ? 'Star a model to keep it here.' : 'No models here yet.';
  return (
    <div className="model-menu" ref={menuRef}>
      <div className="rail">
        {TABS.filter(([t]) => shownTab(t) && !hidden.includes(t)).map(([t, name]) => {
          const unlinked = !signedIn(t) && t !== 'starred' && t !== 'router' && !CHATBOT_ONLY_PAGES.includes(t);
          const status = unlinked ? providerStatus(t) : null;
          return (
            <button
              key={t}
              type="button"
              title={signedIn(t) ? name : `${name} (${status})`}
              aria-label={signedIn(t) ? name : `${name} (${status})`}
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
          );
        })}
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
        <div className="rail-menu" style={{ left: ctx.x, top: Math.max(0, Math.min(ctx.y, (menuRef.current?.clientHeight ?? 470) - 48 - (ctx.tab === 'starred' ? hidden.length : 2) * 32)) }}>
          {ctx.tab === 'starred' ? hidden.map((t) => (
            <button key={t} type="button" className="menu-item" onClick={() => { setHidden(hidden.filter((h) => h !== t)); setCtx(null); }}>
              <RailIcon tab={t} /><span className="grow ellipsis">Show {TABS.find(([id]) => id === t)![1]}</span>
            </button>
          )) : (() => {
            const t = ctx.tab;
            const name = TABS.find(([id]) => id === t)![1];
            return (
              <>
                <div className="tiny dim rail-menu-head">{name}{!signedIn(t) && !CHATBOT_ONLY_PAGES.includes(t) ? ` · ${providerStatus(t)}` : ''}</div>
                {!signedIn(t) && (
                  <button type="button" className="menu-item" onClick={() => { setTab(t); setSearch(''); setAdding(false); setCtx(null); }}>
                    <Icon name="user" /><span className="grow">{t === 'opencode' ? 'Set up OpenCode' : t === 'additional' ? 'Add a provider' : t === 'chatbots' ? 'Connect a chatbot' : CHATBOT_ONLY_PAGES.includes(t) ? `Connect ${name}` : `Sign in to ${name}`}</span>
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
        {listed && (
          <div className="model-sort">
            <select className="input select compact" aria-label="Sort models" title="Sort models" value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
              {SORTS.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
            </select>
            <button type="button" className={`chip ${freeOnly ? 'on' : ''}`} aria-pressed={freeOnly} title="Show only models that cost nothing here" onClick={() => setFreeOnly(!freeOnly)}>Free only</button>
          </div>
        )}
        {tab === 'kilo' && <div className="tiny dim">{kiloWarning}</div>}
        {tab === 'deepseek-web' && !q && deepseek?.enabled && !deepseek.signedIn && <div className="card-note"><DeepSeekWebPanel compact /></div>}
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
          {botPage && <ChatbotPage key={tab} tab={tab} close={close} />}
          {routerTab ? <AgentRouterPanel compact close={close} /> : signIn ? <SignedOut tab={tab} /> : rows.map((r) => {
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
          {listed && rows.length === 0 && <div className="small dim pad">{empty}</div>}
        </div>
      </div>
      {hover && createPortal(<ModelCard {...hover} />, document.body)}
    </div>
  );
}

const newRouterId = () => `router-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

/** A one-line description of a router's route, for its row. */
function routeSummary(r: AgentRouter): string {
  if (r.steps.length === 0) return 'No models yet';
  const n = `${r.steps.length} model${r.steps.length === 1 ? '' : 's'}`;
  return r.useJev ? `${n} · Jev picks by difficulty` : `${n} · in order`;
}

/**
 * The Agent Routers: a list to pick from like models, each with an edit button, and a button to make another.
 * Shared by the Model Menu tab and Settings > Agent Router.
 */
export function AgentRouterPanel({ compact = false, close }: { compact?: boolean; close?: () => void }) {
  const cfg = useView('cfg');
  const current = useView('model');
  const routers: AgentRouter[] = cfg.agentRouters ?? [];
  const [editing, setEditing] = useState<string | null>(null);
  const editingRouter = editing ? routers.find((r) => r.id === editing) : undefined;
  const saveAll = (next: AgentRouter[]) => act('setAgentRouters', { agentRouters: next });
  const activeId = current.provider === 'router' ? (routers.some((r) => r.id === current.key) ? current.key : routers[0]?.id) : null;

  const create = () => {
    const used = new Set(routers.map((r) => r.name));
    let n = routers.length + 1;
    while (used.has(`Router ${n}`)) n++;
    const r: AgentRouter = { id: newRouterId(), name: `Router ${n}`, useJev: false, steps: [], lastProvider: null };
    saveAll([...routers, r]);
    setEditing(r.id);
  };

  const choose = (id: string) => {
    act('chooseModel', { key: id, provider: 'router' });
    close?.();
  };

  if (editingRouter) {
    return (
      <AgentRouterEditor
        compact={compact}
        router={editingRouter}
        active={activeId === editingRouter.id}
        onSave={(next) => saveAll(routers.map((r) => (r.id === next.id ? next : r)))}
        onDelete={() => { saveAll(routers.filter((r) => r.id !== editingRouter.id)); setEditing(null); }}
        onUse={() => choose(editingRouter.id)}
        onBack={() => setEditing(null)}
      />
    );
  }

  return (
    <div className={`router-panel stack ${compact ? 'compact' : ''}`}>
      <div className="row">
        <span className="tiny dim strong grow">AGENT ROUTERS</span>
        <button type="button" className="button compact" title="Make a new Agent Router" onClick={create}>+ New router</button>
      </div>
      {routers.length === 0 ? (
        <div className="small dim pad router-empty">
          No Agent Routers yet. Click <b>+ New router</b> to build a route of providers and models (for example Opus 4.6 → Haiku 4.5 → DeepSeek).
        </div>
      ) : (
        <div className="stack" style={{ gap: 2 }}>
          {routers.map((r) => (
            <div
              key={r.id}
              className={`model-row router-row ${activeId === r.id ? 'selected' : ''}`}
              title={`Use ${r.name}`}
              onClick={() => choose(r.id)}
            >
              <Icon name="route" size={14} />
              <span className="grow ellipsis">
                {r.name}
                <span className="tiny dim"> · {routeSummary(r)}</span>
              </span>
              {activeId === r.id && <Icon name="check" size={14} />}
              <IconButton icon="pencil" tip={`Edit ${r.name}`} size={24} onClick={(e) => { e.stopPropagation(); setEditing(r.id); }} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** One Agent Router's name, Jev switch and route, saved as they change. */
function AgentRouterEditor({ compact, router, active, onSave, onDelete, onUse, onBack }: {
  compact: boolean;
  router: AgentRouter;
  active: boolean;
  onSave: (next: AgentRouter) => void;
  onDelete: () => void;
  onUse: () => void;
  onBack: () => void;
}) {
  const models = useCore((s) => s.models);
  const cfg = useView('cfg');
  const current = useView('model');
  const { additionalProviders } = useCore((s) => s.statics);
  const [name, setName] = useState(router.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  useEffect(() => setName(router.name), [router.id]);

  const rows = useMemo(() => allRows(models), [models]);

  const providers = useMemo(() => {
    const base: { id: string; name: string }[] = [
      { id: 'scoutcode', name: 'ScoutCode' },
      { id: 'claude', name: 'Claude Code' },
      { id: 'codex', name: 'Codex' },
      { id: 'openrouter', name: 'OpenRouter' },
      { id: 'nvidia', name: 'NVIDIA NIM' },
      { id: 'opencode', name: 'OpenCode' },
      { id: 'kilo', name: 'Kilo Code' },
      { id: 'copilot', name: 'GitHub Copilot' },
      { id: 'devin', name: 'Devin' },
      { id: 'kiro', name: 'Kiro' },
      { id: 'grok', name: 'Grok Build' },
      { id: 'hermes', name: 'Hermes' },
    ];
    for (const a of additionalProviders) {
      if (cfg.additional.some((l) => l.id === a.id) || rows.some((r) => r.provider === a.id)) {
        base.push({ id: a.id, name: a.name });
      }
    }
    return base;
  }, [additionalProviders, cfg.additional, rows]);

  const defaultProvider = useMemo(() => {
    if (router.lastProvider && providers.some((p) => p.id === router.lastProvider)) return router.lastProvider;
    const lastStep = router.steps[router.steps.length - 1];
    if (lastStep && providers.some((p) => p.id === lastStep.provider)) return lastStep.provider;
    if (current.provider !== 'router' && providers.some((p) => p.id === current.provider)) return current.provider;
    return 'scoutcode';
  }, [router.lastProvider, router.steps, current.provider, providers]);

  // A new router opens straight on adding its first model.
  const [adding, setAdding] = useState(router.steps.length === 0);
  const [addProvider, setAddProvider] = useState(defaultProvider);
  const [addSearch, setAddSearch] = useState('');
  const [addKey, setAddKey] = useState('');

  const providerRows = useMemo(() => {
    const q = addSearch.trim().toLowerCase();
    return rows
      .filter((r) => r.provider === addProvider && r.key !== 'auto' && (!q || [r.key, r.name, r.star].some((s) => s.toLowerCase().includes(q))))
      .sort((a, b) => compare('smart', a, b));
  }, [rows, addProvider, addSearch]);

  useEffect(() => {
    if (providerRows.length > 0 && !providerRows.some((r) => r.key === addKey)) {
      setAddKey(providerRows[0].key);
    } else if (providerRows.length === 0) {
      setAddKey('');
    }
  }, [providerRows, addKey]);

  const save = onSave;

  const commitName = () => {
    const n = name.trim();
    if (!n) setName(router.name);
    else if (n !== router.name) save({ ...router, name: n });
  };

  const openAdd = () => {
    setAddProvider(defaultProvider);
    setAddSearch('');
    setAdding(true);
  };

  const addStep = (key: string, provider: string) => {
    if (!key) return;
    const found = rows.find((r) => r.provider === provider && r.key === key)?.model ?? models.find((m) => m.id === key);
    const intel = (found ? intelligence(found) : null) ?? 2;
    const nextStep: AgentRouteStep = { key, provider, intelligence: intel, retryRateLimit: false };
    save({ ...router, lastProvider: provider, steps: [...router.steps, nextStep] });
    setAdding(false);
    setAddSearch('');
  };

  const updateStep = (idx: number, patch: Partial<AgentRouteStep>) => {
    save({ ...router, steps: router.steps.map((s, i) => (i === idx ? { ...s, ...patch } : s)) });
  };

  const moveStep = (idx: number, dir: -1 | 1) => {
    const target = idx + dir;
    if (target < 0 || target >= router.steps.length) return;
    const next = [...router.steps];
    const [item] = next.splice(idx, 1);
    next.splice(target, 0, item);
    save({ ...router, steps: next });
  };

  const removeStep = (idx: number) => {
    save({ ...router, steps: router.steps.filter((_, i) => i !== idx) });
  };

  return (
    <div className={`router-panel stack ${compact ? 'compact' : ''}`}>
      <div className="router-header row">
        <IconButton icon="back" tip="All routers" size={24} onClick={() => { commitName(); onBack(); }} />
        <input
          className="input compact grow"
          aria-label="Router name"
          placeholder="Router name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={commitName}
          onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }}
        />
        <button type="button" className={`button compact ${active ? '' : 'primary'}`} disabled={active && !compact} onClick={() => { commitName(); onUse(); }}>
          {active ? 'In use' : 'Use'}
        </button>
      </div>

      <div className="router-option row">
        <div className="grow">
          <div className="small strong">Use Jev</div>
          <div className="tiny dim">
            Rank each model's intelligence (Light, Capable, Frontier). Jev rates prompt difficulty and sends it to the needed model.
          </div>
        </div>
        <Switch on={router.useJev} onChange={(v) => save({ ...router, useJev: v })} label="Use Jev" />
      </div>

      <div className="router-steps stack">
        <div className="row">
          <span className="tiny dim strong grow">
            {router.useJev ? 'MODELS IN ROUTE (RANKED BY INTELLIGENCE, THEN ORDER)' : 'ROUTE ORDER (PROVIDER BY PROVIDER)'}
          </span>
          <button type="button" className="button compact" title="Add a provider and model to the route" onClick={() => (adding ? setAdding(false) : openAdd())}>
            {adding ? 'Cancel' : '+ Add to route'}
          </button>
        </div>

        {adding && (
          <div className="router-add stack">
            <div className="row" style={{ gap: 8 }}>
              <label className="stack grow" style={{ gap: 4 }}>
                <span className="tiny dim">Provider</span>
                <select
                  className="input select compact"
                  value={addProvider}
                  onChange={(e) => {
                    setAddProvider(e.target.value);
                    save({ ...router, lastProvider: e.target.value });
                  }}
                >
                  {providers.map((p) => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              </label>
              <label className="stack grow" style={{ gap: 4 }}>
                <span className="tiny dim">Filter models</span>
                <input
                  className="input compact"
                  placeholder={`Search ${runsOn(addProvider)} models…`}
                  value={addSearch}
                  onChange={(e) => setAddSearch(e.target.value)}
                />
              </label>
            </div>

            {providerRows.length > 0 ? (
              <>
                <div className="row" style={{ gap: 8 }}>
                  <select
                    className="input select compact grow"
                    aria-label="Model"
                    value={addKey}
                    onChange={(e) => setAddKey(e.target.value)}
                  >
                    {providerRows.map((r) => {
                      const lvl = intelligence(r.model);
                      const tag = lvl ? ` · ${INTEL_NAMES[lvl]}` : '';
                      return (
                        <option key={r.key} value={r.key}>
                          {r.name}{tag}{rowFree(r) ? ' (Free)' : ''}
                        </option>
                      );
                    })}
                  </select>
                  <button type="button" className="button primary compact" onClick={() => addStep(addKey, addProvider)}>
                    Add
                  </button>
                </div>
                <div className="router-add-list">
                  {providerRows.slice(0, 25).map((r) => (
                    <div
                      key={`${r.provider}:${r.key}`}
                      className={`model-row compact ${addKey === r.key ? 'selected' : ''}`}
                      onClick={() => addStep(r.key, r.provider)}
                    >
                      {logo(r.provider) && <img className="logo row-logo" src={logo(r.provider)} width={13} height={13} alt="" />}
                      <span className="grow ellipsis small">{r.name}</span>
                      <IntelBars level={intelligence(r.model)} />
                      {rowFree(r) && <span className="badge">Free</span>}
                    </div>
                  ))}
                </div>
              </>
            ) : (
              <div className="tiny dim pad">No models available for {runsOn(addProvider)}. Make sure this provider is signed in or linked.</div>
            )}
          </div>
        )}

        {router.steps.length === 0 ? (
          <div className="small dim pad router-empty">
            No models in this route yet. Click <b>+ Add to route</b> to add your first provider and model.
          </div>
        ) : (
          router.steps.map((step, i) => (
            <AgentRouteStepRow
              key={`${i}:${step.provider}:${step.key}`}
              step={step}
              index={i}
              total={router.steps.length}
              useJev={router.useJev}
              onUpdate={(patch) => updateStep(i, patch)}
              onMove={(dir) => moveStep(i, dir)}
              onRemove={() => removeStep(i)}
            />
          ))
        )}
      </div>

      <div className="row" style={{ gap: 6 }}>
        <div className="grow" />
        {confirmDelete ? (
          <>
            <button type="button" className="button compact" onClick={() => setConfirmDelete(false)}>Keep it</button>
            <button type="button" className="button compact danger" onClick={onDelete}>Delete “{router.name}”</button>
          </>
        ) : (
          <button type="button" className="button compact" onClick={() => setConfirmDelete(true)}>Delete router</button>
        )}
      </div>
    </div>
  );
}

function AgentRouteStepRow({
  step,
  index,
  total,
  useJev,
  onUpdate,
  onMove,
  onRemove,
}: {
  step: AgentRouteStep;
  index: number;
  total: number;
  useJev: boolean;
  onUpdate: (patch: Partial<AgentRouteStep>) => void;
  onMove: (dir: -1 | 1) => void;
  onRemove: () => void;
}) {
  const models = useCore((s) => s.models);
  const found = models.find(
    (x) =>
      x.id === step.key ||
      x.openrouterId === step.key ||
      x.nvidiaId === step.key ||
      x.opencodeId === step.key ||
      x.kiloId === step.key ||
      x.copilotId === step.key ||
      x.devinId === step.key ||
      x.kiroId === step.key ||
      x.codexId === step.key ||
      x.claudeId === step.key ||
      x.grokId === step.key ||
      x.hermesId === step.key
  );
  const modelLabel = found?.label ?? (step.key.split('/').pop()?.split(':')[0] ?? step.key);
  const providerLabel = runsOn(step.provider);
  const level: 1 | 2 | 3 = step.intelligence === 1 || step.intelligence === 3 ? step.intelligence : 2;

  return (
    <div className="router-step">
      <div className="row" style={{ gap: 6 }}>
        <span className="dim small mono">{index + 1}.</span>
        {logo(step.provider) && <img className="logo row-logo" src={logo(step.provider)} width={14} height={14} alt="" />}
        <span className="grow ellipsis strong small">{modelLabel}</span>
        <span className="badge">{providerLabel}</span>
        <IconButton icon="chevronUp" tip="Move up" size={20} disabled={index === 0} onClick={() => onMove(-1)} />
        <IconButton icon="chevronDown" tip="Move down" size={20} disabled={index === total - 1} onClick={() => onMove(1)} />
        <IconButton icon="close" tip="Remove from route" size={20} onClick={onRemove} />
      </div>
      <div className="row router-step-controls">
        <label className="row" style={{ gap: 6 }} title="Intelligence rank used when Jev rates prompt difficulty">
          <IntelBars level={level} />
          <span className="tiny dim">{useJev ? 'Jev rank:' : 'Intelligence:'}</span>
          <select
            className="input select compact tiny-select"
            value={level}
            onChange={(e) => onUpdate({ intelligence: Number(e.target.value) as 1 | 2 | 3 })}
          >
            <option value={1}>1 · Light</option>
            <option value={2}>2 · Capable</option>
            <option value={3}>3 · Frontier</option>
          </select>
        </label>
        <button
          type="button"
          className={`chip tiny-chip ${step.retryRateLimit ? 'on' : ''}`}
          aria-pressed={step.retryRateLimit}
          title={
            step.retryRateLimit
              ? 'When rate-limited (429), wait and retry this provider before moving to the next step'
              : 'When rate-limited (429), immediately move to the next step in the router'
          }
          onClick={() => onUpdate({ retryRateLimit: !step.retryRateLimit })}
        >
          {step.retryRateLimit ? 'Retry on rate limit: On' : 'Retry on rate limit: Off'}
        </button>
      </div>
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
  const isRouter = current.provider === 'router';
  const label = isRouter ? current.short || 'Agent Router' : models.find((m) => m.id === current.key)?.label ?? current.short;
  const shown = label.length > 26 ? `${label.slice(0, 25)}…` : label;
  const onAdditional = additionalProviders.some((a) => a.id === current.provider);
  const hint = isRouter ? `Agent Router · ${current.short}` : current.provider === 'scoutcode' ? 'Model' : `Model · running on ${runsOn(current.provider)}`;
  const start: Tab = isRouter ? 'router' : onAdditional ? 'additional' : TABS.some(([t]) => t === current.provider) ? (current.provider as Tab) : 'scoutcode';
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
export function useModelName(m?: Partial<ModelRef> | null): string {
  const models = useCore((s) => s.models);
  if (!m?.key) return '';
  if (m.provider === 'router' || m.key === 'router') return 'Agent Router';
  const key = m.key;
  const found = models.find(
    (x) =>
      x.id === key ||
      x.openrouterId === key ||
      x.nvidiaId === key ||
      x.opencodeId === key ||
      x.kiloId === key ||
      x.copilotId === key ||
      x.devinId === key ||
      x.kiroId === key ||
      x.codexId === key ||
      x.claudeId === key ||
      x.grokId === key ||
      x.hermesId === key
  );
  let label = found?.label;
  if (!label) {
    const s = key.split('/').pop()?.split(':')[0] ?? key;
    label = s === 'auto' ? 'Auto' : s;
  }
  if (!m.provider || m.provider === 'scoutcode') return label;
  return `${label} · ${runsOn(m.provider)}`;
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
