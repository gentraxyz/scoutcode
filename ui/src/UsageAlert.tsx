import { useEffect, useState } from 'react';
import { act, DISCORD_INVITE_URL, openExternal, usd } from './api';
import { useView } from './store';
import { Icon, IconButton } from './widgets';

/** Once this much of ScoutCode Cloud's usage is used, the alert suggests the Discord. */
const ALERT_AT = 0.5;
/** Set once the alert is dismissed, so it never comes back on this PC. */
const DISMISSED_KEY = 'scout.usageAlertDismissed';

const wasDismissed = (): boolean => {
  try { return localStorage.getItem(DISMISSED_KEY) === '1'; } catch { return false; }
};
const rememberDismissed = () => {
  try { localStorage.setItem(DISMISSED_KEY, '1'); } catch { /* still hidden for this run */ }
};

/**
 * A small corner alert, once, when more than half of ScoutCode Cloud's usage is used.
 * It suggests joining the Discord for more usage. Closing it, or joining, means it never shows again.
 */
export function UsageAlert() {
  const { hasKey } = useView('cfg');
  const { info } = useView('usage');
  const [gone, setGone] = useState(wasDismissed);
  useEffect(() => { if (hasKey && !gone) act('usageRefresh'); }, [hasKey, gone]);
  const plan = info?.cli ?? info?.money ?? null;
  if (!hasKey || gone || !plan || plan.limit <= 0) return null;
  const used = 1 - plan.left / plan.limit;
  if (used <= ALERT_AT) return null;
  const dismiss = () => { rememberDismissed(); setGone(true); };
  return (
    <div className="usage-alert" role="alert">
      <Icon name="bolt" size={15} />
      <div className="grow min0">
        <div className="strong small">ScoutCode Cloud: over half used</div>
        <div className="tiny dim">{usd(plan.left)} of {usd(plan.limit)} left. Join the Discord for more usage.</div>
      </div>
      <button type="button" className="button primary compact" onClick={() => { dismiss(); openExternal(DISCORD_INVITE_URL); }}>Join Discord</button>
      <IconButton icon="close" tip="Don't show again" size={22} onClick={dismiss} />
    </div>
  );
}
