// The narrow rail down the left edge, like the website's: Chats, Inbox and Issues, with the account at the
// bottom. It stays when the chats sidebar is hidden.
import { act, openExternal } from './api';
import { ui, useCore, useUi, useView } from './store';
import { Icon } from './widgets';
import { ScoutLogo } from './logo';
import { DeltaMark } from './delta/Delta';
import { Hideable } from './Hideable';
import { inboxChats } from './Inbox';
import { useUnseenDreams } from './Dreams';
import { useUsageShare } from './Quota';
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';

const SLASH_V1 = new RegExp('/v1$');

function RailButton({ on, tip, onClick, badge, children }: { on: boolean; tip: string; onClick: () => void; badge?: number; children: ReactNode }) {
  return (
    <button type="button" className={`rail-btn ${on ? 'on' : ''}`} onClick={onClick} title={tip} aria-label={tip}>
      {children}
      {!!badge && <span className="rail-badge">{badge > 99 ? '99+' : badge}</span>}
    </button>
  );
}

/** The rail's house: an outline with an arched door. Selected, the body fills and the door is a real hole in it
 *  (even-odd), so it shows the button behind it instead of a painted-on patch. */
const HOME_BODY = 'M2.750 7.200L7.100 3.400a1.400 1.400 0 0 1 1.800 0l4.350 3.800v5a1.500 1.500 0 0 1-1.500 1.500h-7.500a1.500 1.500 0 0 1-1.500-1.500z';
const HOME_DOOR = 'M6.200 13.700v-2.500a1.800 1.800 0 0 1 3.600 0v2.500';
function HomeIcon() {
  return (
    <svg className="icon home-icon" width={18} height={18} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.400" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path className="home-body" fillRule="evenodd" d={`${HOME_BODY}${HOME_DOOR}z`} />
    </svg>
  );
}

/** The usage button as a car's speedometer: a 240° dial, filled and pointed to how much is used, red near the end. */
const DIAL = 'M2.550 12.650A6.300 6.300 0 1 1 13.450 12.650';
function UsageGauge({ frac, low }: { frac: number; low: boolean }) {
  const f = Math.max(0, Math.min(1, frac));
  return (
    <svg className="icon usage-gauge" width={18} height={18} viewBox="0 0 16 16" fill="none" strokeLinecap="round" aria-hidden>
      <path className="gauge-track" strokeWidth={1.600} d={DIAL} />
      {f > 0 && <path className={`gauge-fill ${low ? 'low' : ''}`} strokeWidth={1.600} d={DIAL} pathLength={1} strokeDasharray={`${f} 1`} />}
      <path className="gauge-needle" stroke="currentColor" strokeWidth={1.400} d="M8 9.500V4.700" style={{ transform: `rotate(${-120 + 240 * f}deg)` }} />
      <circle cx={8} cy={9.500} r={1.300} fill="currentColor" />
    </svg>
  );
}

function UsageButton({ on }: { on: boolean }) {
  const { frac, low, tip } = useUsageShare();
  const pct = Math.round(Math.max(0, Math.min(1, frac)) * 100);
  return (
    <RailButton on={on} tip={`Usage: ${tip ? `${tip} · ` : ''}${pct}% used`} onClick={() => ui.set({ page: 'usage' })}>
      <UsageGauge frac={frac} low={low} />
    </RailButton>
  );
}

/** Who's signed in, as the account menu names them. */
function useAccountName() {
  const cfg = useView('cfg');
  const { additionalProviders } = useCore((s) => s.statics);
  const linked = additionalProviders.find((a) => a.id === cfg.additional[0]?.id)?.name;
  return cfg.username ?? (cfg.browserSignIn ? 'Signed in' : cfg.hasKey ? 'API key' : cfg.openrouterKeys.length ? 'OpenRouter key' : cfg.nvidiaKey ? 'NVIDIA key' : cfg.claude ? 'Claude Code' : cfg.codex ? 'Codex' : cfg.opencode ? 'OpenCode' : cfg.grokSignedIn || cfg.grokKey ? 'Grok Build' : cfg.hermesSignedIn || cfg.hermesKey ? 'Hermes' : linked ?? (cfg.guest ? 'No account' : 'Kilo Code'));
}

/** The account at the foot of the rail. Hovering turns the avatar into an arrow; clicking it slides its options up out
 *  of it as icons, named on hover, with a collapse button on top. */
function Account() {
  const cfg = useView('cfg');
  const name = useAccountName();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setOpen(false); };
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
    };
    document.addEventListener('mousedown', down);
    window.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', down); window.removeEventListener('keydown', key, true); };
  }, [open]);
  const items: [string, string, () => void][] = [
    ['gear', 'Settings', () => ui.set({ settings: 'appearance' })],
    ['download', 'Check for updates', () => { ui.set({ settings: 'updates' }); act('checkUpdates'); }],
    ['user', 'Account', () => openExternal(`${cfg.baseUrl.replace(SLASH_V1, '')}/account`)],
  ];
  return (
    <div ref={box} className={`rail-account ${open ? 'open' : ''}`}>
      {items.map(([icon, tip, run], i) => (
        <button
          key={tip}
          type="button"
          className="rail-btn rail-account-item"
          style={{ '--i': items.length - 1 - i } as CSSProperties}
          data-tip={tip}
          aria-label={tip}
          aria-hidden={!open}
          tabIndex={open ? undefined : -1}
          onClick={() => { setOpen(false); run(); }}
        >
          <Icon name={icon} size={17} />
        </button>
      ))}
      <button type="button" className="rail-avatar" onClick={() => setOpen(!open)} title={open ? undefined : name} aria-label="Account menu" aria-expanded={open}>
        <span className="avatar">{name.charAt(0).toUpperCase()}</span>
        <Icon name="chevronUp" size={16} />
      </button>
    </div>
  );
}

export function NavRail() {
  const page = useUi((u) => u.page);
  const sessions = useView('sessions');
  // The badge counts what wants you: chats waiting on an answer, and finished ones you haven't read.
  const { waiting, unread } = inboxChats(sessions);
  const dreams = useUnseenDreams();
  return (
    <nav className="nav-rail" aria-label="Scout">
      <div className="rail-logo" data-tauri-drag-region><ScoutLogo size={24} drag /></div>
      <RailButton on={page === 'chat'} tip="Home" onClick={() => ui.set({ page: 'chat' })}><HomeIcon /></RailButton>
      <Hideable id="rail.inbox">
        <RailButton on={page === 'inbox'} tip="Inbox: what's in progress and what's yours" badge={waiting.length + unread.length + dreams.length} onClick={() => ui.set({ page: 'inbox' })}><Icon name="inbox" size={18} /></RailButton>
      </Hideable>
      <Hideable id="rail.issues">
        <RailButton on={page === 'delta'} tip="Issues: projects, cycles and docs" onClick={() => ui.set({ page: 'delta' })}><DeltaMark size={17} /></RailButton>
      </Hideable>
      <Hideable id="rail.epsilon">
        <RailButton on={page === 'epsilon'} tip="Epsilon: your cloud computers" onClick={() => ui.set({ page: 'epsilon' })}><Icon name="epsilon" size={18} /></RailButton>
      </Hideable>
      <Hideable id="rail.usage">
        <UsageButton on={page === 'usage'} />
      </Hideable>
      <span className="grow" data-tauri-drag-region />
      <Account />
    </nav>
  );
}
