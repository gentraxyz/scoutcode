import { useSyncExternalStore } from 'react';
import type { Doc } from './model';
import type { Route } from './Delta';
import { cycleName, issueKey, newId, pageTitle } from './model';

export interface DeltaTabItem {
  id: string;
  route: Route;
  openId: string | null;
}

export interface DeltaTabsState {
  tabs: DeltaTabItem[];
  activeId: string;
  workspaceKey: string | null;
  doc: Doc | null;
  defaultRoute: Route;
}

const STORAGE_PREFIX = 'delta.desktop.tabs.';

function getInitialState(): DeltaTabsState {
  const initialId = 'tab_default';
  const defaultRoute: Route = { view: 'my' };
  return {
    tabs: [{ id: initialId, route: defaultRoute, openId: null }],
    activeId: initialId,
    workspaceKey: null,
    doc: null,
    defaultRoute,
  };
}

let state: DeltaTabsState = getInitialState();
const subs = new Set<() => void>();
/** An issue to open once its workspace loads, as the Inbox asks for. */
let pendingOpen: { workspaceKey: string; issueId: string } | null = null;

function notify() {
  subs.forEach((f) => f());
}

function saveTabs() {
  if (!state.workspaceKey) return;
  try {
    const data = {
      activeId: state.activeId,
      tabs: state.tabs.map((t) => ({ id: t.id, route: t.route, openId: t.openId })),
    };
    localStorage.setItem(STORAGE_PREFIX + state.workspaceKey, JSON.stringify(data));
  } catch {}
}

export const deltaTabs = {
  get: () => state,
  subscribe: (f: () => void) => {
    subs.add(f);
    return () => subs.delete(f);
  },

  /** Opens an issue in the active tab the next time its workspace loads (or now, if it's the one loaded). */
  openOnLoad(workspaceKey: string, issueId: string) {
    pendingOpen = { workspaceKey, issueId };
  },

  init(workspaceKey: string, doc: Doc, defaultRoute: Route) {
    if (state.workspaceKey === workspaceKey && state.doc === doc) {
      if (pendingOpen?.workspaceKey === workspaceKey) {
        const id = pendingOpen.issueId;
        pendingOpen = null;
        this.setOpenId(id);
      }
      return;
    }

    let restoredTabs: DeltaTabItem[] | null = null;
    let restoredActiveId: string | null = null;

    try {
      const raw = localStorage.getItem(STORAGE_PREFIX + workspaceKey);
      if (raw) {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed.tabs) && parsed.tabs.length > 0) {
          restoredTabs = parsed.tabs;
          restoredActiveId = parsed.activeId ?? parsed.tabs[0].id;
        }
      }
    } catch {}

    const initialTabs = restoredTabs && restoredTabs.length > 0
      ? restoredTabs
      : [{ id: newId(), route: defaultRoute, openId: null }];
    const initialActiveId = (restoredActiveId && initialTabs.some((t) => t.id === restoredActiveId))
      ? restoredActiveId
      : initialTabs[0].id;
    let tabs = initialTabs;
    if (pendingOpen?.workspaceKey === workspaceKey) {
      const id = pendingOpen.issueId;
      tabs = tabs.map((t) => (t.id === initialActiveId ? { ...t, openId: id } : t));
    }
    pendingOpen = null;

    state = {
      ...state,
      tabs,
      activeId: initialActiveId,
      workspaceKey,
      doc,
      defaultRoute,
    };
    saveTabs();
    notify();
  },

  setDoc(doc: Doc) {
    if (state.doc === doc) return;
    state = { ...state, doc };
    notify();
  },

  updateActiveTab(patch: Partial<Omit<DeltaTabItem, 'id'>>) {
    const active = state.tabs.find((t) => t.id === state.activeId);
    if (!active) return;
    const nextTabs = state.tabs.map((t) => (t.id === state.activeId ? { ...t, ...patch } : t));
    state = { ...state, tabs: nextTabs };
    saveTabs();
    notify();
  },

  setRoute(route: Route) {
    this.updateActiveTab({ route, openId: null });
  },

  setOpenId(openId: string | null) {
    this.updateActiveTab({ openId });
  },

  addTab(route?: Route, openId?: string | null) {
    const id = newId();
    const r = route ?? state.defaultRoute;
    const newTab: DeltaTabItem = { id, route: r, openId: openId ?? null };
    state = {
      ...state,
      tabs: [...state.tabs, newTab],
      activeId: id,
    };
    saveTabs();
    notify();
    return id;
  },

  closeTab(id: string) {
    const idx = state.tabs.findIndex((t) => t.id === id);
    if (idx === -1) return;

    if (state.tabs.length <= 1) {
      // Don't remove the only tab; reset it to the default view.
      state = {
        ...state,
        tabs: [{ id: state.tabs[0].id, route: state.defaultRoute, openId: null }],
        activeId: state.tabs[0].id,
      };
      saveTabs();
      notify();
      return;
    }

    const nextTabs = state.tabs.filter((t) => t.id !== id);
    let nextActiveId = state.activeId;
    if (state.activeId === id) {
      const nextIdx = Math.min(idx, nextTabs.length - 1);
      nextActiveId = nextTabs[nextIdx].id;
    }

    state = {
      ...state,
      tabs: nextTabs,
      activeId: nextActiveId,
    };
    saveTabs();
    notify();
  },

  closeActiveTab() {
    this.closeTab(state.activeId);
  },

  switchTab(id: string) {
    if (state.activeId === id) return;
    if (!state.tabs.some((t) => t.id === id)) return;
    state = { ...state, activeId: id };
    saveTabs();
    notify();
  },

  nextTab() {
    if (state.tabs.length <= 1) return;
    const idx = state.tabs.findIndex((t) => t.id === state.activeId);
    const nextIdx = (idx + 1) % state.tabs.length;
    this.switchTab(state.tabs[nextIdx].id);
  },

  prevTab() {
    if (state.tabs.length <= 1) return;
    const idx = state.tabs.findIndex((t) => t.id === state.activeId);
    const prevIdx = (idx - 1 + state.tabs.length) % state.tabs.length;
    this.switchTab(state.tabs[prevIdx].id);
  },

  switchToIndex(idx: number) {
    if (idx >= 0 && idx < state.tabs.length) {
      this.switchTab(state.tabs[idx].id);
    }
  },
};

export function useDeltaTabs<T>(pick: (s: DeltaTabsState) => T): T {
  return useSyncExternalStore(deltaTabs.subscribe, () => pick(deltaTabs.get()));
}

export function getTabInfo(tab: DeltaTabItem, doc: Doc | null): { title: string; icon: string } {
  if (tab.openId && doc?.issue[tab.openId]) {
    const i = doc.issue[tab.openId];
    const key = issueKey(doc, i);
    return { title: `${key} · ${i.title}`, icon: 'issues' };
  }

  const r = tab.route;
  if (r.view === 'my') {
    const sub = r.tab && r.tab !== 'all' ? ` (${r.tab})` : '';
    return { title: `My issues${sub}`, icon: 'inbox' };
  }
  if (r.view === 'team') {
    const team = doc?.team[r.teamId];
    const sub = r.tab && r.tab !== 'all' ? ` · ${r.tab}` : '';
    return { title: team ? `${team.name}${sub}` : 'Issues', icon: 'issues' };
  }
  if (r.view === 'cycles') {
    const team = doc?.team[r.teamId];
    return { title: team ? `${team.name} cycles` : 'Cycles', icon: 'cycle' };
  }
  if (r.view === 'cycle') {
    const cycle = doc?.cycle[r.id];
    return { title: cycle ? cycleName(cycle) : 'Cycle', icon: 'cycle' };
  }
  if (r.view === 'projects') {
    return { title: 'Projects', icon: 'project' };
  }
  if (r.view === 'project') {
    const project = doc?.project[r.id];
    return { title: project?.name ?? 'Project', icon: 'project' };
  }
  if (r.view === 'roadmap') {
    return { title: 'Roadmap', icon: 'roadmap' };
  }
  if (r.view === 'docs') {
    return { title: 'Docs', icon: 'doc' };
  }
  if (r.view === 'doc') {
    const page = doc?.page[r.id];
    return { title: page ? (pageTitle(page) || 'Untitled') : 'Doc', icon: 'doc' };
  }
  if (r.view === 'settings') {
    return { title: 'Settings', icon: 'gear' };
  }

  return { title: 'Delta', icon: 'issues' };
}
