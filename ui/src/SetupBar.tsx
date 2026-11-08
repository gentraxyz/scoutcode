import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { SetupStatus } from './ClaudeSetup';

/**
 * Scout's bar over the browser while it adds the Claude or Gemini connector itself (claude_native.rs): what it's doing, and
 * Pause. Paused, it says why and offers Resume and Stop. It's its own always-on-top window, `setup-bar`.
 */
export function SetupBar() {
  const [s, setS] = useState<SetupStatus>({ state: 'running' });

  useEffect(() => {
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    let on = true;
    const tick = () => invoke<SetupStatus>('claude_setup_status').then((x) => on && setS(x), () => {});
    tick();
    const t = setInterval(tick, 300);
    return () => { on = false; clearInterval(t); };
  }, []);

  const pause = (paused: boolean) => invoke<SetupStatus>('claude_setup_pause', { paused }).then(setS, () => {});
  const stop = () => invoke<SetupStatus>('claude_setup_stop').then(setS, () => {});

  const what =
    s.state === 'done' ? (/completely ready/i.test(s.label ?? '') ? 'All set. Gemini is completely ready: ask it to work on your project.' : /^already/i.test(s.label ?? '') ? 'Already added.' : /^Added to (Claude|Gemini)$/.test(s.label ?? '') ? 'Added. Finish in the ScoutCode window: sign in if it asks, then Link.' : 'Added. Click Connect next to Scout to finish.')
    : s.state === 'failed' ? s.error || 'The setup stopped.'
    : s.state === 'stopped' ? 'Stopped.'
    : s.paused ? s.note || 'Paused.'
    : s.label || 'Starting…';
  const dot = s.state === 'failed' ? 'red' : s.state === 'done' ? 'green' : s.paused ? 'amber' : 'live';

  return (
    <div className="setup-bar">
      <span className={`setup-dot ${dot}`} />
      <div className="setup-what">
        <span>Scout · Adding to {s.bot === 'gemini' ? 'Gemini' : 'Claude'}{s.paused ? ' · Paused' : s.state === 'running' ? ' · Esc to take over' : ''}</span>
        <b title={what}>{what}</b>
      </div>
      {s.state === 'running' && (s.paused
        ? <><button type="button" className="button compact" onClick={stop}>Stop</button><button type="button" className="button compact primary" onClick={() => pause(false)}>Resume</button></>
        : <button type="button" className="button compact" onClick={() => pause(true)}>Pause</button>)}
    </div>
  );
}
