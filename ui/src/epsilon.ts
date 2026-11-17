// Epsilon, ScoutCode's cloud computers, in the desktop app. Everything goes through the ScoutCode site
// (`epsilon_cloud` in src/bridge.rs adds the account's key):
//  - /api/epsilon/machines: the account's two computers (persistent and temporary), shown in the sidebar and opened
//    in a frame of the Epsilon server's /embed page, which asks this window for short-lived tokens (/api/epsilon/token);
//  - /api/epsilon/tasks: cloud chats. Each is a repo task: the site clones a GitHub repository (linked through the
//    ScoutCode GitHub app) onto the temporary computer, an agent works there, and it ends in a pull request.
import { useEffect, useSyncExternalStore } from 'react';
import { invoke } from './api';
import { createStore, ui, useUi, useView } from './store';

export type VmKind = 'persistent' | 'temporary';
export type Machine = {
  kind: VmKind;
  state: 'none' | 'stopped' | 'starting' | 'running' | 'saving' | 'stopping' | 'error';
  type?: string; memoryMB?: number | null; rate?: number | null; savedAt?: string | null; expiresAt?: string | null; error?: string | null;
  desktop?: boolean; mode?: 'resumed' | 'fresh' | 'booted' | null; hasSavedState?: boolean; lowCredits?: boolean;
  /** A temporary computer kept (stopped, files only) after a cloud chat: until when, and for which task. */
  keptUntil?: string | null; tag?: string | null;
};
export type Machines = {
  machines: Record<VmKind, Machine>;
  types: { id: string; name: string; desktop: boolean; fastStart?: number[] }[];
  memory?: { memoryMB: number; rate: number }[];
  defaultType?: string; defaultMemoryMB?: number;
  credits: { unlimited: boolean; left: number; weekResetsAt: string };
  /** The Epsilon server: the computer's screen and terminal connect there. */
  server?: string;
};

export type TaskStatus = 'starting' | 'cloning' | 'working' | 'done' | 'failed' | 'stopped';
export const LIVE = new Set<TaskStatus>(['starting', 'cloning', 'working']);
export const STATUS: Record<TaskStatus, string> = { starting: 'Starting the computer…', cloning: 'Cloning the repository…', working: 'Working…', done: 'Done', failed: 'Failed', stopped: 'Stopped' };
export type TaskEvent = { at: number; kind: 'user' | 'step' | 'say' | 'tool' | 'error'; text: string; detail?: string; name?: string; ok?: boolean; summary?: string };
export type Task = {
  id: string; repoId: number; repo: string; branch: string; base: string; prompt: string; status: TaskStatus; memoryMB: number | null;
  pr: { number: number; url: string } | null; pushed: boolean; error: string | null; cost: number; createdAt: number; updatedAt: number; model?: string;
};
export type Repo = { id: number; name: string; private: boolean; account: string; defaultBranch: string };
export type Repos = { configured: boolean; slug: string | null; repos: Repo[]; errors: string[] };
export type CloudModel = { key: string; provider: string; id: string; label: string; description?: string };
export type CloudModels = { models: CloudModel[]; default: string | null };

/** A request to the site's Epsilon API. */
export const ep = <T>(method: 'GET' | 'POST', path: string, body?: unknown) => invoke<T>('epsilon_cloud', { method, path, body: body ?? null });

const REPO_KEY = 'scout:cloudRepo';
const MODEL_KEY = 'scout:cloudModel';
const DONE_KEY = 'scout:cloudDone';
const read = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch {} };

export const cloud = createStore({
  machines: null as Machines | null,
  machinesError: null as string | null,
  repos: null as Repos | null,
  tasks: null as Task[] | null,
  /** The open task, with its log. */
  task: null as (Task & { events: TaskEvent[] }) | null,
  models: null as CloudModels | null,
  /** The repository a new cloud chat goes to, last picked. */
  repoId: Number(read(REPO_KEY)) || null,
  /** The model picked for cloud chats, or null for the server's default. */
  model: read(MODEL_KEY),
  /** Cloud chats marked done here (the server has no such mark). */
  done: (() => { try { return new Set<string>(JSON.parse(read(DONE_KEY) ?? '[]')); } catch { return new Set<string>(); } })(),
});

export function useCloud<T>(pick: (s: ReturnType<typeof cloud.get>) => T): T {
  return useSyncExternalStore(cloud.subscribe, () => pick(cloud.get()));
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Computers being stopped from here: until the stop request answers, a poll that still says "running" is older news. */
const stopping = new Map<VmKind, Machine['state']>();
export const loadMachines = () =>
  ep<Machines>('GET', '/api/epsilon/machines').then(
    (machines) => {
      for (const [kind, state] of stopping) {
        const m = machines.machines[kind];
        if (m?.state === 'running') machines.machines[kind] = { ...m, state };
      }
      cloud.set({ machines, machinesError: null });
    },
    (e) => cloud.set({ machinesError: message(e) }),
  );

/** Changes what's known of one computer here (from the terminal socket's news, or before the server answers). */
export function setMachine(kind: VmKind, m: Partial<Machine>) {
  const info = cloud.get().machines;
  if (!info) return;
  cloud.set({ machines: { ...info, machines: { ...info.machines, [kind]: { ...info.machines[kind], ...m, kind } } } });
}

/**
 * Saves (persistent) or wipes (temporary) a computer. It shows as saving or wiping at once, so its screen lets go
 * straight away, and polling carries on from there. Saving can outlast the request (a proxy may give up on it first):
 * that's only an error if the computer is still running afterwards.
 */
export async function stopMachine(kind: VmKind): Promise<void> {
  const state = kind === 'temporary' ? 'stopping' : 'saving';
  stopping.set(kind, state);
  setMachine(kind, { state });
  try {
    await ep('POST', '/api/epsilon/machines', { kind, action: 'stop' });
  } catch (e) {
    stopping.delete(kind);
    await loadMachines();
    if (cloud.get().machines?.machines[kind]?.state === 'running') throw e;
    return;
  }
  stopping.delete(kind);
  await loadMachines();
}
export const loadTasks = () => ep<{ tasks: Task[] }>('GET', '/api/epsilon/tasks').then((r) => cloud.set({ tasks: r.tasks }), () => cloud.set({ tasks: cloud.get().tasks ?? [] }));
export const loadRepos = (fresh = false) =>
  ep<Repos>('GET', `/api/epsilon/tasks?repos=1${fresh ? '&fresh=1' : ''}`).then(
    (repos) => cloud.set({ repos }),
    (e) => cloud.set({ repos: { configured: true, slug: null, repos: [], errors: [message(e)] } }),
  );
export const loadModels = () => ep<CloudModels>('GET', '/api/epsilon/tasks?models=1').then((models) => cloud.set({ models }), () => cloud.set({ models: cloud.get().models ?? { models: [], default: null } }));
export const loadTask = (id: string) => ep<{ task: Task & { events: TaskEvent[] } }>('GET', `/api/epsilon/tasks?id=${encodeURIComponent(id)}`).then((r) => cloud.set({ task: r.task }));

export function setRepo(id: number) {
  write(REPO_KEY, String(id));
  cloud.set({ repoId: id });
}
export function setModel(key: string) {
  write(MODEL_KEY, key);
  cloud.set({ model: key });
}
export function setCloudDone(id: string, on: boolean) {
  const done = new Set(cloud.get().done);
  if (on) done.add(id);
  else done.delete(id);
  write(DONE_KEY, JSON.stringify([...done]));
  cloud.set({ done });
}

/** Opens a new cloud chat (in `repoId`, or the last one picked) in place of the local chat. */
export function newCloudChat(repoId?: number) {
  if (repoId) setRepo(repoId);
  ui.set({ page: 'chat', cloud: { repoId: repoId ?? cloud.get().repoId, taskId: null } });
}
export function openCloudTask(id: string) {
  const t = cloud.get().task;
  if (t?.id !== id) cloud.set({ task: null });
  ui.set({ page: 'chat', cloud: { repoId: null, taskId: id } });
}
/** Back to the local chat. */
export const leaveCloud = () => ui.set({ cloud: null });

/** Starts a cloud chat with its first message. */
export async function createTask(repoId: number, prompt: string): Promise<void> {
  const model = cloud.get().model ?? undefined;
  const r = await ep<{ task: Task & { events: TaskEvent[] } }>('POST', '/api/epsilon/tasks', { action: 'create', repoId, prompt, model });
  cloud.set({ task: r.task, tasks: [r.task, ...(cloud.get().tasks ?? []).filter((t) => t.id !== r.task.id)] });
  ui.set({ cloud: { repoId: null, taskId: r.task.id } });
}
/** Carries on a finished cloud chat, from its branch. */
export async function replyTask(id: string, prompt: string): Promise<void> {
  const model = cloud.get().model ?? cloud.get().task?.model ?? undefined;
  const r = await ep<{ task: Task & { events: TaskEvent[] } }>('POST', '/api/epsilon/tasks', { action: 'reply', id, prompt, model });
  cloud.set({ task: r.task });
  setCloudDone(id, false);
  loadTasks();
}
export const stopTask = (id: string) => ep('POST', '/api/epsilon/tasks', { action: 'stop', id }).finally(() => loadTask(id));

/** The site's address, for links to its pages (linking GitHub). */
export function useSite(): string {
  return useView('cfg').baseUrl.replace(/\/+$/, '').replace(/\/v1$/, '');
}

/**
 * Keeps Epsilon's lists fresh while signed in: tasks (often while one runs), the computers, and once each the
 * repositories and models. The open cloud chat is reloaded every 2 seconds while it runs.
 */
export function useEpsilonSync() {
  const { hasKey } = useView('cfg');
  const live = useCloud((s) => !!s.tasks?.some((t) => LIVE.has(t.status)));
  const busyVm = useCloud((s) => !!s.machines && Object.values(s.machines.machines).some((m) => m && ['starting', 'saving', 'stopping'].includes(m.state)));
  const openId = useUi((u) => u.cloud?.taskId ?? null);
  const openLive = useCloud((s) => !!s.task && s.task.id === openId && LIVE.has(s.task.status));
  useEffect(() => {
    if (!hasKey) return;
    if (!cloud.get().repos) loadRepos();
    if (!cloud.get().models) loadModels();
  }, [hasKey]);
  useEffect(() => {
    if (!hasKey) return cloud.set({ tasks: null, machines: null });
    loadTasks();
    const t = setInterval(() => !document.hidden && loadTasks(), live ? 3000 : 20000);
    return () => clearInterval(t);
  }, [hasKey, live]);
  useEffect(() => {
    if (!hasKey) return;
    loadMachines();
    const t = setInterval(() => !document.hidden && loadMachines(), busyVm ? 3000 : 20000);
    return () => clearInterval(t);
  }, [hasKey, busyVm]);
  useEffect(() => {
    if (!hasKey || !openId) return;
    if (cloud.get().task?.id !== openId) loadTask(openId).catch(() => {});
    if (!openLive) return;
    const t = setInterval(() => !document.hidden && loadTask(openId).catch(() => {}), 2000);
    return () => clearInterval(t);
  }, [hasKey, openId, openLive]);
}

export const repoShort = (name: string) => name.split('/').pop() || name;
export const KIND_NAME: Record<VmKind, string> = { persistent: 'Persistent computer', temporary: 'Temporary computer' };

export function vmStateText(m: Machine | null | undefined): string {
  switch (m?.state) {
    case 'running': return 'Running';
    case 'starting': return 'Starting…';
    case 'saving': return 'Saving…';
    case 'stopping': return 'Stopping…';
    case 'error': return 'Couldn’t start';
    case 'stopped': return m.keptUntil ? 'Kept from a cloud chat' : 'Stopped';
    default: return 'Not set up';
  }
}
