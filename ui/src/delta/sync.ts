/**
 * Where a Delta workspace lives and how changes get there. The UI holds one open Session at a time; it changes its own
 * copy of the workspace at once and then saves: to a file on this PC for a local workspace (desktop app only), or to the
 * ScoutCode server for a cloud workspace, which also checks the change and sends back what others changed.
 */
import { applyAll, emptyDoc, fromRecords, merge, newId, OpError, seededIds, starterOps, toRecords, type Doc, type Op, type Rec } from './model';

export type Storage = 'local' | 'cloud';
export type Role = 'owner' | 'admin' | 'member';
export interface Member { username: string; role: Role }
export interface WorkspaceInfo { id: string; name: string; storage: Storage; role: Role; members: number }

/** A project folder Scout can work in, on this PC or on one of the account's computers. */
export interface AgentTarget { pc: string | null; pcName: string; cwd: string; project: string; online: boolean }

/** What the app around Delta provides. */
export interface Host {
  kind: 'desktop' | 'web';
  /** The account's name, or null when not signed in (local workspaces still work). */
  me: string | null;
  /** Calls the server's Delta API as the signed-in account. */
  cloud(method: 'GET' | 'POST', path: string, body?: unknown): Promise<any>;
  /** Local workspaces, on the desktop only. */
  local?: {
    list(): Promise<{ id: string; name: string }[]>;
    read(id: string): Promise<string | null>;
    write(id: string, text: string): Promise<void>;
    remove(id: string): Promise<void>;
  };
  /** Hands issues to Scout. */
  agent?: {
    targets(): Promise<AgentTarget[]>;
    start(target: AgentTarget, prompt: string): Promise<{ chatId: string | null }>;
    /** Opens the chat working an issue, when the host can. */
    open?(run: { pc: string | null; chatId: string | null; cwd: string }): void;
  };
  /** Leaves Delta for the chat, after starting Scout on an issue. */
  showChat?(): void;
}

const API = '/api/account/delta';

export async function listWorkspaces(host: Host): Promise<{ list: WorkspaceInfo[]; cloudError: string | null }> {
  const list: WorkspaceInfo[] = [];
  let cloudError: string | null = null;
  if (host.local) {
    for (const w of await host.local.list().catch(() => [])) list.push({ id: w.id, name: w.name, storage: 'local', role: 'owner', members: 1 });
  }
  if (host.me) {
    try {
      const r = await host.cloud('GET', API);
      for (const w of r.workspaces ?? []) list.push({ ...w, storage: 'cloud' });
    } catch (e) {
      cloudError = message(e);
    }
  } else cloudError = 'Sign in to your ScoutCode account to use cloud workspaces.';
  return { list, cloudError };
}

interface LocalFile { meta: { id: string; name: string; storage: 'local'; updatedAt: number }; records: Rec[] }

export async function createWorkspace(host: Host, name: string, storage: Storage, team = 'Engineering', key = 'ENG'): Promise<WorkspaceInfo> {
  const ops = starterOps(team, key);
  if (storage === 'local') {
    if (!host.local) throw new Error('Local workspaces are only in the desktop app.');
    const id = newId();
    const doc = emptyDoc(id, name);
    applyAll(doc, ops, { actor: host.me ?? 'you', now: Date.now(), members: null });
    await writeLocal(host, doc);
    return { id, name, storage, role: 'owner', members: 1 };
  }
  const r = await host.cloud('POST', API, { action: 'create', name, ops, seed: newId() });
  return { ...r.workspace, storage: 'cloud' };
}

async function writeLocal(host: Host, doc: Doc) {
  const file: LocalFile = { meta: { id: doc.workspace.id, name: doc.workspace.name, storage: 'local', updatedAt: Date.now() }, records: toRecords(doc) };
  await host.local!.write(doc.workspace.id, JSON.stringify(file));
}

/** Copies a local workspace into the cloud, as a new workspace. The local one is kept. */
export async function uploadWorkspace(host: Host, doc: Doc): Promise<WorkspaceInfo> {
  const r = await host.cloud('POST', API, { action: 'import', name: doc.workspace.name, records: toRecords(doc).filter((x) => x.kind !== 'workspace') });
  return { ...r.workspace, storage: 'cloud' };
}

export function message(e: unknown): string {
  return e instanceof Error ? e.message : typeof e === 'string' ? e : 'Something went wrong.';
}

/** An open workspace. Components re-render when `version` changes; `doc` is changed in place. */
export class Session {
  doc: Doc;
  members: Member[];
  role: Role;
  version = 0;
  error: string | null = null;
  saving = false;
  private subs = new Set<() => void>();
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private queue: Promise<void> = Promise.resolve();
  private pending = 0;
  private closed = false;

  private constructor(readonly host: Host, readonly info: WorkspaceInfo, doc: Doc, members: Member[], role: Role) {
    this.doc = doc;
    this.members = members;
    this.role = role;
  }

  get me(): string {
    return this.host.me ?? 'you';
  }

  static async open(host: Host, info: WorkspaceInfo): Promise<Session> {
    if (info.storage === 'local') {
      const text = await host.local?.read(info.id);
      if (!text) throw new Error('This workspace file is gone.');
      const file = JSON.parse(text) as LocalFile;
      const doc = fromRecords(info.id, file.records);
      return new Session(host, info, doc, [{ username: host.me ?? 'you', role: 'owner' }], 'owner');
    }
    const r = await host.cloud('GET', `${API}/${info.id}`);
    const s = new Session(host, info, fromRecords(info.id, r.records), r.members, r.role);
    s.seq = r.seq;
    s.poll();
    return s;
  }

  subscribe = (f: () => void) => {
    this.subs.add(f);
    return () => {
      this.subs.delete(f);
    };
  };
  getVersion = () => this.version;
  private changed() {
    this.version++;
    this.subs.forEach((f) => f());
  }

  close() {
    this.closed = true;
    if (this.timer) clearTimeout(this.timer);
    this.subs.clear();
  }

  /** Who can be assigned: the members, and Scout. */
  get people(): string[] {
    return this.members.map((m) => m.username);
  }

  get canManage(): boolean {
    return this.role === 'owner' || this.role === 'admin';
  }

  /**
   * Makes changes. They show at once; a cloud workspace then sends them, and puts back the server's copy if it refuses.
   * Returns false, with `error` set, when they aren't allowed.
   */
  dispatch(ops: Op | Op[]): boolean {
    const list = Array.isArray(ops) ? ops : [ops];
    if (!list.length) return true;
    const seed = newId();
    const local = this.info.storage === 'local';
    try {
      applyAll(this.doc, list, { actor: this.me, now: Date.now(), members: local ? null : this.people, newId: seededIds(seed) });
    } catch (e) {
      this.error = e instanceof OpError ? e.message : message(e);
      this.changed();
      return false;
    }
    this.error = null;
    this.changed();
    if (local) this.saveLocal();
    else this.send(list, seed);
    return true;
  }

  clearError() {
    this.error = null;
    this.changed();
  }

  private saveLocal() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      writeLocal(this.host, this.doc).catch((e) => {
        this.error = `Couldn't save: ${message(e)}`;
        this.changed();
      });
    }, 250);
  }

  private send(ops: Op[], seed: string) {
    this.pending++;
    this.saving = true;
    this.queue = this.queue.then(async () => {
      try {
        const r = await this.host.cloud('POST', `${API}/${this.info.id}`, { ops, seed, since: this.seq });
        this.take(r);
      } catch (e) {
        this.error = message(e);
        await this.reload();
      } finally {
        this.pending--;
        this.saving = this.pending > 0;
        this.changed();
      }
    });
  }

  private take(r: { seq: number; records: Rec[]; members?: Member[]; role?: Role }) {
    if (r.records?.length) merge(this.doc, r.records);
    if (r.seq) this.seq = Math.max(this.seq, r.seq);
    if (r.members) this.members = r.members;
    if (r.role) this.role = r.role;
  }

  /** Starts again from the server's copy. */
  async reload() {
    try {
      const r = await this.host.cloud('GET', `${API}/${this.info.id}`);
      this.doc = fromRecords(this.info.id, r.records);
      this.seq = r.seq;
      this.members = r.members;
      this.role = r.role;
    } catch (e) {
      this.error = message(e);
    }
    this.changed();
  }

  /** Others' changes, every few seconds, while nothing of ours is on its way. */
  private poll() {
    const tick = async () => {
      if (this.closed) return;
      if (!this.pending && document.visibilityState !== 'hidden') {
        try {
          const r = await this.host.cloud('GET', `${API}/${this.info.id}?since=${this.seq}`);
          if (!this.pending && !this.closed && (r.seq !== this.seq || r.records?.length || JSON.stringify(r.members) !== JSON.stringify(this.members))) {
            this.take(r);
            this.changed();
          }
        } catch {
          // Offline for a moment; the next tick tries again.
        }
      }
      if (!this.closed) this.timer = setTimeout(tick, 3000);
    };
    this.timer = setTimeout(tick, 3000);
  }

  /** Workspace admin, for cloud workspaces. */
  async admin(action: string, args: Record<string, unknown> = {}): Promise<void> {
    const r = await this.host.cloud('POST', `${API}/${this.info.id}`, { action, ...args, since: this.seq });
    this.take(r);
    this.changed();
  }

  /** Deletes the whole workspace: the file, or (owner only) the cloud workspace. */
  async destroy(): Promise<void> {
    if (this.info.storage === 'local') await this.host.local!.remove(this.info.id);
    else await this.host.cloud('POST', `${API}/${this.info.id}`, { action: 'delete' });
    this.close();
  }

  exportJson(): string {
    return JSON.stringify({ workspace: this.doc.workspace, records: toRecords(this.doc) }, null, 2);
  }
}
