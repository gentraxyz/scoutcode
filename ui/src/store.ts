// Two small stores: `core`, what Rust last said, and `ui`, what's only about this window.
import { useSyncExternalStore } from 'react';
import { flushSync } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { invoke, type AdditionalProvider, type Item, type Model, type Snapshot, type Statics, type View } from './api';

const reducedMotion = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : null;

/** `morph` says when a change rearranges the window (a page, a panel, a dialog): the old look then cross-fades into the
 *  new one through a view transition, where the WebView has them, instead of snapping. The state itself changes at once. */
export function createStore<T>(initial: T, morph?: (prev: T, patch: Partial<T>) => boolean) {
  let state = initial;
  const subs = new Set<() => void>();
  const notify = () => subs.forEach((f) => f());
  return {
    get: () => state,
    set(patch: Partial<T>) {
      const prev = state;
      state = { ...state, ...patch };
      if (morph && document.startViewTransition && !reducedMotion?.matches && !document.hidden && morph(prev, patch)) {
        document.startViewTransition(() => flushSync(notify));
      } else {
        notify();
      }
    },
    subscribe(f: () => void) {
      subs.add(f);
      return () => subs.delete(f);
    },
  };
}

export type CoreState = { view: View; models: Model[]; items: Item[]; statics: Statics };
export const core = createStore<CoreState>(null as unknown as CoreState);

/** A tab in the panel: one of its pages, the New Tab page, a sub-agent's work (by its tool item's id) or a background shell. */
export type DockTab = 'terminal' | `terminal:${number}` | 'changes' | 'files' | 'games' | 'review' | 'new' | `new:${number}` | `task:${number}` | `shell:${string}`;
export type SettingsTab = 'appearance' | 'preferences' | 'agents' | 'memory' | 'providers' | 'router' | 'images' | 'skills' | 'mcp' | 'chatbots' | 'account' | 'updates' | 'logs';

export const ui = createStore({
  sidebar: true,
  /** Agent (the home page: persistent agents, Agent.tsx), the Code agent ('code', CodeAgent.tsx), one of its Code Agents
   *  (the coding chats, 'chat'), or another page. */
  page: 'agent' as 'agent' | 'code' | 'chat' | 'mission' | 'social',
  /** The page Social was opened from, for its Back button. */
  socialFrom: 'agent' as 'agent' | 'code' | 'chat' | 'mission',
  /** The Usage popup is up (over whatever page is open, like Settings). */
  usageOpen: false,
  /** The Scout Season popup is up (Season.tsx). */
  seasonOpen: false,
  /** The first-run setup is showing: on first run, or reopened from Settings. */
  onboarding: false,
  /** Which setup: all of it, only signing in and meeting an agent (from the Agent page), or only ScoutCode's steps (from the Code tab). */
  onboardingMode: 'full' as 'full' | 'agent' | 'code',
  dockOpen: false,
  dockTab: 'new' as DockTab,
  /** The panel's open tabs, in the strip's order. */
  dockTabs: [] as DockTab[],
  /** The folder each terminal tab's shell is in, for its title. */
  termFolders: {} as Record<string, string>,
  /** Restart to update was pressed: the window shows that it's installing until it closes. */
  restarting: false,
  /** A file the Files tab should open, from a double click in Changes or a file link in a reply. */
  openFile: null as string | null,
  /** The line to show in it, from a link like src/main.rs:42. */
  openLine: null as number | null,
  editorDirty: false,
  settings: null as SettingsTab | null,
  deleting: null as { id: string; title: string } | null,
  /** The window was asked to close and "Close Scout?" is up. */
  confirmingClose: false,
  /** A model marked uncensored was picked before the account could use it: the popup asks, then `pick` picks it. */
  uncensored: null as { name: string; pick: () => void } | null,
  renaming: null as string | null,
  /** The folder picker is up, to add a project. */
  pickingFolder: false,
  /** The same picker is up, to attach files to the message. */
  pickingFiles: false,
  input: '',
  attachments: [] as string[],
  focusEpoch: 0,
  findOpen: false,
  findEpoch: 0,
  dropping: false,
  /** The floating game card is up. In the panel, the game is open when the panel shows its tab. */
  gameOpen: false,
  /** The floating music player card is up. Closing it keeps music playing in the background. */
  musicOpen: false,
}, (prev, patch) => {
  // Opening and closing count; moving between tabs inside an open panel doesn't (Settings animates its own pages).
  const flips = (k: 'settings' | 'deleting') => k in patch && !prev[k] !== !patch[k];
  return ('page' in patch && patch.page !== prev.page) || ('sidebar' in patch && patch.sidebar !== prev.sidebar)
    || ('dockOpen' in patch && patch.dockOpen !== prev.dockOpen) || ('pickingFolder' in patch && patch.pickingFolder !== prev.pickingFolder) || ('pickingFiles' in patch && patch.pickingFiles !== prev.pickingFiles)
    || flips('settings') || flips('deleting');
});

export function useCore<T>(pick: (s: CoreState) => T): T {
  return useSyncExternalStore(core.subscribe, () => pick(core.get()));
}

export function useView<K extends keyof View>(key: K): View[K] {
  return useSyncExternalStore(core.subscribe, () => core.get().view[key]);
}

export function useUi<T>(pick: (s: ReturnType<typeof ui.get>) => T): T {
  return useSyncExternalStore(ui.subscribe, () => pick(ui.get()));
}

export function focusComposer() {
  ui.set({ focusEpoch: ui.get().focusEpoch + 1 });
}

/**
 * Starts listening, then reads where things stand. What arrives before the snapshot is held back, and only what's newer
 * than the snapshot is then applied: Rust sends each change once, so one lost here would stay lost.
 */
/** The built-in additional providers, as the snapshot gave them: the user's own are added to them from each new view. */
let builtIn: AdditionalProvider[] = [];

function withCustom(statics: Statics, view: View): Statics {
  const custom = view.cfg?.customProviders ?? [];
  const additionalProviders = [
    ...builtIn,
    ...custom.map((c): AdditionalProvider => ({ id: c.id, name: c.name, needsKey: false, note: `${c.kind === 'anthropic' ? 'Anthropic API' : 'OpenAI-compatible API'} at ${c.url}`, keyUrl: '', keyLink: '', keyHint: 'API key', custom: c })),
  ];
  return { ...statics, additionalProviders };
}

export async function connect() {
  let ready = false;
  const held: { seq: number; apply: () => void }[] = [];
  const on = <T extends { seq: number }>(name: string, apply: (payload: T) => void) =>
    listen<T>(name, ({ payload }) => {
      if (ready) apply(payload);
      else held.push({ seq: payload.seq, apply: () => apply(payload) });
    });
  await on<{ seq: number; data: Partial<View> }>('view', (p) => {
    const view = { ...core.get().view, ...p.data };
    core.set(p.data.cfg ? { view, statics: withCustom(core.get().statics, view) } : { view });
  });
  await on<{ seq: number; data: Model[] }>('models', (p) => core.set({ models: p.data }));
  await on<{ seq: number; reset: boolean; items?: Item[]; changes?: { at: number; item: Item }[] }>('items', (p) => {
    if (p.reset) return core.set({ items: p.items ?? [] });
    const items = core.get().items.slice();
    for (const c of p.changes ?? []) items[c.at] = c.item;
    core.set({ items });
  });
  const { seq, view, models, items, ...statics } = await invoke<Snapshot & { seq: number }>('snapshot');
  builtIn = statics.additionalProviders;
  core.set({ view, models, items, statics: withCustom(statics as Statics, view) });
  ready = true;
  for (const h of held) if (h.seq > seq) h.apply();
}
