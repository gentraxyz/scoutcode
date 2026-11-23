import { useSyncExternalStore } from 'react';

/** The model menu's rail: the providers (and pages) it shows down its left side, in the order the user put them. */
export type Tab = 'router' | 'scoutcode' | 'claude' | 'codex' | 'grok' | 'openrouter' | 'nvidia' | 'opencode' | 'kilo' | 'copilot' | 'devin' | 'kiro' | 'antigravity' | 'vibe' | 'hermes' | 'local' | 'additional' | 'chatbots';

// Claude and Grok are also chatbots: their pages hold both the chatbot and its models.
export const TABS: [Tab, string][] = [
  ['router', 'Agent Router'], ['codex', 'Codex'], ['claude', 'Claude'], ['grok', 'Grok'], ['chatbots', 'Other chatbots'],
  ['scoutcode', 'ScoutCode'], ['openrouter', 'OpenRouter'], ['nvidia', 'NVIDIA NIM'], ['opencode', 'OpenCode'], ['kilo', 'Kilo Code'],
  ['copilot', 'GitHub Copilot'], ['devin', 'Devin'], ['kiro', 'Kiro'], ['antigravity', 'Antigravity'], ['vibe', 'Mistral Vibe'], ['hermes', 'Hermes'], ['local', 'Local models'], ['additional', 'Additional Providers'],
];

export const tabName = (t: Tab): string => TABS.find(([id]) => id === t)?.[1] ?? t;
export const isTab = (t: unknown): t is Tab => TABS.some(([id]) => id === t);

/** What onboarding starts a new user's rail with. They can add or remove the rest there. */
export const DEFAULT_RAIL: Tab[] = ['router', 'scoutcode', 'kilo', 'opencode', 'claude', 'codex'];

const KEY = 'scout.railOrder';
/** Before the rail could be ordered, providers were hidden from the full list instead. */
const LEGACY_HIDDEN = 'scout.hiddenProviders';

function read(key: string): unknown {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null');
  } catch {
    return null;
  }
}

function load(): Tab[] {
  const saved = read(KEY);
  if (Array.isArray(saved)) return [...new Set(saved.filter(isTab))];
  const hidden = read(LEGACY_HIDDEN);
  return TABS.map(([t]) => t).filter((t) => !(Array.isArray(hidden) && hidden.includes(t)));
}

let rail = load();
const subs = new Set<() => void>();

function save(next: Tab[]) {
  rail = [...new Set(next)];
  try {
    localStorage.setItem(KEY, JSON.stringify(rail));
  } catch {
    // Kept for this run only.
  }
}

export function setRail(next: Tab[]) {
  save(next);
  subs.forEach((f) => f());
}

/** Whether this PC has a rail of its own yet (onboarding seeds one if not). */
export function hasSavedRail(): boolean {
  try {
    return localStorage.getItem(KEY) !== null;
  } catch {
    return false;
  }
}

/** Sets the starting rail before anything that shows it has rendered, so nothing animates out. */
export function seedRail(next: Tab[]) {
  save(next);
}

export function useRail(): Tab[] {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => rail,
  );
}
