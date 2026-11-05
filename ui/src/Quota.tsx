import { useEffect, useState } from 'react';
import { act, openExternal, shortDate, usd, type QuotaMeter, type QuotaSnapshot, type View } from './api';
import { ui, useCore, useView } from './store';
import { Icon, Popover } from './widgets';

/** How each provider tells Scout its limits: Scout can ask, it says so in its answers, or it doesn't say. */
const HOW: Record<string, 'asks' | 'answers' | 'never'> = {
  scoutcode: 'asks', openrouter: 'asks', claude: 'asks', codex: 'asks', grok: 'asks',
  copilot: 'answers', nvidia: 'answers', kilo: 'answers', hermes: 'answers',
  devin: 'never', kiro: 'never', opencode: 'never',
};

const TIERS: Record<string, string> = { guest: 'Without an account', desktop: 'Without an account', starter: 'Free account', paying: 'Account with money' };

const amount = (n: number, unit: QuotaMeter['unit']) => (unit === 'USD' ? usd(n) : unit === '%' ? `${Math.round(n)}%` : Math.round(n).toLocaleString());

/** "312 of 400 left", "$7.50 of $10.00 left", "60% left", "Unlimited". */
export function meterText(m: QuotaMeter): string {
  if (m.left === null) return m.note ?? 'Unknown';
  if (m.unit === '%') return `${amount(m.left, '%')} left`;
  return m.limit !== null ? `${amount(m.left, m.unit)} of ${amount(m.limit, m.unit)} left` : `${amount(m.left, m.unit)} left`;
}

/** "resets in 40s", "resets in 3h", "resets Nov 5". */
function resetText(at: number | null, now: number): string | null {
  if (at === null) return null;
  const s = Math.round((at - now) / 1000);
  if (s <= 0) return 'reset now';
  if (s < 90) return `resets in ${s}s`;
  if (s < 5400) return `resets in ${Math.round(s / 60)} min`;
  if (s < 86_400) return `resets in ${Math.round(s / 3600)}h`;
  return `resets ${shortDate(new Date(at).toISOString().slice(0, 10))}`;
}

function ago(at: number, now: number): string {
  const s = Math.max(0, Math.round((now - at) / 1000));
  return s < 60 ? 'just now' : s < 3600 ? `${Math.round(s / 60)} min ago` : `${Math.round(s / 3600)}h ago`;
}

/** Ticks every few seconds, so "resets in" and "ago" stay true. */
function useNow(): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(t);
  }, []);
  return now;
}

/** Share left, 0 to 1, or null when there's nothing to compare it with. */
const share = (m: QuotaMeter) => (m.left !== null && m.limit ? Math.max(0, Math.min(1, m.left / m.limit)) : null);

export function MeterRow({ m, now }: { m: QuotaMeter; now: number }) {
  const f = share(m);
  const reset = resetText(m.resetsAt, now);
  return (
    <div className="quota-meter">
      <div className="row">
        <span className="small grow">{m.label}</span>
        <span className={`small ${f !== null && f < 0.1 ? 'red' : ''}`}>{meterText(m)}</span>
      </div>
      {f !== null && <div className="quota-bar"><span className={f < 0.1 ? 'low' : ''} style={{ width: `${f * 100}%` }} /></div>}
      {(reset || (m.note && m.left !== null)) && <div className="tiny dim">{[reset, m.left !== null ? m.note : null].filter(Boolean).join(' · ')}</div>}
    </div>
  );
}

/** Whether the picked model costs nothing on the provider it runs on. */
function useCurrentFree(): boolean {
  const current = useView('model');
  const models = useCore((s) => s.models);
  const m = models.find((x) => x.id === current.key);
  if (!m) return false;
  if (current.provider === 'scoutcode') return m.free;
  if (current.provider === 'openrouter') return current.id.endsWith(':free') || (m.hosted ? m.openrouterFree : m.free);
  if (current.provider === 'kilo') return m.kiloFree;
  return false;
}

/** The meter the composer shows for a provider: ScoutCode's daily free requests, otherwise the one closest to running out. */
function headline(s: QuotaSnapshot): QuotaMeter | null {
  const daily = s.meters.find((m) => m.label === 'Free requests today' && m.left !== null);
  if (daily) return daily;
  const counted = s.meters.filter((m) => share(m) !== null);
  return counted.sort((a, b) => share(a)! - share(b)!)[0] ?? null;
}

/** In the composer while a free model is picked: what's left of the free requests, with all of the provider's limits on click. */
export function FreeQuotaChip() {
  const current = useView('model');
  const quotas = useView('quotas');
  const free = useCurrentFree();
  const now = useNow();
  const mine = quotas.filter((s) => s.provider === current.provider);
  const top = mine.map(headline).find((m) => m) ?? null;
  if (!free || !top) return null;
  const f = share(top);
  const short = top.label === 'Free requests today' ? `${amount(top.left!, top.unit)} free today` : `${amount(top.left!, top.unit)} left`;
  return (
    <Popover align="end" side="top" width={300} onOpen={() => act('quotaRefresh')} trigger={(_, toggle) => (
      <button type="button" className={`chip ${f !== null && f < 0.1 ? 'red' : ''}`} title="Free requests left" onClick={toggle}>{short}</button>
    )}>
      {(close) => (
        <div className="stack pad">
          <b className="strong">Limits{mine[0]?.tier ? ` · ${TIERS[mine[0].tier] ?? mine[0].tier}` : ''}</b>
          {mine.map((s) => (
            <div key={s.account ?? ''} className="stack">
              {s.account && <div className="tiny muted">{s.account}</div>}
              {s.meters.map((m) => <MeterRow key={m.label} m={m} now={now} />)}
            </div>
          ))}
          <button type="button" className="link small quota-all" onClick={() => { close(); ui.set({ settings: 'usage' }); }}>Every provider's limits</button>
        </div>
      )}
    </Popover>
  );
}

/** Providers whose plan limits Scout asks for, so the composer's ring can show them as soon as one is picked. */
const PLAN_LIMITS = new Set(['claude', 'codex', 'grok']);

/** The picked provider's plan limit closest to running out, with every meter it has, or null when it hasn't said any. */
export function usePlanLimit(): { top: QuotaMeter; snapshots: QuotaSnapshot[] } | null {
  const { provider } = useView('model');
  const quotas = useView('quotas');
  const mine = quotas.filter((s) => s.provider === provider);
  const known = mine.length > 0;
  useEffect(() => {
    if (PLAN_LIMITS.has(provider) && !known) act('quotaRefresh');
  }, [provider, known]);
  const top = mine.map(headline).filter((m): m is QuotaMeter => !!m).sort((a, b) => share(a)! - share(b)!)[0];
  return top ? { top, snapshots: mine } : null;
}

/** Share of a meter used, 0 to 1. */
export const usedShare = (m: QuotaMeter) => 1 - (share(m) ?? 1);

/** Every meter of a provider's snapshots, for a popover. */
export function PlanLimits({ snapshots }: { snapshots: QuotaSnapshot[] }) {
  const now = useNow();
  const tier = snapshots[0]?.tier;
  return (
    <div className="stack">
      <b className="strong">Plan limits{tier ? ` · ${TIERS[tier] ?? tier}` : ''}</b>
      {snapshots.map((s) => (
        <div key={s.account ?? ''} className="stack">
          {s.account && <div className="tiny muted">{s.account}</div>}
          {s.meters.map((m) => <MeterRow key={m.label} m={m} now={now} />)}
        </div>
      ))}
    </div>
  );
}

type Linked = { id: string; name: string };

/** The providers this app is set up with, in the order Settings > Providers lists them. */
function linkedProviders(cfg: View['cfg'], additional: { id: string; name: string }[]): Linked[] {
  const all: [string, string, boolean][] = [
    ['scoutcode', 'ScoutCode', cfg.hasKey || cfg.guest],
    ['claude', 'Claude Code', cfg.claude],
    ['codex', 'Codex', cfg.codex],
    ['copilot', 'GitHub Copilot', cfg.copilot],
    ['openrouter', 'OpenRouter', cfg.openrouterKeys.length > 0],
    ['nvidia', 'NVIDIA NIM', !!cfg.nvidiaKey],
    ['opencode', 'OpenCode', cfg.opencode],
    ['kilo', 'Kilo Code', cfg.kilo],
    ['devin', 'Devin', cfg.devin],
    ['kiro', 'Kiro', cfg.kiro],
    ['grok', 'Grok Build', cfg.grokSignedIn || !!cfg.grokKey],
    ['hermes', 'Hermes', cfg.hermesSignedIn || !!cfg.hermesKey],
  ];
  const own = all.filter(([, , on]) => on).map(([id, name]) => ({ id, name }));
  return [...own, ...cfg.additional.map((a) => ({ id: a.id, name: additional.find((x) => x.id === a.id)?.name ?? a.id }))];
}

/** Settings > Usage: what each provider has said about its limits. */
export function ProviderQuotas() {
  const cfg = useView('cfg');
  const quotas = useView('quotas');
  const { plans, additionalProviders } = useCore((s) => s.statics);
  const now = useNow();
  useEffect(() => act('quotaRefresh'), []);
  const providers = linkedProviders(cfg, additionalProviders);
  if (!providers.length) return null;
  return (
    <div className="stack quota-providers">
      <div className="row">
        <b className="strong grow">Limits by provider</b>
        <button type="button" className="button" onClick={() => act('quotaRefresh')}><Icon name="refresh" size={13} />Refresh</button>
      </div>
      <div className="tiny dim">What each provider last told Scout. Some only say in their answers, so they show after the next request.</div>
      {providers.map((p) => {
        const mine = quotas.filter((s) => s.provider === p.id);
        const how = HOW[p.id] ?? 'answers';
        const plan = (plans as Record<string, string | undefined>)[p.id];
        return (
          <div key={p.id} className="quota-provider stack">
            <div className="row">
              <span className="strong grow">{p.name}</span>
              {mine[0]?.tier && <span className="tiny muted">{TIERS[mine[0].tier] ?? mine[0].tier}</span>}
              {mine.length > 0 && <span className="tiny dim">{ago(Math.max(...mine.map((s) => s.at)), now)}</span>}
            </div>
            {mine.map((s) => (
              <div key={s.account ?? ''} className="stack">
                {s.account && <div className="tiny muted">{s.account}</div>}
                {s.meters.map((m) => <MeterRow key={m.label} m={m} now={now} />)}
              </div>
            ))}
            {mine.length === 0 && (
              <div className="small dim">
                {how === 'never'
                  ? `${p.name} doesn't tell Scout its limits.`
                  : how === 'asks'
                    ? 'Asking…'
                    : `Shows after ${p.name} answers a request, if it sends its limits.`}
                {plan && <> <button type="button" className="link small" onClick={() => openExternal(plan)}>See your plan</button></>}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
