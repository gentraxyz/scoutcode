// Someone who set up only their agent in onboarding is offered ScoutCode's steps (providers, project, options) the first
// time they open the Code tab, until they run them or dismiss the offer.
import { useSyncExternalStore } from 'react';
import { ScoutLogo } from './logo';
import { ui } from './store';
import { Icon } from './widgets';

const KEY = 'scout.codeSetup';
type State = 'pending' | 'done' | null;

const listeners = new Set<() => void>();
function read(): State {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'pending' || v === 'done' ? v : null;
  } catch {
    return null;
  }
}
function write(v: 'pending' | 'done') {
  try {
    localStorage.setItem(KEY, v);
  } catch {
    /* blocked: the offer just doesn't show */
  }
  listeners.forEach((f) => f());
}

/** Onboarding ended with only the agent set up: offer ScoutCode on the Code tab (unless it was set up before). */
export function markCodeSetupPending() {
  if (read() !== 'done') write('pending');
}

/** ScoutCode's steps were run, or the offer was dismissed. */
export function markCodeSetupDone() {
  write('done');
}

function useCodeSetupState(): State {
  return useSyncExternalStore((f) => { listeners.add(f); return () => listeners.delete(f); }, read);
}

export function CodeSetupPrompt() {
  if (useCodeSetupState() !== 'pending') return null;
  return (
    <div className="code-setup-prompt" role="status">
      <ScoutLogo size={30} />
      <div className="stack tight grow min0">
        <b className="strong">Finish setting up ScoutCode</b>
        <span className="small dim">Link free AI providers and pick a project folder so the coding agent can get to work.</span>
      </div>
      <button type="button" className="button ghost compact" onClick={markCodeSetupDone}>Not now</button>
      <button type="button" className="button primary compact" onClick={() => ui.set({ onboarding: true, onboardingMode: 'code' })}>
        Set up<Icon name="arrowRight" size={13} />
      </button>
    </div>
  );
}
