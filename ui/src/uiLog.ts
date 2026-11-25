// Sends the window's errors, warnings and crashes to the session log (src/logs.rs), so Settings > Logs has them too.
import { invoke } from './api';

type Level = 'info' | 'warn' | 'error';
let queue: [Level, string][] = [];
let timer: ReturnType<typeof setTimeout> | null = null;
// A failing send mustn't log its own failure, round and round.
let sending = false;

function text(parts: unknown[]): string {
  return parts.map((p) => {
    if (p instanceof Error) return `${p.name}: ${p.message}${p.stack ? `\n${p.stack}` : ''}`;
    if (typeof p === 'string') return p;
    try { return JSON.stringify(p); } catch { return String(p); }
  }).join(' ');
}

function flush() {
  timer = null;
  if (!queue.length) return;
  const entries = queue;
  queue = [];
  sending = true;
  invoke('log_ui', { entries }).catch(() => {}).finally(() => { sending = false; });
}

/** Adds a line to the session log. Sent in batches, at once for errors. */
export function uiLog(level: Level, ...parts: unknown[]) {
  if (sending) return;
  queue.push([level, text(parts)]);
  if (queue.length > 200) queue.splice(0, queue.length - 200);
  if (level === 'error') flush();
  else if (!timer) timer = setTimeout(flush, 1000);
}

/** Starts sending uncaught errors, rejected promises and console errors and warnings. Once, at startup. */
export function startUiLog() {
  window.addEventListener('error', (e) => uiLog('error', `Uncaught: ${e.message} at ${e.filename}:${e.lineno}:${e.colno}`, e.error ?? ''));
  window.addEventListener('unhandledrejection', (e) => uiLog('error', 'Unhandled rejection:', e.reason));
  for (const level of ['error', 'warn'] as const) {
    const original = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      original(...args);
      uiLog(level, ...args);
    };
  }
  window.addEventListener('beforeunload', flush);
  uiLog('info', `UI loaded: ${navigator.userAgent}`);
}
