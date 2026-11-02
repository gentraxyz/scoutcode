import { useMemo, useState } from 'react';
import { act, tokens, type Model, type ModelRef } from './api';
import { AdditionalOptions, BrowserEntry, COPILOT_NOTE, DEVIN_NOTE, GROK_NOTE, GUEST_NOTE, GrokEntry, GuestEntry, HERMES_NOTE, HERMES_WARNING, HermesEntry, KIRO_NOTE, KeyField, logo, NvidiaEntry, OPENCODE_NOTE, OPENCODE_WARNING, OpencodeEntry, ScoutKeyEntry, SignInEntry } from './Accounts';
import { ChatbotsPanel } from './Chatbots';
import { core, useCore, useView } from './store';
import { Icon, IconButton, Link, Popover, Star } from './widgets';
import scoutLogo from '../../assets/icon.png';

type Tab = 'starred' | 'scoutcode' | 'openrouter' | 'nvidia' | 'opencode' | 'kilo' | 'copilot' | 'devin' | 'kiro' | 'grok' | 'hermes' | 'additional' | 'chatbots';
const TABS: [Tab, string][] = [['starred', 'Starred'], ['chatbots', 'MCP chatbots'], ['scoutcode', 'ScoutCode'], ['openrouter', 'OpenRouter'], ['nvidia', 'NVIDIA NIM'], ['opencode', 'OpenCode'], ['kilo', 'Kilo Code'], ['copilot', 'GitHub Copilot'], ['devin', 'Devin'], ['kiro', 'Kiro'], ['grok', 'Grok Build'], ['hermes', 'Hermes'], ['additional', 'Additional Providers']];

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

function runsOn(id: string): string {
  const a = core.get().statics.additionalProviders.find((p) => p.id === id);
  return a?.name ?? TABS.find(([t]) => t === id)?.[1] ?? id;
}

function RailIcon({ tab }: { tab: Tab }) {
  if (tab === 'starred') return <Star on={false} />;
  if (tab === 'additional') return <Icon name="plus" />;
  if (tab === 'chatbots') return <Icon name="chat" />;
  const file = { scoutcode: '', openrouter: 'openrouter', nvidia: 'nvidia', opencode: 'opencode', kilo: 'kilo', copilot: 'copilot', devin: 'devin', kiro: 'kiro', grok: 'grok', hermes: 'hermes' }[tab];
  if (!file) return <img className="logo" src={scoutLogo} width={16} height={16} alt="" />;
  return <img className="logo" src={logo(file)} width={16} height={16} alt="" />;
}

function ModelCard({ m, top, left }: { m: Model; top: number; left: number }) {
  const caps = [m.tools && 'Tools', m.vision && 'Vision'].filter(Boolean).join(' · ');
  const on = [m.hosted && 'ScoutCode', m.openrouterId && 'OpenRouter', m.nvidiaId && 'NVIDIA NIM', m.opencodeId && 'OpenCode', m.kiloId && 'Kilo Code', m.hermesId && 'Hermes', m.additional && runsOn(m.additional[0])].filter(Boolean).join(' or ');
  const stats: [string, string | null | undefined][] = [
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
  const opencodeError = useView('opencodeModelsError');
  const grokError = useView('grokModelsError');
  const hermesError = useView('hermesModelsError');
  const link = useView('link');
  const { kiloWarning, additionalProviders } = useCore((s) => s.statics);
  const [tab, setTab] = useState<Tab>(startTab);
  const [search, setSearch] = useState('');
  const [adding, setAdding] = useState(false);
  const [hover, setHover] = useState<{ m: Model; top: number; left: number } | null>(null);
  const signedIn = (t: Tab) => ({ scoutcode: cfg.hasKey || cfg.guest, openrouter: cfg.openrouterKeys.length > 0, nvidia: cfg.nvidiaKey !== null, opencode: cfg.opencode, copilot: cfg.copilot, devin: cfg.devin, kiro: cfg.kiro, grok: cfg.grokSignedIn || cfg.grokKey !== null, hermes: cfg.hermesSignedIn || cfg.hermesKey !== null, additional: cfg.additional.length > 0, starred: true, kilo: true, chatbots: link.on })[t];
  const q = search.trim().toLowerCase();
  const rows = useMemo(() => {
    const all = allRows(models);
    const pos = (r: ModelRow) => { const i = starred.indexOf(r.star); return i < 0 ? Infinity : i; };
    let list: ModelRow[];
    if (q) list = all.filter((r) => [r.key, r.name, r.star].some((s) => s.toLowerCase().includes(q)));
    else if (tab === 'starred') list = all.filter((r) => starred.includes(r.star));
    else if (tab === 'additional') list = all.filter((r) => additionalProviders.some((a) => a.id === r.provider));
    else list = all.filter((r) => r.provider === tab);
    // Starred first, in the order they were starred. The sort keeps the rest as they came.
    return [...list].sort((a, b) => (pos(a) === pos(b) ? 0 : pos(a) - pos(b)));
  }, [models, q, tab, starred, additionalProviders]);
  const signIn = !q && (!signedIn(tab) || (tab === 'additional' && adding));
  // Chatbots bring their own model, so the tab shows the link instead of a list.
  const chatbots = !q && tab === 'chatbots';
  const empty = loading > 0 ? 'Loading models…' : q ? 'No models match.' : tab === 'starred' ? 'Star a model to keep it here.' : 'No models here yet.';
  return (
    <div className="model-menu">
      <div className="rail">
        {TABS.map(([t, tip]) => (
          <button key={t} type="button" title={signedIn(t) ? tip : `${tip} (not connected)`} aria-label={signedIn(t) ? tip : `${tip} (not connected)`} className={`rail-tab ${tab === t && !q ? 'on' : ''} ${signedIn(t) ? '' : 'off'}`} onClick={() => { setTab(t); setSearch(''); setAdding(false); }}>
            <RailIcon tab={t} />
          </button>
        ))}
      </div>
      <div className="model-column">
        <input className="input" autoFocus placeholder="Search models" value={search} onChange={(e) => setSearch(e.target.value)} />
        {tab === 'kilo' && <div className="tiny dim">{kiloWarning}</div>}
        {tab === 'scoutcode' && cfg.guest && <div className="tiny dim">{GUEST_NOTE} Sign in from Settings &gt; Account for more.</div>}
        {tab === 'kiro' && cfg.kiro && kiroError && <div className="tiny red">{kiroError}</div>}
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
              {cfg.additional.length > 0 && (adding
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
            const free = r.provider === 'kilo' ? m.kiloFree : r.provider !== 'nvidia' && (m.free || r.star.endsWith(':free'));
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
