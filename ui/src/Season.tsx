// The Scout Season: Scout Points and the season's tiers, a popup like Usage. Admins set the seasons here too.
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { act, num, shortDate, usd, type AdminSeason, type AdminSeasons, type PointSource, type SeasonInfo, type SeasonReward, type SeasonTier, type SeasonTierState } from './api';
import { ui, useUi, useView } from './store';
import { ErrorBoundary } from './ErrorBoundary';
import { Icon, IconButton, Select, Spinner } from './widgets';
import './season.css';

const DAY = 86_400_000;
const day = (iso: string | null | undefined) => (iso ? shortDate(iso.slice(0, 10)) : '');
const points = (n: number) => `${num(n).toLocaleString()} ${num(n) === 1 ? 'point' : 'points'}`;

export const rewardText = (r: SeasonReward): string => (r.kind === 'balance' ? `${usd(r.amount)} balance` : `${r.days} ${r.days === 1 ? 'day' : 'days'} of ScoutPass`);

function daysLeft(endsAt: string): string {
  const d = Math.ceil((Date.parse(endsAt) - Date.now()) / DAY);
  return d <= 1 ? 'Ends today' : `${d} days left`;
}

/** One reward on a tier: what it is, and claiming it. */
function Reward({ reward, track, tier, at, pass, busy }: { reward: SeasonReward | null; track: 'free' | 'pass'; tier: SeasonTierState; at: number; pass: boolean; busy: boolean }) {
  if (!reward) return <div className={`season-reward none ${track}`}><span className="tiny faint">No reward</span></div>;
  const claimed = track === 'free' ? tier.claimed : tier.claimedPass;
  const locked = !tier.unlocked || (track === 'pass' && !pass);
  const balance = reward.kind === 'balance';
  return (
    <div className={`season-reward ${track} ${claimed ? 'claimed' : locked ? 'locked' : 'ready'}`} title={rewardText(reward)}>
      <span className="season-reward-icon"><Icon name={balance ? 'credit' : 'star'} size={14} /></span>
      <span className="season-reward-value">{balance ? usd(reward.amount) : `${reward.days} ${reward.days === 1 ? 'day' : 'days'}`}</span>
      <span className="season-reward-kind">{balance ? 'Balance' : 'ScoutPass'}</span>
      <span className="season-reward-state">
        {claimed ? (
          <span className="row green"><Icon name="check" size={12} />Claimed</span>
        ) : locked ? (
          <span className="row faint"><Icon name="lock" size={11} />{tier.unlocked ? 'ScoutPass' : 'Locked'}</span>
        ) : (
          <button type="button" className="button season-claim" disabled={busy} onClick={() => act('usageChange', { body: { action: 'season-claim', tier: at, track }, said: `Claimed ${rewardText(reward)} from tier ${at + 1}.` })}>
            Claim
          </button>
        )}
      </span>
    </div>
  );
}

/** The season's tiers left to right, scrolled to where the user is. The mouse wheel scrolls it sideways. */
function Track({ s, frac, pass, busy }: { s: NonNullable<SeasonInfo['season']>; frac: number; pass: boolean; busy: boolean }) {
  const strip = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const box = strip.current;
    const el = box?.querySelector<HTMLElement>('.season-tier.current') ?? box?.querySelector<HTMLElement>('.season-tier:last-child');
    if (box && el) box.scrollLeft = el.offsetLeft - box.clientWidth / 2 + el.clientWidth / 2;
  }, [s.id]);
  useEffect(() => {
    const box = strip.current;
    if (!box) return;
    // Sideways only while there's further to go, so the page still scrolls past the ends.
    const wheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      const max = box.scrollWidth - box.clientWidth;
      if ((e.deltaY < 0 && box.scrollLeft <= 0) || (e.deltaY > 0 && box.scrollLeft >= max - 1)) return;
      e.preventDefault();
      box.scrollLeft += e.deltaY;
    };
    box.addEventListener('wheel', wheel, { passive: false });
    return () => box.removeEventListener('wheel', wheel);
  }, []);
  const current = s.tiers.findIndex((t) => !t.unlocked);
  return (
    <div className="season-track-wrap">
      <div className="season-track-labels">
        <span className="season-track-label"><Icon name="gift" size={13} />Free</span>
        <span className="season-track-label pass"><Icon name="star" size={13} />ScoutPass</span>
      </div>
      <div className="season-track" ref={strip}>
        {s.tiers.map((t, i) => (
          <div key={i} className={`season-tier ${t.unlocked ? 'unlocked' : ''} ${i === current ? 'current' : ''}`}>
            <div className="season-tier-head">
              <span className="season-tier-line" style={{ '--f': t.unlocked ? 1 : i === current ? frac : 0 } as CSSProperties}><span /></span>
              <span className="season-tier-num">{i + 1}</span>
              <span className="tiny dim">{num(t.points).toLocaleString()} pts</span>
            </div>
            <Reward reward={t.free} track="free" tier={t} at={i} pass={pass} busy={busy} />
            <Reward reward={t.pass} track="pass" tier={t} at={i} pass={pass} busy={busy} />
          </div>
        ))}
      </div>
    </div>
  );
}

function PassStatus({ info }: { info: SeasonInfo }) {
  if (info.scoutPassForever) return <span className="season-chip pass"><Icon name="star" size={12} />ScoutPass</span>;
  if (info.scoutPass) return <span className="season-chip pass"><Icon name="star" size={12} />ScoutPass until {day(info.scoutPassUntil)}</span>;
  return (
    <span className="row min0 season-nopass">
      <span className="season-chip"><Icon name="lock" size={11} />No ScoutPass</span>
      <span className="tiny dim ellipsis">Gold rewards unlock with ScoutPass and stay claimable once you get it.</span>
    </span>
  );
}

function Stat({ value, label }: { value: ReactNode; label: string }) {
  return <div className="season-stat"><b>{value}</b><span className="tiny dim">{label}</span></div>;
}

function SeasonBody() {
  const { hasKey } = useView('cfg');
  const { info: usage, busy, message } = useView('usage');
  useEffect(() => { if (hasKey) act('usageRefresh'); }, [hasKey]);
  if (!hasKey) return <p className="muted">Sign in to ScoutCode to earn Scout Points and climb the Season.</p>;
  if (!usage) return busy ? <Spinner size={16} /> : null;
  const info = usage.season;
  if (!info) return <p className="muted">This ScoutCode server doesn't have Scout Seasons yet.</p>;
  const s = info.season;
  const prev = s && s.tier > 0 ? s.tiers[s.tier - 1].points : 0;
  const frac = !s ? 0 : s.nextPoints === null ? 1 : Math.max(0, Math.min(1, (info.points - prev) / Math.max(1, s.nextPoints - prev)));
  return (
    <div className="stack season">
      {s ? (
        <section className="season-hero">
          <div className="season-hero-head">
            <div className="min0">
              <div className="season-eyebrow">Scout Season · {day(s.startsAt)} – {day(s.endsAt)}</div>
              <h2 className="season-name ellipsis">{s.name}</h2>
            </div>
            <span className="season-chip"><Icon name="clock" size={12} />{daysLeft(s.endsAt)}</span>
          </div>
          <div className="season-stats">
            <Stat value={<>{s.tier}<span className="season-stat-of"> / {s.tiers.length}</span></>} label="Tier" />
            <Stat value={num(info.points).toLocaleString()} label="Season points" />
            <Stat value={num(info.total).toLocaleString()} label="All-time points" />
          </div>
          <div className="season-progress">
            <div className="season-bar" style={{ '--f': frac } as CSSProperties}><span /></div>
            <div className="season-between tiny">
              <span className="dim">{s.nextPoints === null ? `All ${s.tiers.length} tiers unlocked` : `Tier ${s.tier}`}</span>
              {s.nextPoints !== null && <span className="muted"><b className="strong">{points(s.nextPoints - info.points)}</b> to tier {s.tier + 1}</span>}
            </div>
          </div>
          <div className="season-hero-foot">
            <PassStatus info={info} />
            {info.claimable > 0 && (
              <button type="button" className="button season-claim-all" disabled={busy} onClick={() => act('usageChange', { body: { action: 'season-claim' }, said: `Claimed ${info.claimable} Season ${info.claimable === 1 ? 'reward' : 'rewards'}.` })}>
                <Icon name="gift" size={14} />Claim {info.claimable === 1 ? 'reward' : `all ${info.claimable}`}
              </button>
            )}
          </div>
        </section>
      ) : (
        <section className="season-hero">
          <div className="season-eyebrow">Scout Season</div>
          <h2 className="season-name">No Season right now</h2>
          <p className="small dim season-hero-note">
            {info.next ? `${info.next.name} starts ${day(info.next.startsAt)}. ` : 'The next Season hasn’t been announced yet. '}
            Points you earn until then still count towards your total ({points(info.total)}).
          </p>
        </section>
      )}
      {message && <div className={`season-message small ${message[0] ? 'ok' : 'bad'}`}>{message[1]}</div>}

      {s && (
        <section className="season-section">
          <div className="season-section-head">
            <h3>Rewards</h3>
            <span className="tiny dim">Claim them before the Season ends. A new Season starts everyone again from tier 0.</span>
          </div>
          <Track s={s} frac={frac} pass={info.scoutPass} busy={busy} />
        </section>
      )}

      <section className="season-section">
        <div className="season-section-head"><h3>Earn Scout Points</h3></div>
        <div className="season-earn">
          {info.earn.map((e) => (
            <div key={e.id} className="season-earn-item">
              <span className="grow small">{e.label}</span>
              <b className="season-plus">+{num(e.points).toLocaleString()}</b>
            </div>
          ))}
        </div>
      </section>

      {info.recent.length > 0 && (
        <section className="season-section">
          <div className="season-section-head"><h3>Recent points</h3></div>
          <div className="season-list">
            {info.recent.map((r, i) => (
              <div key={i} className="season-list-row">
                <span className="grow small ellipsis">{r.label}{r.note ? <span className="dim"> · {r.note}</span> : null}</span>
                <span className="tiny dim season-date">{day(r.at)}</span>
                <b className={`season-plus ${r.points < 0 ? 'minus' : ''}`}>{r.points < 0 ? '' : '+'}{num(r.points).toLocaleString()}</b>
              </div>
            ))}
          </div>
        </section>
      )}
      {usage.admin && usage.seasons && <ErrorBoundary what="Season admin"><SeasonAdmin seasons={usage.seasons} busy={busy} /></ErrorBoundary>}
    </div>
  );
}

// ---------- admins: setting seasons ----------

type DraftReward = { kind: 'none' | 'balance' | 'scoutpass'; value: string };
type DraftTier = { points: string; free: DraftReward; pass: DraftReward };
type Draft = { id?: string; name: string; startsAt: string; endsAt: string; tiers: DraftTier[]; points: Record<string, string> };

const NONE: DraftReward = { kind: 'none', value: '' };
const toDraftReward = (r: SeasonReward | null): DraftReward => (!r ? NONE : r.kind === 'balance' ? { kind: 'balance', value: String(r.amount) } : { kind: 'scoutpass', value: String(r.days) });
const fromDraftReward = (r: DraftReward): SeasonReward | null => (r.kind === 'none' ? null : r.kind === 'balance' ? { kind: 'balance', amount: Number(r.value) } : { kind: 'scoutpass', days: Number(r.value) });
const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** A season to start from: 20 tiers 40 points apart, balance on the way and ScoutPass at tiers 10 and 20. */
function starterTiers(count = 20, step = 40): DraftTier[] {
  return Array.from({ length: count }, (_, i) => {
    const n = i + 1;
    const free: DraftReward = n % 10 === 0 ? { kind: 'scoutpass', value: n === count ? '7' : '3' } : n % 2 ? { kind: 'balance', value: '0.1' } : NONE;
    return { points: String(step * n), free, pass: { kind: 'balance', value: n === count ? '1' : '0.2' } };
  });
}

function draftOf(s: AdminSeason | null, all: AdminSeasons): Draft {
  const points = Object.fromEntries(all.sources.map((p) => [p.id, String(s?.points[p.id] ?? p.points)]));
  if (s) return { id: s.id, name: s.name, startsAt: s.startsAt.slice(0, 10), endsAt: s.endsAt.slice(0, 10), tiers: s.tiers.map((t: SeasonTier) => ({ points: String(t.points), free: toDraftReward(t.free), pass: toDraftReward(t.pass) })), points };
  // A new season starts when the last one ends, or today.
  const start = Math.max(Date.parse(isoDay(Date.now())), ...all.list.map((x) => Date.parse(x.endsAt)));
  return { name: `Season ${all.list.length + 1}`, startsAt: isoDay(start), endsAt: isoDay(start + 30 * DAY), tiers: starterTiers(), points };
}

function RewardEdit({ value, onChange }: { value: DraftReward; onChange: (r: DraftReward) => void }) {
  return (
    <div className="row season-reward-edit">
      <Select className="input select" value={value.kind} onChange={(e) => onChange({ kind: e.target.value as DraftReward['kind'], value: '' })}>
        <option value="none">Nothing</option>
        <option value="balance">Balance</option>
        <option value="scoutpass">ScoutPass</option>
      </Select>
      {value.kind !== 'none' && <input className="input field-70" placeholder={value.kind === 'balance' ? 'USD' : 'Days'} value={value.value} onChange={(e) => onChange({ ...value, value: e.target.value })} />}
    </div>
  );
}

function SeasonEditor({ draft, setDraft, sources, busy, onDone }: { draft: Draft; setDraft: (d: Draft) => void; sources: AdminSeasons['sources']; busy: boolean; onDone: () => void }) {
  const [count, setCount] = useState(String(draft.tiers.length));
  const [step, setStep] = useState('40');
  const [error, setError] = useState<string | null>(null);
  const tier = (i: number, t: Partial<DraftTier>) => setDraft({ ...draft, tiers: draft.tiers.map((x, j) => (j === i ? { ...x, ...t } : x)) });
  const remake = (rewards: boolean) => {
    const n = Number(count);
    const p = Number(step);
    if (!Number.isInteger(n) || n < 1 || n > 200 || !Number.isInteger(p) || p < 1) return setError('Tiers are a whole number from 1 to 200, and points per tier a whole number from 1.');
    setError(null);
    const starter = starterTiers(n, p);
    setDraft({ ...draft, tiers: starter.map((t, i) => (rewards ? t : { ...t, free: draft.tiers[i]?.free ?? NONE, pass: draft.tiers[i]?.pass ?? NONE })) });
  };
  const save = () => {
    const season = {
      id: draft.id,
      name: draft.name,
      startsAt: draft.startsAt,
      endsAt: draft.endsAt,
      tiers: draft.tiers.map((t) => ({ points: Number(t.points), free: fromDraftReward(t.free), pass: fromDraftReward(t.pass) })),
      points: Object.fromEntries(Object.entries(draft.points).map(([k, v]) => [k, Number(v)])),
    };
    act('usageChange', { body: { action: 'season-save', season }, said: `Saved ${draft.name}.` });
    onDone();
  };
  return (
    <div className="season-editor">
      <div className="season-editor-title">{draft.id ? `Editing ${draft.name}` : 'New season'}</div>

      <div className="season-fields">
        <label className="season-field grow"><span>Name</span><input className="input" placeholder="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} /></label>
        <label className="season-field"><span>Starts (UTC)</span><input className="input" type="date" value={draft.startsAt} onChange={(e) => setDraft({ ...draft, startsAt: e.target.value })} /></label>
        <label className="season-field"><span>Ends (UTC)</span><input className="input" type="date" value={draft.endsAt} onChange={(e) => setDraft({ ...draft, endsAt: e.target.value })} /></label>
      </div>
      <div className="tiny dim">A season starts at 00:00 UTC on its start date and ends at 00:00 UTC on its end date.</div>

      <div className="season-editor-group">
        <div className="season-editor-label">Points per action</div>
        <div className="season-points-edit">
          {sources.map((p) => (
            <label key={p.id} className="season-points-row small">
              <span className="grow ellipsis">{p.label}</span>
              <input className="input field-70" value={draft.points[p.id] ?? ''} onChange={(e) => setDraft({ ...draft, points: { ...draft.points, [p.id as PointSource]: e.target.value } })} />
            </label>
          ))}
        </div>
      </div>

      <div className="season-editor-group">
        <div className="season-between">
          <div className="season-editor-label">Tiers ({draft.tiers.length})</div>
          <div className="row wrap season-generate">
            <input className="input field-70" placeholder="Tiers" value={count} onChange={(e) => setCount(e.target.value)} />
            <span className="small dim">tiers,</span>
            <input className="input field-70" placeholder="Points" value={step} onChange={(e) => setStep(e.target.value)} />
            <span className="small dim">points apart</span>
            <button type="button" className="button compact" onClick={() => remake(false)}>Space out</button>
            <button type="button" className="button compact" onClick={() => remake(true)}>Starter rewards</button>
          </div>
        </div>
        {error && <div className="small red">{error}</div>}
        <div className="season-edit-tiers">
          <div className="season-edit-tier head tiny dim"><span>#</span><span>Points</span><span>Free reward</span><span>ScoutPass reward</span><span /></div>
          {draft.tiers.map((t, i) => (
            <div key={i} className="season-edit-tier">
              <span className="tiny dim">{i + 1}</span>
              <input className="input" value={t.points} onChange={(e) => tier(i, { points: e.target.value })} />
              <RewardEdit value={t.free} onChange={(free) => tier(i, { free })} />
              <RewardEdit value={t.pass} onChange={(pass) => tier(i, { pass })} />
              <IconButton icon="trash" tip="Remove this tier" size={26} onClick={() => setDraft({ ...draft, tiers: draft.tiers.filter((_, j) => j !== i) })} />
            </div>
          ))}
        </div>
      </div>

      <div className="row season-editor-foot">
        <button type="button" className="button compact" onClick={() => {
          const last = Number(draft.tiers[draft.tiers.length - 1]?.points) || 0;
          setDraft({ ...draft, tiers: [...draft.tiers, { points: String(last + (Number(step) || 40)), free: NONE, pass: NONE }] });
        }}><Icon name="plus" size={13} />Add tier</button>
        <span className="grow" />
        <button type="button" className="button ghost" onClick={onDone}>Cancel</button>
        <button type="button" className="button primary" disabled={busy} onClick={save}>Save season</button>
      </div>
    </div>
  );
}

function SeasonAdmin({ seasons, busy }: { seasons: AdminSeasons; busy: boolean }) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [user, setUser] = useState('');
  const [give, setGive] = useState('');
  const now = Date.now();
  const givePoints = () => {
    const n = Number(give);
    if (!user.trim() || !Number.isInteger(n) || n === 0) return;
    act('usageChange', { body: { action: 'season-give-points', username: user.trim(), points: n }, said: `${n > 0 ? 'Gave' : 'Took'} ${Math.abs(n)} points ${n > 0 ? 'to' : 'from'} ${user.trim()}.` });
    setGive('');
  };
  return (
    <fieldset className="admin season-admin" disabled={busy}>
      <div className="season-section-head">
        <h3>Admin: Seasons</h3>
        {!draft && <button type="button" className="button compact" onClick={() => setDraft(draftOf(null, seasons))}><Icon name="plus" size={13} />New season</button>}
      </div>
      <div className="tiny dim">Seasons run one after another. When one ends, the next one that's set starts on its date, and everyone's season points start again from 0.</div>
      <div className="season-list">
        {seasons.list.map((s) => {
          const state = s.id === seasons.currentId ? 'Running' : Date.parse(s.startsAt) > now ? 'Upcoming' : 'Ended';
          return (
            <div key={s.id} className="season-list-row">
              <div className="grow min0">
                <div className="row small strong"><span className="ellipsis">{s.name}</span><span className={`badge ${state === 'Running' ? '' : state === 'Upcoming' ? 'paid' : 'warn'}`}>{state}</span></div>
                <div className="tiny dim">{day(s.startsAt)} – {day(s.endsAt)} · {s.tiers.length} tiers up to {num(s.tiers[s.tiers.length - 1]?.points).toLocaleString()} points · {s.players} {s.players === 1 ? 'player' : 'players'}</div>
              </div>
              <button type="button" className="button compact" onClick={() => { setDraft(draftOf(s, seasons)); setConfirm(null); }}>Edit</button>
              <button type="button" className={`button compact ${confirm === s.id ? 'danger' : 'ghost'}`} onClick={() => {
                if (confirm !== s.id) return setConfirm(s.id);
                setConfirm(null);
                act('usageChange', { body: { action: 'season-delete', id: s.id }, said: `Deleted ${s.name}.` });
              }}>{confirm === s.id ? 'Confirm delete' : 'Delete'}</button>
            </div>
          );
        })}
        {!seasons.list.length && <div className="season-list-row small dim">No seasons yet.</div>}
      </div>
      {draft && <SeasonEditor key={draft.id ?? 'new'} draft={draft} setDraft={setDraft} sources={seasons.sources} busy={busy} onDone={() => setDraft(null)} />}
      <div className="season-editor-group">
        <div className="season-editor-label">Give points <span className="dim">(negative takes them away)</span></div>
        <div className="row wrap">
          <input className="input field-180" placeholder="Username" value={user} onChange={(e) => setUser(e.target.value)} />
          <input className="input field-70" placeholder="Points" value={give} onChange={(e) => setGive(e.target.value)} />
          <button type="button" className="button" onClick={givePoints}>Give</button>
        </div>
      </div>
    </fieldset>
  );
}

/** The Scout Season: a popup in the middle of the window, like Usage. Closed with its X, Esc, or a click outside it. */
export function SeasonPopup() {
  const open = useUi((u) => u.seasonOpen);
  const close = () => ui.set({ seasonOpen: false });
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || document.querySelector('.modal-back:not(.season-backdrop)')) return;
      e.preventDefault();
      close();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [open]); // eslint-disable-line
  if (!open) return null;
  return (
    <div className="modal-back season-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) close(); }}>
      <div className="modal settings settings-modal usage-modal season-modal" role="dialog" aria-modal aria-label="Scout Season">
        <header className="settings-bar">
          <Icon name="star" size={15} />
          <b className="chat-title">Scout Season</b>
          <span className="grow" />
          <span className="tiny dim usage-exit-hint">Press <kbd>Esc</kbd> or click outside to close</span>
          <button type="button" className="settings-close" onClick={close} title="Close (Esc)" aria-label="Close Scout Season"><Icon name="close" size={14} /></button>
        </header>
        <div className="settings-scroll">
          <div className="settings-page">
            <ErrorBoundary what="The Scout Season"><SeasonBody /></ErrorBoundary>
          </div>
        </div>
      </div>
    </div>
  );
}
