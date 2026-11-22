// Dreaming: once Scout has sat untouched for 20 minutes, and at most once a day, a free model reads the last two days of
// chats and tidies user and project memory (see dream.rs). This is the Dream Log, in Settings and in the Inbox.
import { useEffect, useState } from 'react';
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

/** The Inbox's dreams: opening the page marks them seen, but they stay listed until you leave it or dismiss them. */
export function useInboxDreams() {
  const d = useView('dreaming');
  const unseen = useUnseenDreams();
  const [shown, setShown] = useState<string[]>([]);
  const fresh = unseen.map((x) => x.id).join();
  useEffect(() => {
    if (!unseen.length) return;
    setShown((prev) => [...unseen.map((x) => x.id).filter((id) => !prev.includes(id)), ...prev]);
    unseen.forEach((x) => act('dreamSeen', { id: x.id }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fresh]);
  const dreams = (d?.log ?? []).filter((x) => shown.includes(x.id));
  return {
    dreams,
    dismiss: (id: string) => setShown((prev) => prev.filter((x) => x !== id)),
    dismissAll: () => setShown([]),
  };
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
export function DreamEntry({ d, onDismiss }: { d: Dream; onDismiss?: () => void }) {
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
        {onDismiss && <button type="button" className="button compact" onClick={onDismiss}>Dismiss</button>}
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
