// The shapes `src/core.rs` and `src/bridge.rs` send, and how to call them.
import { invoke } from '@tauri-apps/api/core';

export type DiffRow = { kind: 'add' | 'del' | 'same'; text: string; no: number | null } | null;
export type DiffLines = { rows: DiffRow[]; more: number };
export type Todo = { content: string; status: string };
export type Display =
  | { type: 'text'; text: string }
  | { type: 'diff'; path: string; created: boolean; add: number; del: number; lines: DiffLines }
  | { type: 'diffs'; diffs: Display[] }
  | { type: 'todos'; todos: Todo[] }
  | { type: 'image'; path: string; preview: string };

export type ToolResult = { isError: boolean; summary: string | null; display: Display };
export type Item =
  | { id: number; kind: 'user'; text: string; files: string[] }
  | { id: number; kind: 'assistant'; text: string; reasoning: string }
  | { id: number; kind: 'tool'; name: string; label: string; progress: string; result: ToolResult | null }
  | { id: number; kind: 'notice' | 'error' | 'outOfCredit'; text: string }
  | { id: number; kind: 'fallbackSetup'; text: string; key: string; provider: string };

/** A model in the picker: its list key and where it runs. */
export type ModelRef = { key: string; provider: string };
/** The models to try, in order, when the chosen one fails. */
export type Fallbacks = { chosen: boolean; all: ModelRef[]; models: [ModelRef, ModelRef[]][] };

export type Model = {
  id: string; label: string; description: string | null; context: number | null; tools: boolean | null; vision: boolean | null; cost: string | null;
  providers: string[]; routesTo: string[]; available: boolean | null; openrouterId: string | null; nvidiaId: string | null; opencodeId: string | null; kiloId: string | null; kiloFree: boolean;
  kiloTier: number | null; copilotId: string | null; devinId: string | null; kiroId: string | null; codexId: string | null; claudeId: string | null; grokId: string | null; hermesId: string | null; additional: [string, string] | null; hosted: boolean; free: boolean;
  tier: number | null; openrouterTier: number | null;
  /** Free on OpenRouter, for a model ScoutCode serves too; `free` is ScoutCode's own price then. */
  openrouterFree: boolean;
};

export type Permission = { tool: string; label: string; preview: Display | null; prefix: string | null; reason: string | null };
export type Question = { question: string; header: string; multiSelect: boolean; options: { label: string; description: string }[] };
export type SignIn = { code: string | null; url: string | null; done: number; total: number; installed: boolean; waiting: boolean };
export type Plan = { limit: number; left: number };
export type ManagedUser = {
  username: string; mode: string; limits: { monthly: number | null } | null; cliMonthly: number; cliLimit: number | null;
  balance: number; banked: number; spentThisMonth: number; harnessSpentThisMonth: number;
};
export type UsageInfo = {
  username: string; admin: boolean; resetsAt: string; money: Plan | null; cli: Plan | null; balance: number;
  bankedMoney: { amount: number; expiresAt: string }[]; bankedResets: { expiresAt: string }[]; users: ManagedUser[];
};
/** A file a linked chatbot changed, with the lines added and removed. */
export type LinkEdit = { path: string; added: number; removed: number; created: boolean };
/** One tool call a linked chatbot made, finished at `at` (ms since 1970). */
export type LinkCall = { tool: string; label: string; ok: boolean; at: number; edits: LinkEdit[] };

export type McpServer = {
  name: string; summary: string; disabled: boolean; state: 'off' | 'connecting' | 'ready' | 'failed'; error: string | null; tools: string[];
  form: { remote: boolean; command: string; args: string; env: string; url: string; headers: string };
};
export type UpdateStatus =
  | { state: 'idle' | 'checking' | 'upToDate' }
  | { state: 'available' | 'ready'; version: string; summary: string; changelog: string }
  | { state: 'downloading'; version: string; got: number; total: number }
  | { state: 'failed'; error: string };

/** Experimental games: where they show, and whether they open and close with Scout's work. */
export type Games = { on: boolean; place: 'float' | 'panel'; openOnRun: boolean; autoClose: boolean; pos: [number, number] | null };

/** The Scout Enterprise team the account is in: the providers it runs through the server, and the ones on this PC it allows. */
export type Team = { name: string; role: string; providers: string[]; acp: string[] };

export type View = {
  cfg: {
    loginNeeded: boolean; team: Team | null; hasKey: boolean; guest: boolean; browserSignIn: boolean; username: string | null; baseUrl: string; openrouterKeys: { name: string; masked: string }[];
    nvidiaKey: string | null; kiloKey: string | null; kilo: boolean; copilot: boolean; copilotUser: string | null; devin: boolean; kiro: boolean; codex: boolean; claude: boolean; opencode: boolean; opencodeSignedIn: boolean; grokSignedIn: boolean; grokKey: string | null; hermesSignedIn: boolean; hermesKey: string | null;
    additional: { id: string; masked: string | null }[]; notifyFlash: boolean | null; notifyToast: boolean | null; discordPresence: boolean; discordProject: boolean; autoUpdate: boolean; autoCompactTokens: number; fallbacks: Fallbacks; experimentalImages: boolean; imageModel: string | null; games: Games;
  };
  mode: string;
  model: { id: string; provider: string; key: string; short: string };
  effort: string;
  starred: string[];
  modelsLoading: number;
  additionalErrors: string[];
  kiroModelsError: string | null;
  codexModelsError: string | null;
  claudeModelsError: string | null;
  opencodeModelsError: string | null;
  grokModelsError: string | null;
  hermesModelsError: string | null;
  run: { running: boolean; stopping: boolean; status: string; anyRunning: boolean };
  todos: Todo[];
  totals: { totals: { promptTokens: number; completionTokens: number; cost: number; requests: number; balance?: number; cachedTokens: number }; context: number };
  permission: Permission | null;
  plan: string | null;
  questions: Question[] | null;
  queue: { text: string; files: number }[];
  steering: string[];
  sessions: { id: string; cwd: string; title: string; ago: string; running: boolean; waiting: boolean; unread: boolean }[];
  projects: { name: string; path: string }[];
  chat: { id: string | null; cwd: string; project: string; title: string; epoch: number };
  filesEpoch: number;
  focusEpoch: number;
  login: {
    busy: boolean; url: string | null; error: string | null; kiloCheck: { ok: boolean; message: string } | null;
    copilot: SignIn | null; copilotError: string | null; devin: SignIn | null; devinError: string | null; kiro: SignIn | null; kiroError: string | null; codex: SignIn | null; codexError: string | null; claude: SignIn | null; claudeError: string | null; opencode: SignIn | null; opencodeError: string | null; grok: SignIn | null; grokError: string | null; hermes: SignIn | null; hermesError: string | null;
  };
  skills: { name: string; about: string; source: string | null; dir: string }[];
  link: {
    on: boolean; online: boolean; cwd: string; status: string; mode: string | null; since: number | null; running: string | null;
    log: LinkCall[]; edits: LinkEdit[]; chatbots: { name: string; seen: number | null }[]; permission: Permission | null;
  };
  scoutMote: { on: boolean; running: boolean; status: string; online: boolean; name: string };
  usage: { info: UsageInfo | null; busy: boolean; message: [boolean, string] | null };
  voice: { state: 'idle' } | { state: 'recording'; level: number } | { state: 'busy'; label: string };
  updater: { enabled: boolean; version: string; status: UpdateStatus };
  /** The releases since the version that ran before this one, newest first. Empty unless the app was just updated. */
  whatsNew: { version: string; summary: string; changelog: string }[];
  mcp: { error: string | null; servers: McpServer[] };
};

export type Statics = {
  modes: { id: string; label: string }[];
  efforts: string[];
  additionalProviders: { id: string; name: string; needsKey: boolean; note: string; keyUrl: string; keyLink: string; keyHint: string }[];
  kiloWarning: string;
  plans: { copilot: string; devin: string; kiro: string; codex: string; claude: string; opencode: string; grok: string; hermes: string };
  languages: [string, string][];
  exe: string;
};

export type Snapshot = Statics & { view: View; models: Model[]; items: Item[] };

/** Asks the core to do something. What comes of it arrives as a `view` event. */
export function act(type: string, args: Record<string, unknown> = {}): void {
  invoke('act', { action: { type, ...args } }).catch((e) => console.error(type, e));
}

/** Opens a web page in the browser. */
export function openExternal(url: string): void {
  invoke('open_url', { url }).catch((e) => console.error(e));
}

export { invoke };

/** "$0.50", "$12.00", "$0.0015". */
export function usd(v: number): string {
  const cents = v.toFixed(2);
  return Math.abs(Number(cents) - v) < 1e-9 ? `$${cents}` : `$${money(v)}`;
}

/** Up to four decimals, without trailing zeros. */
export function money(v: number): string {
  return v.toFixed(4).replace(/0+$/, '').replace(/\.$/, '');
}

/** A USD amount as typed ("1.5", "$2"), or null when it isn't one. */
export function parseUsd(s: string): number | null {
  const t = s.trim().replace(/^\$+/, '');
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) && n >= 0 ? n : null;
}

/** 131072 → "131K", 1048576 → "1M". */
export function tokens(n: number): string {
  if (n >= 1_000_000) return `${Math.round((n / 1_000_000) * 10) / 10}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}K`;
  return String(n);
}

export function shortDate(day: string): string {
  const [, m, d] = day.split('-').map(Number);
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return m && d ? `${months[m - 1]} ${d}` : day;
}
