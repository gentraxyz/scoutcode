import { useEffect } from 'react';
import { act, openExternal } from './api';
import { useView } from './store';
import { Icon } from './widgets';

export function VerifyAlert() {
  const { hasKey, baseUrl } = useView('cfg');
  const { info } = useView('usage');
  useEffect(() => { if (hasKey) act('usageRefresh'); }, [hasKey]);
  const ver = info?.verification;
  if (!hasKey || !ver || ver.verified) return null;
  const site = baseUrl.replace(/\/v1\/?$/, '');
  return (
    <div className="usage-alert verify-alert" role="alert">
      <Icon name="user" size={15} />
      <div className="grow min0">
        <div className="strong small">Verify your account to get your full credit</div>
        <div className="tiny dim">Until you do, you get {Math.round(ver.unverifiedShare * 100)}% of the weekly desktop credit and free-model requests. Link Discord, Google or GitHub.</div>
      </div>
      <button type="button" className="button primary compact" onClick={() => openExternal(`${site}/account#verify`)}>Verify</button>
    </div>
  );
}
