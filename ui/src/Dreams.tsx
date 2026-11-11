// Dreaming: once Scout has sat untouched for 20 minutes, and at most once a day, a free model reads the last two days of
// chats and tidies user and project memory (see dream.rs). This is the Dream Log, in Settings and in the Inbox.
import { useEffect } from 'react';
import { act } from './api';
import { useView } from './store';
import { Icon, Spinner } from './widgets';

export type DreamChange = { memory: 'user' | 'project'; project?: string; action: 'add' | 'replace' | 'remove'; section: string; text: string; oldText: string };
export type Dream = { id: string; started: number; finished: number; status: 'done' | 'failed'; error: string | null; chats: string[]; changes: DreamChange[]; seen: boolean; undone: boolean };
export type DreamingView = { on: boolean | null; running: boolean; model: string | null; last: number; log: Dream[]; message: { ok: boolean; text: string } | null };

/** How long the window must go untouched, and how often a ping says it was touched. */
const PING_MS = 60_000;

/** Tells the core whenever the window is used, at most once a minute, so Dreaming knows when the user is away. */
export function useActivityPing() {
  useEffect(() => {
    let last = 0;
    const ping = () => {
      const now = Date.now();
      if (now - last < PING_MS) return;
      last = now;
      act('activity');
    };
    const events = ['keydown', 'pointerdown', 'pointermove', 'wheel'] as const;
    events.forEach((e) => window.addEventListener(e, ping, { passive: true }));
    return () => events.forEach((e) => window.removeEventListener(e, ping));
  }, []);
}

/** Dreams that changed memory and haven't been dismissed: what the Inbox shows. */
export function useUnseenDreams(): Dream[] {
  const d = useView('dreaming');
  return (d?.log ?? []).filter((x) => !x.seen && !x.undone && x.status === 'done' && x.changes.length > 0);
}

function when(ms: number) {
  return new Date(ms).toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' });
}

function ChangeLine({ c }: { c: DreamChange }) {
  const where = c.memory === 'user' ? 'You' : c.project ?? 'Project';
  const verb = c.action === 'add' ? 'Remembered' : c.action === 'replace' ? 'Updated' : 'Forgot';
  const text = (c.action === 'remove' ? c.oldText : c.text).replace(/^[-*]\s*/, '');
  return (
    <li className="dream-change">
      <span className="tiny dim dream-where">{verb} · {where}{c.section ? ` · ${c.section}` : ''}</span>
      <span className={`small${c.action === 'remove' ? ' dream-removed' : ''}`}>{text}</span>
    </li>
  );
}

/** One dream: when, what it read, what it changed, with Undo, and Dismiss when it's in the Inbox. */
export function DreamEntry({ d, inbox }: { d: Dream; inbox?: boolean }) {
  return (
    <div className={`dream-entry${d.undone ? ' undone' : ''}`}>
      <div className="row dream-head">
        <Icon name="sparkle" size={14} />
        <span className="small strong grow">
          {d.status === 'failed' ? 'Dream stopped' : d.changes.length ? `${d.changes.length} memory ${d.changes.length === 1 ? 'change' : 'changes'}` : 'Nothing lasting to change'}
          <span className="tiny dim"> · {when(d.started)}</span>
        </span>
        {d.undone && <span className="tiny dim">Undone</span>}
        {!d.undone && d.changes.length > 0 && <button type="button" className="button compact" onClick={() => act('dreamUndo', { id: d.id })}>Undo</button>}
        {inbox && <button type="button" className="button compact" onClick={() => act('dreamSeen', { id: d.id })}>Dismiss</button>}
      </div>
      {d.error && <div className="small red">{d.error}</div>}
      {d.changes.length > 0 && <ul className="dream-changes">{d.changes.map((c, i) => <ChangeLine key={i} c={c} />)}</ul>}
      {d.chats.length > 0 && <div className="tiny dim">From {d.chats.length} {d.chats.length === 1 ? 'chat' : 'chats'}: {d.chats.slice(0, 4).join(' · ')}{d.chats.length > 4 ? ' …' : ''}</div>}
    </div>
  );
}

/** "Dreaming…" while a dream runs. */
export function DreamingNow() {
  const d = useView('dreaming');
  if (!d?.running) return null;
  return (
    <div className="row dream-now small dim">
      <Spinner size={11} /> Dreaming: reading recent chats and tidying memory…
    </div>
  );
}
