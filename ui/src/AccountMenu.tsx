// The account: your avatar in the top bar opens a menu with Usage, Settings, updates and the account page.
import { act, openExternal } from './api';
import { ui, useCore, useView } from './store';
import { MenuItem, Popover } from './widgets';
import { useUsageShare } from './Quota';

const SLASH_V1 = new RegExp('/v1$');

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

/** Who's signed in, as the account menu names them. */
function useAccountName() {
  const cfg = useView('cfg');
  const { additionalProviders } = useCore((s) => s.statics);
  const linked = additionalProviders.find((a) => a.id === cfg.additional[0]?.id)?.name;
  return cfg.username ?? (cfg.browserSignIn ? 'Signed in' : cfg.hasKey ? 'API key' : cfg.openrouterKeys.length ? 'OpenRouter key' : cfg.nvidiaKey ? 'NVIDIA key' : cfg.claude ? 'Claude Code' : cfg.codex ? 'Codex' : cfg.opencode ? 'OpenCode' : cfg.grokSignedIn || cfg.grokKey ? 'Grok Build' : cfg.hermesSignedIn || cfg.hermesKey ? 'Hermes' : linked ?? (cfg.guest ? 'No account' : 'Kilo Code'));
}

export function AccountMenu() {
  const cfg = useView('cfg');
  const name = useAccountName();
  const { frac, low, tip } = useUsageShare();
  const pct = Math.round(Math.max(0, Math.min(1, frac)) * 100);
  return (
    <Popover align="end" width={232} trigger={(open, toggle) => (
      <button type="button" className={`account-btn ${open ? 'open' : ''}`} title={name} aria-label="Account menu" aria-expanded={open} onClick={toggle}>
        <span className="avatar">{name.charAt(0).toUpperCase()}</span>
      </button>
    )}>
      {(close) => (
        <div className="account-menu">
          <div className="account-head">
            <span className="avatar">{name.charAt(0).toUpperCase()}</span>
            <span className="grow ellipsis strong">{name}</span>
          </div>
          <hr />
          <button type="button" className="menu-item" onClick={() => { close(); ui.set({ page: 'usage' }); }} title={tip ? `${tip} · ${pct}% used` : `${pct}% used`}>
            <UsageGauge frac={frac} low={low} />
            <span className="grow ellipsis">Usage</span>
            <span className={`tiny ${low ? 'usage-low' : 'dim'}`}>{pct}%</span>
          </button>
          <hr />
          <MenuItem icon="gear" onClick={() => { close(); ui.set({ settings: 'appearance' }); }}>Settings</MenuItem>
          <MenuItem icon="download" onClick={() => { close(); ui.set({ settings: 'updates' }); act('checkUpdates'); }}>Check for updates</MenuItem>
          <MenuItem icon="user" onClick={() => { close(); openExternal(`${cfg.baseUrl.replace(SLASH_V1, '')}/account`); }}>Account</MenuItem>
        </div>
      )}
    </Popover>
  );
}
