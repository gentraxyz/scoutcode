/**
 * ScoutCode Delta's data and the one place it changes. A workspace is a set of records (teams, workflow states, issues,
 * comments, labels, projects, cycles and the issues' history). Every change is an Op, and `apply` turns an op into the
 * records it creates or changes. The same code runs in the desktop app for local workspaces, in the browser for its
 * optimistic copy and on the ScoutCode server for cloud workspaces, so all three agree on what an op does.
 *
 * No framework imports here: the website's server imports this file too.
 */

export type Kind = 'workspace' | 'team' | 'state' | 'label' | 'issue' | 'comment' | 'project' | 'cycle' | 'activity';
export const KINDS: Kind[] = ['workspace', 'team', 'state', 'label', 'issue', 'comment', 'project', 'cycle', 'activity'];

export type StateType = 'backlog' | 'unstarted' | 'started' | 'review' | 'completed' | 'canceled';
export const STATE_TYPES: StateType[] = ['backlog', 'unstarted', 'started', 'review', 'completed', 'canceled'];
export type ProjectStatus = 'planned' | 'active' | 'paused' | 'completed' | 'canceled';
export const PROJECT_STATUSES: ProjectStatus[] = ['planned', 'active', 'paused', 'completed', 'canceled'];

/** 0 none, 1 urgent, 2 high, 3 medium, 4 low, as Linear-style trackers number them. */
export const PRIORITIES = ['No priority', 'Urgent', 'High', 'Medium', 'Low'] as const;

export interface Workspace { id: string; name: string; createdAt: number }
export interface Team { id: string; key: string; name: string; nextNumber: number; nextCycle: number; createdAt: number }
export interface State { id: string; teamId: string; name: string; type: StateType; color: string; position: number }
export interface Label { id: string; name: string; color: string }
/** The Scout chat working an issue, when one was handed it. */
export interface AgentRun { where: 'desktop' | 'scoutmote'; pc: string | null; cwd: string; chatId: string | null; startedAt: number; by: string }
export interface Issue {
  id: string; teamId: string; number: number; title: string; description: string; stateId: string; priority: number;
  assignee: string | null; labelIds: string[]; projectId: string | null; cycleId: string | null; parentId: string | null;
  estimate: number | null; dueDate: string | null; sortOrder: number; creator: string; createdAt: number; updatedAt: number;
  completedAt: number | null; agent: AgentRun | null;
}
export interface Comment { id: string; issueId: string; author: string; body: string; createdAt: number; editedAt: number | null }
export interface Project {
  id: string; name: string; description: string; status: ProjectStatus; lead: string | null; startDate: string | null;
  targetDate: string | null; color: string; teamIds: string[]; createdAt: number;
}
export interface Cycle { id: string; teamId: string; number: number; name: string; startsAt: string; endsAt: string }
export interface Activity { id: string; issueId: string; actor: string; at: number; field: string; from: unknown; to: unknown }

export interface Doc {
  workspace: Workspace;
  team: Record<string, Team>;
  state: Record<string, State>;
  label: Record<string, Label>;
  issue: Record<string, Issue>;
  comment: Record<string, Comment>;
  project: Record<string, Project>;
  cycle: Record<string, Cycle>;
  activity: Record<string, Activity>;
}

/** A record as it's stored and sent: `data` is null once it's deleted, so a deletion syncs like any other change. */
export interface Rec { kind: Kind; id: string; data: any; seq?: number }

export type Op =
  | { type: 'create'; kind: Exclude<Kind, 'workspace' | 'activity'>; id: string; data: Record<string, unknown> }
  | { type: 'update'; kind: Exclude<Kind, 'activity'>; id: string; patch: Record<string, unknown> }
  | { type: 'delete'; kind: Exclude<Kind, 'workspace' | 'activity'>; id: string };

export class OpError extends Error {}

// ---------- ids and empties ----------

export function newId(): string {
  const a = new Uint8Array(12);
  crypto.getRandomValues(a);
  return Array.from(a, (b) => 'abcdefghijklmnopqrstuvwxyz0123456789'[b % 36]).join('');
}

export function emptyDoc(id: string, name: string, now = Date.now()): Doc {
  return { workspace: { id, name, createdAt: now }, team: {}, state: {}, label: {}, issue: {}, comment: {}, project: {}, cycle: {}, activity: {} };
}

/** A new workspace's first team and labels, as ops, so a cloud workspace is seeded the same way as a local one. */
export function starterOps(teamName = 'Engineering', key = 'ENG'): Op[] {
  const team = newId();
  return [
    { type: 'create', kind: 'team', id: team, data: { name: teamName, key } },
    ...[['Bug', '#e5484d'], ['Feature', '#8e4ec6'], ['Improvement', '#3e63dd'], ['Docs', '#12a594']].map(
      ([name, color]): Op => ({ type: 'create', kind: 'label', id: newId(), data: { name, color } }),
    ),
  ];
}

const DEFAULT_STATES: [string, StateType, string][] = [
  ['Backlog', 'backlog', '#8b8d98'],
  ['Todo', 'unstarted', '#b0b4ba'],
  ['In Progress', 'started', '#f5a524'],
  ['In Review', 'review', '#3e9b4f'],
  ['Done', 'completed', '#5e6ad2'],
  ['Canceled', 'canceled', '#6f6e77'],
];

// ---------- records <-> doc ----------

export function toRecords(doc: Doc): Rec[] {
  const out: Rec[] = [{ kind: 'workspace', id: 'ws', data: doc.workspace }];
  for (const kind of KINDS) if (kind !== 'workspace') for (const [id, data] of Object.entries(doc[kind])) out.push({ kind, id, data });
  return out;
}

/** Puts records into a doc in place. Unknown kinds are skipped, so an older app survives a newer server. */
export function merge(doc: Doc, recs: Rec[]): void {
  for (const r of recs) {
    if (r.kind === 'workspace') {
      if (r.data) doc.workspace = { ...doc.workspace, ...r.data };
      continue;
    }
    const table = (doc as any)[r.kind] as Record<string, unknown> | undefined;
    if (!table) continue;
    if (r.data == null) delete table[r.id];
    else table[r.id] = r.data;
  }
}

export function fromRecords(id: string, recs: Rec[]): Doc {
  const doc = emptyDoc(id, 'Workspace');
  merge(doc, recs);
  return doc;
}

export function cloneDoc(doc: Doc): Doc {
  const out = { ...doc, workspace: { ...doc.workspace } } as Doc;
  for (const kind of KINDS) if (kind !== 'workspace') (out as any)[kind] = { ...doc[kind] };
  return out;
}

// ---------- field checks ----------

const str = (v: unknown, max: number, what: string, allowEmpty = false): string => {
  if (typeof v !== 'string') throw new OpError(`${what} must be text.`);
  const t = v.length > max ? v.slice(0, max) : v;
  if (!allowEmpty && !t.trim()) throw new OpError(`${what} can't be empty.`);
  return t;
};
const int = (v: unknown, min: number, max: number, what: string): number => {
  const n = Number(v);
  if (!Number.isInteger(n) || n < min || n > max) throw new OpError(`${what} must be a whole number from ${min} to ${max}.`);
  return n;
};
const num = (v: unknown, what: string): number => {
  const n = Number(v);
  if (!Number.isFinite(n)) throw new OpError(`${what} must be a number.`);
  return n;
};
const date = (v: unknown, what: string): string | null => {
  if (v == null || v === '') return null;
  if (typeof v !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(v)) throw new OpError(`${what} must be a date like 2026-11-04.`);
  return v;
};
const color = (v: unknown): string => (typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v) ? v : '#8b8d98');
const oneOf = <T extends string>(v: unknown, list: readonly T[], what: string): T => {
  if (!list.includes(v as T)) throw new OpError(`${what} must be one of ${list.join(', ')}.`);
  return v as T;
};
const ref = (doc: Doc, kind: Kind, v: unknown, what: string): string | null => {
  if (v == null || v === '') return null;
  if (typeof v !== 'string' || !(doc as any)[kind][v]) throw new OpError(`That ${what} doesn't exist.`);
  return v;
};
const user = (v: unknown, members: string[] | null): string | null => {
  if (v == null || v === '') return null;
  if (typeof v !== 'string') throw new OpError('Assignee must be a username.');
  if (v === SCOUT) return v;
  if (members && !members.includes(v)) throw new OpError(`${v} isn't in this workspace.`);
  return v;
};

/** The assignee that stands for a Scout agent. */
export const SCOUT = 'scout';

function teamKey(doc: Doc, v: unknown, self: string | null): string {
  const key = str(v, 8, 'Team key').toUpperCase();
  if (!/^[A-Z][A-Z0-9]{0,7}$/.test(key)) throw new OpError('A team key is 1–8 letters or digits, starting with a letter.');
  if (Object.values(doc.team).some((t) => t.key === key && t.id !== self)) throw new OpError(`Another team already uses ${key}.`);
  return key;
}

function agentRun(v: unknown): AgentRun | null {
  if (v == null) return null;
  const a = v as Record<string, unknown>;
  return {
    where: a.where === 'scoutmote' ? 'scoutmote' : 'desktop',
    pc: typeof a.pc === 'string' ? a.pc.slice(0, 200) : null,
    cwd: typeof a.cwd === 'string' ? a.cwd.slice(0, 1000) : '',
    chatId: typeof a.chatId === 'string' ? a.chatId.slice(0, 200) : null,
    startedAt: Number(a.startedAt) || Date.now(),
    by: typeof a.by === 'string' ? a.by.slice(0, 100) : '',
  };
}

/** Each kind's editable fields and how they're checked. */
function clean(doc: Doc, kind: Kind, id: string, input: Record<string, unknown>, members: string[] | null): Record<string, any> {
  const out: Record<string, any> = {};
  const has = (k: string) => Object.prototype.hasOwnProperty.call(input, k);
  const f = (k: string, check: (v: unknown) => unknown) => {
    if (has(k)) out[k] = check(input[k]);
  };
  switch (kind) {
    case 'workspace':
      f('name', (v) => str(v, 80, 'Name'));
      break;
    case 'team':
      f('name', (v) => str(v, 80, 'Team name'));
      f('key', (v) => teamKey(doc, v, id));
      break;
    case 'state':
      f('name', (v) => str(v, 40, 'State name'));
      f('type', (v) => oneOf(v, STATE_TYPES, 'State type'));
      f('color', color);
      f('position', (v) => num(v, 'Position'));
      break;
    case 'label':
      f('name', (v) => str(v, 40, 'Label name'));
      f('color', color);
      break;
    case 'issue':
      f('title', (v) => str(v, 300, 'Title'));
      f('description', (v) => str(v ?? '', 50_000, 'Description', true));
      f('priority', (v) => int(v, 0, 4, 'Priority'));
      f('assignee', (v) => user(v, members));
      f('labelIds', (v) => {
        if (!Array.isArray(v)) throw new OpError('Labels must be a list.');
        return [...new Set(v.map((x) => ref(doc, 'label', x, 'label')!))].slice(0, 20);
      });
      f('projectId', (v) => ref(doc, 'project', v, 'project'));
      f('cycleId', (v) => ref(doc, 'cycle', v, 'cycle'));
      f('parentId', (v) => {
        const p = ref(doc, 'issue', v, 'parent issue');
        if (p === id) throw new OpError("An issue can't be its own parent.");
        // No loops: walk up from the new parent.
        for (let at = p, n = 0; at && n < 100; at = doc.issue[at]?.parentId ?? null, n++) if (at === id) throw new OpError('That would make a loop of sub-issues.');
        return p;
      });
      f('estimate', (v) => (v == null || v === '' ? null : int(v, 0, 100, 'Estimate')));
      f('dueDate', (v) => date(v, 'Due date'));
      f('sortOrder', (v) => num(v, 'Order'));
      f('agent', agentRun);
      break;
    case 'comment':
      f('body', (v) => str(v, 20_000, 'Comment'));
      break;
    case 'project':
      f('name', (v) => str(v, 120, 'Project name'));
      f('description', (v) => str(v ?? '', 20_000, 'Description', true));
      f('status', (v) => oneOf(v, PROJECT_STATUSES, 'Status'));
      f('lead', (v) => user(v, members));
      f('startDate', (v) => date(v, 'Start date'));
      f('targetDate', (v) => date(v, 'Target date'));
      f('color', color);
      f('teamIds', (v) => {
        if (!Array.isArray(v)) throw new OpError('Teams must be a list.');
        return [...new Set(v.map((x) => ref(doc, 'team', x, 'team')!))];
      });
      break;
    case 'cycle':
      f('name', (v) => str(v ?? '', 80, 'Cycle name', true));
      f('startsAt', (v) => date(v, 'Start') ?? (() => { throw new OpError('A cycle needs a start date.'); })());
      f('endsAt', (v) => date(v, 'End') ?? (() => { throw new OpError('A cycle needs an end date.'); })());
      break;
  }
  return out;
}

// ---------- apply ----------

export interface Ctx {
  actor: string;
  now: number;
  /** Who may be assigned. Null skips the check, for a local workspace with one person. */
  members: string[] | null;
  /** Makes the ids of records an op creates on its own (history, a new team's states). Seeded the same in the browser and
   * on the server, so the browser's optimistic copy and the server's agree. */
  newId?: () => string;
}

/** Ids from a seed: seed + a counter. */
export function seededIds(seed: string): () => string {
  let n = 0;
  return () => `${seed}${(n++).toString(36)}`;
}

/**
 * Applies one op to the doc in place and returns the records it changed. Throws OpError, leaving the doc as it was, when
 * the op isn't allowed.
 */
export function apply(doc: Doc, op: Op, ctx: Ctx): Rec[] {
  if (!op || typeof op !== 'object' || !KINDS.includes((op as Op).kind) || (op as { kind: string }).kind === 'activity') throw new OpError('Unknown change.');
  if (typeof op.id !== 'string' || !/^[a-z0-9]{6,40}$/i.test(op.id) && !(op.kind === 'workspace' && op.id === 'ws')) throw new OpError('Bad id.');
  const changed: Rec[] = [];
  const gen = () => {
    const id = (ctx.newId ?? newId)();
    if (doc.activity[id] || doc.state[id]) throw new OpError('Those changes were already made.');
    return id;
  };
  const put = (kind: Kind, id: string, data: any) => {
    if (kind === 'workspace') doc.workspace = data;
    else (doc as any)[kind][id] = data;
    changed.push({ kind, id, data });
  };
  const drop = (kind: Kind, id: string) => {
    if (!(doc as any)[kind][id]) return;
    delete (doc as any)[kind][id];
    changed.push({ kind, id, data: null });
  };
  const log = (issueId: string, field: string, from: unknown, to: unknown) => {
    const id = gen();
    put('activity', id, { id, issueId, actor: ctx.actor, at: ctx.now, field, from, to } satisfies Activity);
  };

  if (op.type === 'create') {
    if ((doc as any)[op.kind]?.[op.id]) throw new OpError('That already exists.');
    const d = clean(doc, op.kind, op.id, op.data ?? {}, ctx.members);
    switch (op.kind) {
      case 'team': {
        if (!d.name) throw new OpError('A team needs a name.');
        const key = d.key ?? teamKey(doc, String(d.name).replace(/[^a-z0-9]/gi, '').slice(0, 3) || 'T', op.id);
        put('team', op.id, { id: op.id, name: d.name, key, nextNumber: 1, nextCycle: 1, createdAt: ctx.now } satisfies Team);
        DEFAULT_STATES.forEach(([name, type, c], i) => {
          const sid = gen();
          put('state', sid, { id: sid, teamId: op.id, name, type, color: c, position: i } satisfies State);
        });
        break;
      }
      case 'state': {
        const teamId = ref(doc, 'team', op.data?.teamId, 'team');
        if (!teamId) throw new OpError('A state belongs to a team.');
        const max = Math.max(-1, ...Object.values(doc.state).filter((s) => s.teamId === teamId).map((s) => s.position));
        put('state', op.id, { id: op.id, teamId, name: d.name ?? 'New state', type: d.type ?? 'unstarted', color: d.color ?? '#8b8d98', position: d.position ?? max + 1 });
        break;
      }
      case 'label':
        if (!d.name) throw new OpError('A label needs a name.');
        put('label', op.id, { id: op.id, name: d.name, color: d.color ?? '#8b8d98' });
        break;
      case 'issue': {
        const teamId = ref(doc, 'team', op.data?.teamId, 'team');
        if (!teamId) throw new OpError('An issue belongs to a team.');
        if (!d.title) throw new OpError('An issue needs a title.');
        const team = doc.team[teamId];
        const states = teamStates(doc, teamId);
        let stateId = ref(doc, 'state', op.data?.stateId, 'state');
        if (stateId && doc.state[stateId].teamId !== teamId) throw new OpError("That state isn't in this team's workflow.");
        stateId ??= (states.find((s) => s.type === 'unstarted') ?? states[0])?.id ?? null;
        if (!stateId) throw new OpError('This team has no workflow states.');
        put('team', teamId, { ...team, nextNumber: team.nextNumber + 1 });
        const issue: Issue = {
          id: op.id, teamId, number: team.nextNumber, title: d.title as string, description: (d.description as string) ?? '', stateId,
          priority: (d.priority as number) ?? 0, assignee: (d.assignee as string | null) ?? null, labelIds: (d.labelIds as string[]) ?? [],
          projectId: (d.projectId as string | null) ?? null, cycleId: (d.cycleId as string | null) ?? null, parentId: (d.parentId as string | null) ?? null,
          estimate: (d.estimate as number | null) ?? null, dueDate: (d.dueDate as string | null) ?? null, sortOrder: (d.sortOrder as number) ?? -ctx.now,
          creator: ctx.actor, createdAt: ctx.now, updatedAt: ctx.now, completedAt: isDone(doc.state[stateId]) ? ctx.now : null, agent: (d.agent as AgentRun | null) ?? null,
        };
        put('issue', op.id, issue);
        log(op.id, 'created', null, null);
        break;
      }
      case 'comment': {
        const issueId = ref(doc, 'issue', op.data?.issueId, 'issue');
        if (!issueId) throw new OpError('A comment belongs to an issue.');
        if (!d.body) throw new OpError("A comment can't be empty.");
        put('comment', op.id, { id: op.id, issueId, author: ctx.actor, body: d.body, createdAt: ctx.now, editedAt: null } satisfies Comment);
        break;
      }
      case 'project':
        if (!d.name) throw new OpError('A project needs a name.');
        put('project', op.id, {
          id: op.id, name: d.name, description: d.description ?? '', status: d.status ?? 'planned', lead: d.lead ?? null, startDate: d.startDate ?? null,
          targetDate: d.targetDate ?? null, color: d.color ?? '#5e6ad2', teamIds: d.teamIds ?? [], createdAt: ctx.now,
        } satisfies Project);
        break;
      case 'cycle': {
        const teamId = ref(doc, 'team', op.data?.teamId, 'team');
        if (!teamId) throw new OpError('A cycle belongs to a team.');
        if (!d.startsAt || !d.endsAt) throw new OpError('A cycle needs start and end dates.');
        if (String(d.endsAt) < String(d.startsAt)) throw new OpError('A cycle has to end after it starts.');
        const team = doc.team[teamId];
        put('team', teamId, { ...team, nextCycle: team.nextCycle + 1 });
        put('cycle', op.id, { id: op.id, teamId, number: team.nextCycle, name: d.name ?? '', startsAt: d.startsAt, endsAt: d.endsAt } satisfies Cycle);
        break;
      }
    }
    return changed;
  }

  if (op.type === 'update') {
    const old = op.kind === 'workspace' ? doc.workspace : (doc as any)[op.kind]?.[op.id];
    if (!old) throw new OpError("That doesn't exist any more.");
    const d = clean(doc, op.kind, op.id, op.patch ?? {}, ctx.members);
    if (op.kind === 'comment' && old.author !== ctx.actor) throw new OpError('Only the person who wrote a comment can edit it.');
    if (op.kind === 'cycle' && String(d.endsAt ?? old.endsAt) < String(d.startsAt ?? old.startsAt)) throw new OpError('A cycle has to end after it starts.');
    const next: any = { ...old, ...d };
    if (op.kind === 'issue') {
      // Moving teams: the issue takes the new team's number and a state of the same type in its workflow.
      const teamId = op.patch?.teamId !== undefined ? ref(doc, 'team', op.patch.teamId, 'team') : old.teamId;
      if (!teamId) throw new OpError('An issue belongs to a team.');
      if (teamId !== old.teamId) {
        const team = doc.team[teamId];
        put('team', teamId, { ...team, nextNumber: team.nextNumber + 1 });
        next.teamId = teamId;
        next.number = team.nextNumber;
        const type = doc.state[old.stateId]?.type;
        const states = teamStates(doc, teamId);
        next.stateId = (states.find((s) => s.type === type) ?? states[0]).id;
        next.cycleId = null;
        log(op.id, 'team', old.teamId, teamId);
      }
      if (op.patch?.stateId !== undefined) {
        const s = ref(doc, 'state', op.patch.stateId, 'state');
        if (!s || doc.state[s].teamId !== next.teamId) throw new OpError("That state isn't in this team's workflow.");
        next.stateId = s;
      }
      if (next.cycleId && doc.cycle[next.cycleId]?.teamId !== next.teamId) throw new OpError("That cycle belongs to another team.");
      if (next.stateId !== old.stateId) next.completedAt = isDone(doc.state[next.stateId]) ? ctx.now : null;
      next.updatedAt = ctx.now;
      for (const field of ['title', 'stateId', 'priority', 'assignee', 'labelIds', 'projectId', 'cycleId', 'parentId', 'estimate', 'dueDate', 'description'] as const) {
        if (JSON.stringify(old[field]) !== JSON.stringify(next[field])) log(op.id, field, field === 'description' ? null : old[field], field === 'description' ? null : next[field]);
      }
    }
    if (op.kind === 'comment') next.editedAt = ctx.now;
    put(op.kind, op.id, next);
    return changed;
  }

  if (op.type === 'delete') {
    const old = (doc as any)[op.kind]?.[op.id];
    if (!old) return changed;
    switch (op.kind) {
      case 'team':
        if (Object.keys(doc.team).length <= 1) throw new OpError('A workspace needs at least one team.');
        if (Object.values(doc.issue).some((i) => i.teamId === op.id)) throw new OpError('Move or delete this team’s issues first.');
        for (const s of teamStates(doc, op.id)) drop('state', s.id);
        for (const c of Object.values(doc.cycle)) if (c.teamId === op.id) drop('cycle', c.id);
        for (const p of Object.values(doc.project)) if (p.teamIds.includes(op.id)) put('project', p.id, { ...p, teamIds: p.teamIds.filter((t) => t !== op.id) });
        break;
      case 'state': {
        const left = teamStates(doc, old.teamId).filter((s) => s.id !== op.id);
        if (!left.length) throw new OpError('A team needs at least one state.');
        if (Object.values(doc.issue).some((i) => i.stateId === op.id)) throw new OpError('Move the issues in this state first.');
        break;
      }
      case 'label':
        for (const i of Object.values(doc.issue)) if (i.labelIds.includes(op.id)) put('issue', i.id, { ...i, labelIds: i.labelIds.filter((l) => l !== op.id) });
        break;
      case 'issue':
        for (const c of Object.values(doc.comment)) if (c.issueId === op.id) drop('comment', c.id);
        for (const a of Object.values(doc.activity)) if (a.issueId === op.id) drop('activity', a.id);
        for (const i of Object.values(doc.issue)) if (i.parentId === op.id) put('issue', i.id, { ...i, parentId: null });
        break;
      case 'comment':
        if (old.author !== ctx.actor) throw new OpError('Only the person who wrote a comment can delete it.');
        break;
      case 'project':
        for (const i of Object.values(doc.issue)) if (i.projectId === op.id) put('issue', i.id, { ...i, projectId: null });
        break;
      case 'cycle':
        for (const i of Object.values(doc.issue)) if (i.cycleId === op.id) put('issue', i.id, { ...i, cycleId: null });
        break;
    }
    drop(op.kind, op.id);
    return changed;
  }
  throw new OpError('Unknown change.');
}

/** Applies ops in order, all or nothing: on an error the doc is put back. */
export function applyAll(doc: Doc, ops: Op[], ctx: Ctx): Rec[] {
  const before = cloneDoc(doc);
  const out = new Map<string, Rec>();
  try {
    for (const op of ops) for (const r of apply(doc, op, ctx)) out.set(`${r.kind}:${r.id}`, r);
  } catch (e) {
    Object.assign(doc, before);
    throw e;
  }
  return [...out.values()];
}

// ---------- reading ----------

export const isDone = (s: State | undefined) => s?.type === 'completed' || s?.type === 'canceled';

export function teamStates(doc: Doc, teamId: string): State[] {
  return Object.values(doc.state).filter((s) => s.teamId === teamId).sort((a, b) => STATE_TYPES.indexOf(a.type) - STATE_TYPES.indexOf(b.type) || a.position - b.position);
}

export const issueKey = (doc: Doc, i: Issue) => `${doc.team[i.teamId]?.key ?? '?'}-${i.number}`;

export function findByKey(doc: Doc, key: string): Issue | undefined {
  const m = /^([A-Z][A-Z0-9]*)-(\d+)$/i.exec(key.trim());
  if (!m) return undefined;
  const team = Object.values(doc.team).find((t) => t.key === m[1].toUpperCase());
  return team && Object.values(doc.issue).find((i) => i.teamId === team.id && i.number === Number(m[2]));
}

/** Today as 2026-11-04, in local time. */
export function today(now = new Date()): string {
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
}

export function addDays(day: string, n: number): string {
  const d = new Date(`${day}T12:00:00`);
  d.setDate(d.getDate() + n);
  return today(d);
}

export function cycleStatus(c: Cycle, now = today()): 'past' | 'current' | 'upcoming' {
  return now > c.endsAt ? 'past' : now < c.startsAt ? 'upcoming' : 'current';
}

export const cycleName = (c: Cycle) => c.name || `Cycle ${c.number}`;

/** How far along a set of issues is: done over all, counting estimates when there are any. */
export function progress(doc: Doc, issues: Issue[]): { done: number; total: number; started: number } {
  const w = (i: Issue) => i.estimate ?? 1;
  let done = 0, total = 0, started = 0;
  for (const i of issues) {
    const s = doc.state[i.stateId];
    if (s?.type === 'canceled') continue;
    total += w(i);
    if (s?.type === 'completed') done += w(i);
    else if (s?.type === 'started' || s?.type === 'review') started += w(i);
  }
  return { done, total, started };
}

/** The prompt a Scout agent gets for an issue. */
export function agentPrompt(doc: Doc, i: Issue): string {
  const labels = i.labelIds.map((l) => doc.label[l]?.name).filter(Boolean);
  const comments = Object.values(doc.comment).filter((c) => c.issueId === i.id).sort((a, b) => a.createdAt - b.createdAt);
  return [
    `Work on Delta issue ${issueKey(doc, i)}: ${i.title}`,
    '',
    i.description.trim() || '(No description.)',
    labels.length ? `\nLabels: ${labels.join(', ')}` : '',
    PRIORITIES[i.priority] && i.priority ? `Priority: ${PRIORITIES[i.priority]}` : '',
    comments.length ? `\nComments:\n${comments.map((c) => `- ${c.author}: ${c.body}`).join('\n')}` : '',
    '',
    'Make the change in this project, check that it works, and finish with a short summary of what you changed.',
  ].filter((l, n, a) => l !== '' || a[n - 1] !== '').join('\n').trim();
}
