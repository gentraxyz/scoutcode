// Agent, the home page: the account's persistent agents, kept on the ScoutCode site (src/lib/agent-bot.ts there) and
// reached through `agent_cloud` in src/bridge.rs. The list and recent messages are also kept on this PC
// (see "kept on this PC") so the page shows at once; the site's answers replace them.
//
// While the page is open, the list and the open chat are long-polled: each request names the `rev` it last saw and the
// site holds it until something changes, so replies show up the moment they're written. A site without long-polling
// (no `rev` in its answers) is polled instead. Elsewhere only the list is fetched, slowly, for the rail's unread badge.
//
// A new message from an agent shows a Windows notification while the window is in the background (Rust's agent_alert),
// and the unread count is the badge in the corner of the taskbar button.
//
// Sending is optimistic: the message shows at once and the agent starts "typing"; it's only marked if it failed.
import { useEffect, useLayoutEffect, useSyncExternalStore } from 'react';
import { act, invoke } from '../api';
import { createStore, ui, useUi, useView } from '../store';

/** 'code' is the built-in Code agent's own shape: it's not in AGENT_SHAPES, so no other agent can pick it. */
export type Shape = 'blob' | 'circle' | 'square' | 'triangle' | 'hexagon' | 'star' | 'diamond' | 'ghost' | 'code';
export type Status = 'idle' | 'thinking' | 'working' | 'waiting' | 'sleeping' | 'blocked' | 'error';

export type Agent = {
  id: string;
  name: string;
  title: string;
  shape: Shape;
  color: string;
  instructions: string;
  status: Status;
  activity: string | null;
  activityKind: string | null;
  wakeAt: string | null;
  mcp: App[];
  /** How many timers, alarms and recurring tasks it has. */
  schedules?: number;
  unread: number;
  last: { role: string; text: string; at: string } | null;
  createdAt: string;
};

/** A connected app (an MCP server). `needs`: what the user still has to type in (an access token, API keys). */
export type App = { id: string; name: string; url: string; local?: boolean; connected: boolean; needs?: string[] };
/** Who a connection is for in Connect to Apps: one agent (its id), or all of them. */
export type AppTarget = 'all' | string;
export type SecretField = { key: string; label: string; hint?: string; optional?: boolean };
/** What connecting led to: ready, a sign-in page (opened in the browser), or values to type in. */
export type ConnectAnswer = ({ kind: 'connected'; text: string } | { kind: 'signin'; url: string } | { kind: 'secret'; fields: SecretField[] }) & { app?: App };

export type Card =
  /** An action the agent marked risky (its `reason` is the card's text), held until the user answers. */
  | { kind: 'approval'; action: string; details?: string | null; reason?: string | null; risky?: boolean; answer?: 'approve' | 'deny' }
  /** A command held for the user's OK: on the agent's computer or this PC. `setup`: the first one, which also sets
   *  how commands are approved from now on. `risky`: the agent marked it, with a `reason`. */
  | { kind: 'command'; command: string; where: 'computer' | 'pc'; reason?: string | null; risky?: boolean; setup?: boolean; answer?: 'approve' | 'deny'; always?: boolean }
  /** What an app needs to connect (a token, API keys), typed here and sent straight to the server's settings. */
  | { kind: 'secret'; server: string; name: string; fields: { key: string; label: string; hint?: string; optional?: boolean }[]; answer?: string }
  /** A timer, alarm or recurring task going off. */
  | { kind: 'alarm'; schedule: string; type: 'timer' | 'alarm' | 'repeat'; task?: string | null }
  | { kind: 'local'; command: string; answer?: 'approve' | 'deny' }
  | { kind: 'permission'; setting: 'localAccess' }
  | { kind: 'auth'; name: string; url: string; answer?: string }
  | { kind: 'open'; url: string }
  | { kind: 'notice' }
  /** Something the agent did with ScoutCode (its text says what). Shown between the messages, not as a card. */
  | { kind: 'activity'; tool?: string };

export type Message = {
  id: number;
  role: 'user' | 'agent' | 'card';
  text: string;
  data: (Card & { files?: undefined }) | { files: AgentFile[] } | null;
  created_at: string;
  /** Only on this PC: a message still on its way, or one that didn't make it (with why). */
  local?: { state: 'sending' | 'failed'; error?: string };
};

export type AgentFile = { id: string; name: string; type: string; size: number; created_at?: string };
/** null: not chosen yet (the first command asks). 'ask': every command. 'auto': only the ones the agent marks risky. */
export type CommandMode = 'ask' | 'auto' | null;
export type Prefs = { localAccess: boolean; commandMode?: CommandMode; pcMode?: CommandMode; timezone?: string | null };

export type Schedule = {
  id: string;
  kind: 'timer' | 'alarm' | 'repeat';
  label: string;
  task: string;
  tz: string;
  ring: boolean;
  nextAt: string | null;
  lastAt: string | null;
  runs: number;
  paused: boolean;
  /** "Every weekday at 09:00", "Timer", "Alarm". */
  summary: string;
};
export type NewSchedule = {
  type: Schedule['kind'];
  label: string;
  task?: string;
  minutes?: number;
  at?: string;
  every?: 'minutes' | 'hours' | 'days' | 'weeks' | 'months';
  interval?: number;
  time?: string;
  days?: string[];
  day_of_month?: number;
  ring?: boolean;
  timezone?: string;
};
export type Look = Pick<Agent, 'name' | 'title' | 'shape' | 'color' | 'instructions'>;
export type MemoryLog = { at: string; by: string; verb: string; text: string };
export type Memory = { text: string; log: MemoryLog[]; updatedAt: string | null; limit: number; used: number };

/** `more`: older messages are left on the site (loadOlder). `older`: they're being fetched now. */
type Chat = { messages: Message[]; files: AgentFile[]; error?: string | null; more?: boolean; older?: boolean };

export const agentStore = createStore({
  list: null as Agent[] | null,
  error: null as string | null,
  max: 10,
  prefs: { localAccess: false, commandMode: null, pcMode: null } as Prefs,
  /** The schedules panel is open for this agent. */
  schedulesFor: null as string | null,
  model: null as string | null,
  openrouter: false,
  selected: null as string | null,
  /** The newest message id of each chat the user has seen on screen (window in front, on this page, chat open). Loading
   *  a chat marks it read on the site even with the window in the background, so what came after this still counts as
   *  unread here. */
  viewed: {} as Record<string, number>,
  chats: {} as Record<string, Chat>,
  /** The create/edit dialog: a new agent, or the id of the one being edited. */
  editing: null as 'new' | string | null,
  /** The account's apps, which every agent can use (Connect to Apps). */
  apps: [] as App[],
  /** Connect to Apps is open, showing all agents or one. */
  appsFor: null as AppTarget | null,
});

export function useAgents<T>(pick: (s: ReturnType<typeof agentStore.get>) => T): T {
  return useSyncExternalStore(agentStore.subscribe, () => pick(agentStore.get()));
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const call = <T>(method: 'GET' | 'POST', path: string, body?: unknown) => invoke<T>('agent_cloud', { method, path, body: body ?? null });
const PATH = '/api/account/agents';
const post = <T = { ok: true }>(action: string, body: Record<string, unknown> = {}) => call<T>('POST', PATH, { action, ...body });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- merging

/** The server's messages merged into what's shown, by id, plus the ones still on their way from this PC. `whole`: the
 *  incoming page is everything from its oldest message on, so shown ones in that range it lacks are gone (older pages
 *  loaded by scrolling up stay). */
function merge(cur: Message[], incoming: Message[], whole: boolean): Message[] {
  const byId = new Map<number, Message>();
  const from = whole ? (incoming.length ? incoming[0].id : Infinity) : Infinity;
  for (const m of cur) if (m.id > 0 && m.id < from) byId.set(m.id, m);
  for (const m of incoming) byId.set(m.id, m);
  const real = [...byId.values()].sort((a, b) => a.id - b.id);
  // A local message whose real copy has arrived (same text, sent after it was typed) is done.
  const local = cur.filter((m) => m.id < 0 && !real.some((r) => r.role === 'user' && r.text === m.text && r.created_at >= new Date(-m.id - 60_000).toISOString()));
  return [...real, ...local];
}

const lastRealId = (c?: Chat) => {
  for (let i = (c?.messages.length ?? 0) - 1; i >= 0; i--) if (c!.messages[i].id > 0) return c!.messages[i].id;
  return 0;
};

function setChat(id: string, f: (c: Chat) => Chat) {
  const chats = agentStore.get().chats;
  agentStore.set({ chats: { ...chats, [id]: f(chats[id] ?? { messages: [], files: [] }) } });
}

/** Most recently active first: the newest message either way, else when it was made. */
const activeAt = (a: Agent) => a.last?.at ?? a.createdAt;
const byActivity = (list: Agent[]) => [...list].sort((x, y) => (activeAt(y) > activeAt(x) ? 1 : activeAt(y) < activeAt(x) ? -1 : 0));

function patchListed(id: string, f: (a: Agent) => Agent) {
  const list = agentStore.get().list;
  if (list) agentStore.set({ list: byActivity(list.map((a) => (a.id === id ? f(a) : a))) });
}

/** Moves an agent's `last` up to its newest real message, so the list reorders as soon as one shows. */
function bumpLast(id: string, msgs: Message[]) {
  const m = [...msgs].reverse().find((x) => x.id > 0 || x.role === 'user');
  if (!m) return;
  patchListed(id, (a) => (a.last && a.last.at >= m.created_at ? a : { ...a, last: { role: m.role, text: m.text, at: m.created_at } }));
}

// ---------------------------------------------------------------- kept on this PC

// The list and each chat's recent messages are kept in localStorage per account, so the page shows at once on the next
// start and only what's newer is fetched. The site stays the source of truth: everything is replaced as answers come.
const KEEP_MESSAGES = 60;
let owner: string | null = null;
const cacheKey = () => `agent.cache.v1.${owner}`;

function hydrate(username: string | null) {
  if (owner === username) return;
  owner = username;
  notified = null;
  agentStore.set({ list: null, chats: {}, selected: null, error: null, viewed: {} });
  if (!username) return;
  try {
    const c = JSON.parse(localStorage.getItem(cacheKey()) ?? 'null');
    if (!c || !Array.isArray(c.list)) return;
    const list = byActivity(c.list);
    // Opens on the agent used last: the top of the list.
    agentStore.set({ list, max: c.max ?? 10, prefs: c.prefs ?? agentStore.get().prefs, model: c.model ?? null, openrouter: !!c.openrouter, selected: list[0]?.id ?? null, chats: c.chats ?? {} });
  } catch {
    /* nothing kept */
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
function saveSoon() {
  if (!owner || saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const s = agentStore.get();
    if (!owner || !s.list) return;
    const chats: Record<string, Chat> = {};
    for (const [id, c] of Object.entries(s.chats)) {
      if (!s.list.some((a) => a.id === id)) continue;
      const real = c.messages.filter((m) => m.id > 0);
      const kept = real.slice(-KEEP_MESSAGES);
      chats[id] = { messages: kept, files: c.files, more: c.more || kept.length < real.length };
    }
    try {
      localStorage.setItem(cacheKey(), JSON.stringify({ list: s.list, max: s.max, prefs: s.prefs, model: s.model, openrouter: s.openrouter, selected: s.selected, chats }));
    } catch {
      /* full or blocked: it just loads from the site next time */
    }
  }, 800);
}
agentStore.subscribe(saveSoon);

// ---------------------------------------------------------------- loading

type ListAnswer = { agents: Agent[]; rev?: number; max: number; prefs: Prefs; model: string | null; openrouter: boolean; apps?: App[] };
type ChatAnswer = { agent: Agent; messages: Message[]; files?: AgentFile[]; rev?: number; more?: boolean };

function applyList(r: ListAnswer) {
  const s = agentStore.get();
  // Keep a newer `last` this PC already saw (a message just sent), so the order doesn't jump back.
  const prev = new Map((s.list ?? []).map((a) => [a.id, a]));
  const agents = byActivity(
    r.agents.map((a) => {
      const p = prev.get(a.id);
      const last = p?.last && (!a.last || p.last.at > a.last.at) ? p.last : a.last;
      // The open chat's unread count is zero while it's on screen: it's being read.
      return { ...a, last, unread: a.id === s.selected && viewing() ? 0 : a.unread };
    }),
  );
  const selected = s.selected && agents.some((a) => a.id === s.selected) ? s.selected : agents[0]?.id ?? null;
  agentStore.set({ list: agents, max: r.max, prefs: r.prefs, model: r.model, openrouter: r.openrouter, apps: r.apps ?? [], error: null, selected });
  const first = !notified;
  for (const a of agents) if (a.last) consider(a, a.last, first);
}

export async function loadAgents(since = 0): Promise<ListAnswer | null> {
  try {
    const r = await call<ListAnswer>('GET', since ? `${PATH}?since=${since}` : PATH);
    applyList(r);
    return r;
  } catch (e) {
    agentStore.set({ error: message(e) });
    return null;
  }
}

/** How many messages a whole-chat load brings: the latest ones; older ones come in as the user scrolls up. */
const PAGE = 50;

export async function loadChat(id: string, full = false, since = 0): Promise<ChatAnswer | null> {
  const had = agentStore.get().chats[id];
  const after = full ? 0 : lastRealId(had);
  const q = [`agent=${id}`, after ? `after=${after}` : `limit=${PAGE}`, since ? `since=${since}` : ''].filter(Boolean).join('&');
  try {
    const r = await call<ChatAnswer>('GET', `${PATH}?${q}`);
    setChat(id, (c) => ({ ...c, messages: merge(c.messages, r.messages, !after), files: r.files ?? c.files, error: null, more: r.more ?? c.more }));
    patchListed(id, (a) => ({ ...r.agent, unread: viewing() ? 0 : a.unread, last: a.last }));
    markViewed();
    bumpLast(id, r.messages);
    const a = agentStore.get().list?.find((x) => x.id === id);
    if (a && notified) for (const m of r.messages) consider(a, { role: m.role, text: m.text, at: m.created_at, kind: m.data && 'kind' in m.data ? m.data.kind : undefined });
    return r;
  } catch (e) {
    setChat(id, (c) => ({ ...c, error: message(e) }));
    return null;
  }
}

/** The page of messages before the oldest one shown, when the user scrolls to the top. */
export async function loadOlder(id: string): Promise<void> {
  const c = agentStore.get().chats[id];
  const oldest = c?.messages.find((m) => m.id > 0)?.id;
  if (!c || !c.more || c.older || !oldest) return;
  setChat(id, (x) => ({ ...x, older: true }));
  try {
    const r = await call<ChatAnswer>('GET', `${PATH}?agent=${id}&before=${oldest}&limit=${PAGE}`);
    setChat(id, (x) => ({ ...x, messages: merge(x.messages, r.messages, false), more: !!r.more, older: false }));
  } catch (e) {
    setChat(id, (x) => ({ ...x, older: false, error: message(e) }));
  }
}

/** Whether this chat has messages to show already (kept from last time, or loaded). */
const hasChat = (id: string) => (agentStore.get().chats[id]?.messages.length ?? 0) > 0;

/** A file's contents as a data: URL, fetched once per file (files never change). */
const fileUrls = new Map<string, Promise<string>>();
export function fileUrl(agentId: string, f: AgentFile): Promise<string> {
  let p = fileUrls.get(f.id);
  if (!p) {
    p = call<{ file: { type: string; data: string } }>('GET', `${PATH}?agent=${agentId}&file=${f.id}`).then((r) => `data:${r.file.type || f.type};base64,${r.file.data}`);
    p.catch(() => fileUrls.delete(f.id));
    fileUrls.set(f.id, p);
  }
  return p;
}

export const isImage = (f: AgentFile) => /^image\/(png|jpe?g|gif|webp|svg\+xml)$/.test(f.type);

// ---------------------------------------------------------------- actions

const readFile = (f: File) =>
  new Promise<{ name: string; type: string; data: string }>((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve({ name: f.name, type: f.type || 'application/octet-stream', data: String(r.result) });
    r.onerror = () => reject(r.error);
    r.readAsDataURL(f);
  });

/** Files of messages that failed, for Retry. */
const unsent = new Map<number, File[]>();

/** Shows the message at once and sends it in the background. Never throws: a failure marks the message. */
export function sendToAgent(id: string, text: string, files: File[]): void {
  const temp: Message = {
    id: -Date.now(),
    role: 'user',
    text,
    data: files.length ? { files: files.map((f, i) => ({ id: `local${i}`, name: f.name, type: f.type, size: f.size })) } : null,
    created_at: new Date().toISOString(),
    local: { state: 'sending' },
  };
  setChat(id, (c) => ({ ...c, messages: [...c.messages, temp] }));
  bumpLast(id, [temp]);
  // It starts "typing" now; the site confirms within a moment.
  patchListed(id, (a) => (a.status === 'idle' || a.status === 'sleeping' || a.status === 'error' || a.status === 'blocked' ? { ...a, status: 'thinking', activity: null, wakeAt: null } : a));
  void deliver(id, temp, files);
}

async function deliver(id: string, temp: Message, files: File[]) {
  try {
    if (files.some((f) => f.size > 10 << 20)) throw new Error('Attachments can be up to 10 MB each.');
    const data = await Promise.all(files.map(readFile));
    const r = await post<{ message: Message }>('send', { id, text: temp.text, files: data, tz: localTz() });
    unsent.delete(temp.id);
    setChat(id, (c) => ({ ...c, messages: merge(c.messages.filter((m) => m.id !== temp.id), [r.message], false) }));
  } catch (e) {
    unsent.set(temp.id, files);
    setChat(id, (c) => ({ ...c, messages: c.messages.map((m) => (m.id === temp.id ? { ...m, local: { state: 'failed', error: message(e) } } : m)) }));
    patchListed(id, (a) => (a.status === 'thinking' ? { ...a, status: 'idle' } : a));
  }
}

export function retrySend(id: string, temp: Message) {
  const files = unsent.get(temp.id) ?? [];
  unsent.delete(temp.id);
  setChat(id, (c) => ({ ...c, messages: c.messages.filter((m) => m.id !== temp.id) }));
  sendToAgent(id, temp.text, files);
}

export function discardUnsent(id: string, temp: Message) {
  unsent.delete(temp.id);
  setChat(id, (c) => ({ ...c, messages: c.messages.filter((m) => m.id !== temp.id) }));
}

/** "always": approve it and auto-accept commands of that kind from now on (risky ones still ask). */
export async function answerCard(id: string, msg: number, choice: 'approve' | 'deny' | 'always') {
  await post('answer', { id, message: msg, answer: choice });
  const answer = choice === 'deny' ? 'deny' : 'approve';
  // Shown answered at once; the reload confirms it.
  setChat(id, (c) => ({ ...c, messages: c.messages.map((m) => (m.id === msg && m.data && 'kind' in m.data ? { ...m, data: { ...m.data, answer, always: choice === 'always' || undefined } as Card } : m)) }));
  if (choice === 'always') void loadAgents();
  void loadChat(id, true);
}

/** The values of a secret card: they go to the site, which keeps them on the app's connection, not in the chat. */
export async function answerSecret(id: string, msg: number, values: Record<string, string>) {
  await post('secret', { id, message: msg, values });
  setChat(id, (c) => ({ ...c, messages: c.messages.map((m) => (m.id === msg && m.data && 'kind' in m.data ? { ...m, data: { ...m.data, answer: 'approve' } as Card } : m)) }));
  void loadChat(id, true);
}

// ---------------------------------------------------------------- Connect to Apps

/** Connects an app for `target`: a remote server ({ name, url, auth? }) or one run on the agents' computers ({ name, command, env_needed }). */
export async function connectApp(target: AppTarget, app: { name: string; url?: string; auth?: 'token'; header?: string; command?: string; env_needed?: string[] }): Promise<ConnectAnswer> {
  const r = await post<ConnectAnswer>('app_connect', { agent: target, ...app });
  void loadAgents();
  return r;
}

export async function appSecret(target: AppTarget, server: string, values: Record<string, string>): Promise<App> {
  const r = await post<{ app: App }>('app_secret', { agent: target, server, values });
  void loadAgents();
  return r.app;
}

export async function disconnectApp(target: AppTarget, server: string): Promise<void> {
  await post('app_disconnect', { agent: target, server });
  await loadAgents();
}

/** This PC's time zone, sent with each message so alarms and "every day at 9" use the user's own clock. */
export function localTz(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
  } catch {
    return undefined;
  }
}

export const loadSchedules = (id: string) => call<{ schedules: Schedule[] }>('GET', `${PATH}?agent=${id}&schedules=1`).then((r) => r.schedules);
export const addSchedule = (id: string, schedule: NewSchedule) => post<{ schedule: Schedule }>('schedule', { id, schedule: { timezone: localTz(), ...schedule } }).then((r) => r.schedule);
export const removeSchedule = (id: string, schedule: string) => post('unschedule', { id, schedule });
export const pauseSchedule = (id: string, schedule: string, paused: boolean) => post<{ schedule: Schedule }>('pause_schedule', { id, schedule, paused }).then((r) => r.schedule);

export async function createAgent(look: Look): Promise<void> {
  const r = await post<{ agent: Agent }>('create', look);
  agentStore.set({ selected: r.agent.id, list: byActivity([...(agentStore.get().list ?? []), r.agent]) });
  void loadAgents();
}

export async function updateAgent(id: string, look: Look): Promise<void> {
  await post('update', { id, ...look });
  patchListed(id, (a) => ({ ...a, ...look }));
}

export async function deleteAgent(id: string): Promise<void> {
  await post('delete', { id });
  const { [id]: _gone, ...chats } = agentStore.get().chats;
  const list = agentStore.get().list?.filter((a) => a.id !== id) ?? null;
  agentStore.set({ chats, list, selected: list?.[0]?.id ?? null });
}

export async function stopAgent(id: string): Promise<void> {
  patchListed(id, (a) => ({ ...a, status: 'idle', activity: null, activityKind: null, wakeAt: null }));
  await post('stop', { id });
}

export async function setPrefs(p: Partial<Prefs>): Promise<void> {
  const before = agentStore.get().prefs;
  agentStore.set({ prefs: { ...before, ...p } });
  try {
    const r = await post<{ prefs: Prefs }>('prefs', p);
    agentStore.set({ prefs: r.prefs });
  } catch (e) {
    agentStore.set({ prefs: before });
    throw e;
  }
}

export async function setOpenRouterKey(key: string | null): Promise<void> {
  await post('openrouter', { key });
  await loadAgents();
}

export const loadMemory = () => call<Memory>('GET', `${PATH}?memory=1`);
export const saveMemory = (saved: string, text: string) => post<{ memory: Memory }>('memory', { saved, text }).then((r) => r.memory);

// ---------------------------------------------------------------- notifications

/** The open chat is on screen: the window is in front and on this page. */
const viewing = () => document.hasFocus() && ui.get().page === 'agent';

/** Notes the open chat as seen up to its newest message, when it's on screen. */
function markViewed() {
  const s = agentStore.get();
  const id = s.selected;
  if (!id || !viewing()) return;
  const top = lastRealId(s.chats[id]);
  const listed = s.list?.find((a) => a.id === id);
  if (s.viewed[id] !== top) agentStore.set({ viewed: { ...s.viewed, [id]: top } });
  if (listed?.unread) patchListed(id, (a) => ({ ...a, unread: 0 }));
}

/** Messages from the agent in a chat after the newest one seen on screen. */
function unseen(s: ReturnType<typeof agentStore.get>, id: string): number {
  const v = s.viewed[id];
  if (v === undefined) return 0;
  return s.chats[id]?.messages.filter((m) => m.id > v && m.role !== 'user').length ?? 0;
}

/** The newest message of each agent already notified (or seen), by its time. null until the site's first answer, whose
 *  messages are only noted: the badge counts what came while Scout was closed, without a notification for each. */
let notified: Map<string, string> | null = null;

/** Notifies about a message from an agent if it's newer than the last one and the user isn't looking at its chat. Cards
 *  notify when they need the user; what the agent did along the way (activity) doesn't. */
function consider(a: Agent, m: { role: string; text: string; at: string; kind?: Card['kind'] }, quiet = false) {
  notified ??= new Map();
  const seen = notified.get(a.id);
  if (seen && seen >= m.at) return;
  notified.set(a.id, m.at);
  const worth = m.role === 'agent' || (m.role === 'card' && !!m.kind && m.kind !== 'activity' && m.kind !== 'notice');
  const looking = viewing() && agentStore.get().selected === a.id;
  if (quiet || !worth || looking || !m.text.trim()) return;
  act('agentAlert', { title: a.name, body: m.text.replace(/\s+/g, ' ').trim() });
}

/** The unread count in the corner of the taskbar button, Discord style but white. */
function useTaskbarBadge() {
  const unread = useAgentUnread();
  useEffect(() => {
    const label = unread <= 0 ? '' : unread > 9 ? '9+' : String(unread);
    if (!label) return void invoke('set_badge', { rgba: null, size: 0 }).catch(() => {});
    const size = 32;
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = size;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.beginPath();
    ctx.arc(size / 2, size / 2, size / 2 - 1, 0, Math.PI * 2);
    ctx.fillStyle = '#ffffff';
    ctx.fill();
    // A faint edge, so it still shows on a light taskbar.
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = 'rgba(0, 0, 0, 0.35)';
    ctx.stroke();
    ctx.fillStyle = '#111111';
    ctx.font = `bold ${label.length > 1 ? 17 : 21}px "Segoe UI", sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, size / 2, size / 2 + 1);
    const rgba = Array.from(ctx.getImageData(0, 0, size, size).data);
    invoke('set_badge', { rgba, size }).catch((e) => console.error('set_badge', e));
  }, [unread]);
}

// ---------------------------------------------------------------- staying fresh

/** Keeps the agents fresh: long-polled while the page is open, slowly elsewhere for the badge. */
export function useAgentSync() {
  const { hasKey, username } = useView('cfg');
  const onPage = useUi((u) => u.page === 'agent');
  const selected = useAgents((s) => s.selected);

  // What this PC kept from last time shows before anything is fetched.
  useLayoutEffect(() => hydrate(hasKey ? username ?? null : null), [hasKey, username]);
  useTaskbarBadge();

  // Coming back to the window, or to the chat, reads it.
  useEffect(() => {
    markViewed();
    window.addEventListener('focus', markViewed);
    return () => window.removeEventListener('focus', markViewed);
  }, [onPage, selected]);

  useEffect(() => {
    if (!hasKey) return;
    let live = true;
    void (async () => {
      let rev = 0;
      while (live) {
        // A hidden window still checks now and then, for notifications and the taskbar badge.
        const watching = onPage && !document.hidden;
        const r = await loadAgents(watching ? rev : 0);
        if (!live) return;
        if (watching && r?.rev) {
          rev = r.rev;
          continue;
        }
        // Not on the page, hidden, an older site, or it failed: plain polling.
        rev = 0;
        await sleep(watching ? (r ? 4000 : 6000) : 15_000);
      }
    })();
    return () => {
      live = false;
    };
  }, [hasKey, onPage]);

  useEffect(() => {
    if (!hasKey || !onPage || !selected) return;
    let live = true;
    void (async () => {
      let rev = 0;
      let n = 0;
      while (live) {
        if (document.hidden && rev) {
          await sleep(2000);
          continue;
        }
        // A chat kept on this PC first fetches only what's newer; the latest page in full follows on the next round
        // and now and then after, for cards answered elsewhere (a sign-in finished in the browser).
        const full = !rev ? !hasChat(selected) : ++n % 8 === 1;
        const r = await loadChat(selected, full, full && n === 1 ? 0 : rev);
        if (!live) return;
        if (r?.rev) {
          rev = r.rev;
          continue;
        }
        rev = 0;
        await sleep(r ? 2000 : 5000);
      }
    })();
    return () => {
      live = false;
    };
  }, [hasKey, onPage, selected]);
}

export function useAgentUnread(): number {
  return useAgents((s) => s.list?.reduce((n, a) => n + Math.max(a.unread, unseen(s, a.id)), 0) ?? 0);
}

export const STATUS_TEXT: Record<Status, string> = {
  idle: 'Idle',
  thinking: '',
  working: 'Working…',
  waiting: 'Waiting for you',
  sleeping: 'Sleeping',
  blocked: 'Blocked',
  error: 'Something went wrong',
};
