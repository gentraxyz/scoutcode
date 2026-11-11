// Two small stores: `core`, what Rust last said, and `ui`, what's only about this window.
import { useSyncExternalStore } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke, type Item, type Model, type Snapshot, type Statics, type View } from './api';

function createStore<T>(initial: T) {
  let state = initial;
  const subs = new Set<() => void>();
  return {
    get: () => state,
    set(patch: Partial<T>) {
      state = { ...state, ...patch };
      subs.forEach((f) => f());
    },
    subscribe(f: () => void) {
      subs.add(f);
      return () => subs.delete(f);
    },
  };
}

export type CoreState = { view: View; models: Model[]; items: Item[]; statics: Statics };
export const core = createStore<CoreState>(null as unknown as CoreState);

export type DockTab = 'terminal' | 'changes' | 'files' | 'games' | 'review';
export type SettingsTab = 'appearance' | 'instructions' | 'memory' | 'dreaming' | 'notifications' | 'discord' | 'fallbacks' | 'router' | 'explore' | 'usage' | 'account' | 'providers' | 'skills' | 'mcp' | 'chatbots' | 'scoutmote' | 'experimental' | 'privacy' | 'updates';

export const ui = createStore({
  sidebar: true,
  /** The chat, or ScoutCode Delta (the issue tracker) in its place. */
  page: 'chat' as 'chat' | 'delta' | 'inbox',
  /** The first-run setup is showing: on first run, or reopened from Settings or the account menu. */
  onboarding: false,
  dockOpen: false,
  dockTab: 'terminal' as DockTab,
  /** A file the Files tab should open, from a double click in Changes or a file link in a reply. */
  openFile: null as string | null,
  /** The line to show in it, from a link like src/main.rs:42. */
  openLine: null as number | null,
  editorDirty: false,
  settings: null as SettingsTab | null,
  deleting: null as { id: string; title: string } | null,
  renaming: null as string | null,
  /** The folder picker is up, to add a project. */
  pickingFolder: false,
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
export async function connect() {
  let ready = false;
  const held: { seq: number; apply: () => void }[] = [];
  const on = <T extends { seq: number }>(name: string, apply: (payload: T) => void) =>
    listen<T>(name, ({ payload }) => {
      if (ready) apply(payload);
      else held.push({ seq: payload.seq, apply: () => apply(payload) });
    });
  await on<{ seq: number; data: Partial<View> }>('view', (p) => core.set({ view: { ...core.get().view, ...p.data } }));
  await on<{ seq: number; data: Model[] }>('models', (p) => core.set({ models: p.data }));
  await on<{ seq: number; reset: boolean; items?: Item[]; changes?: { at: number; item: Item }[] }>('items', (p) => {
    if (p.reset) return core.set({ items: p.items ?? [] });
    const items = core.get().items.slice();
    for (const c of p.changes ?? []) items[c.at] = c.item;
    core.set({ items });
  });
  const { seq, view, models, items, ...statics } = await invoke<Snapshot & { seq: number }>('snapshot');
  core.set({ view, models, items, statics });
  ready = true;
  for (const h of held) if (h.seq > seq) h.apply();
}
