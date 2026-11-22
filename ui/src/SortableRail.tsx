import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

/** How long a press has to be held before the icon lifts and can be dragged. */
const HOLD_MS = 350;
/** Moving further than this before the hold ends counts as a scroll, not a drag. */
const SLOP = 5;
const LEAVE_MS = 260;
const ENTER_MS = 380;

type Drag<T> = { id: T; from: number; to: number; dy: number };
type Press<T> = { id: T; x: number; y: number; pointer: number; el: HTMLElement; slot: number; timer: number };

function move<T>(list: T[], from: number, to: number): T[] {
  const next = [...list];
  const [x] = next.splice(from, 1);
  next.splice(to, 0, x);
  return next;
}

/**
 * A vertical list whose items are pressed and held to lift, then dragged into place while the others slide out of the
 * way. Items added grow in and items removed shrink out, so the list never jumps.
 */
export function SortableRail<T extends string>({ items, onReorder, render, className = '', children }: {
  items: T[];
  onReorder: (next: T[]) => void;
  render: (id: T, leaving: boolean) => ReactNode;
  className?: string;
  /** Shown after the items, not draggable. */
  children?: ReactNode;
}) {
  const els = useRef(new Map<T, HTMLDivElement>());
  const prev = useRef<T[]>(items);
  const shownRef = useRef<T[]>(items);
  const [leaving, setLeaving] = useState<T[]>([]);
  const [entering, setEntering] = useState<T[]>([]);
  const [pressing, setPressing] = useState<T | null>(null);
  const [drag, setDrag] = useState<Drag<T> | null>(null);
  const [snap, setSnap] = useState<{ id: T; offset: number } | null>(null);
  const press = useRef<Press<T> | null>(null);
  const dragRef = useRef<Drag<T> | null>(null);
  const slotRef = useRef(34);
  // A drag ends with a click on the same icon, which mustn't open it.
  const dragged = useRef(false);

  // Keep what was removed on screen while it shrinks away, and mark what's new so it grows in.
  useLayoutEffect(() => {
    const before = prev.current;
    prev.current = items;
    const gone = before.filter((x) => !items.includes(x));
    const added = items.filter((x) => !before.includes(x));
    setLeaving((l) => {
      const kept = l.filter((x) => !items.includes(x));
      return gone.length || kept.length !== l.length ? [...kept, ...gone] : l;
    });
    if (gone.length) setTimeout(() => setLeaving((l) => l.filter((x) => !gone.includes(x))), LEAVE_MS);
    if (added.length) {
      setEntering((e) => [...e, ...added]);
      setTimeout(() => setEntering((e) => e.filter((x) => !added.includes(x))), ENTER_MS);
    }
  }, [items]);

  // After a drop, the lifted icon sits where the pointer left it with transitions off; next frame it glides home.
  useLayoutEffect(() => {
    if (!snap) return;
    void els.current.get(snap.id)?.offsetHeight;
    let b = 0;
    const a = requestAnimationFrame(() => { b = requestAnimationFrame(() => setSnap(null)); });
    return () => { cancelAnimationFrame(a); cancelAnimationFrame(b); };
  }, [snap]);

  // Items still leaving go back where they were, after whatever came before them.
  const shown: T[] = [...items];
  for (const id of leaving) {
    if (shown.includes(id)) continue;
    const old = shownRef.current;
    let at = 0;
    for (let j = old.indexOf(id) - 1; j >= 0; j--) {
      const k = shown.indexOf(old[j]);
      if (k >= 0) { at = k + 1; break; }
    }
    shown.splice(at, 0, id);
  }
  shownRef.current = shown;

  const slot = () => {
    const [a, b] = items.map((i) => els.current.get(i)).filter((e): e is HTMLDivElement => !!e);
    if (a && b) return Math.abs(b.offsetTop - a.offsetTop) || a.offsetHeight + 2;
    return (a?.offsetHeight ?? 32) + 2;
  };

  const end = () => {
    const p = press.current;
    if (p) clearTimeout(p.timer);
    press.current = null;
    setPressing(null);
    const d = dragRef.current;
    if (!d) return;
    dragRef.current = null;
    setSnap({ id: d.id, offset: d.dy - (d.to - d.from) * slotRef.current });
    setDrag(null);
    if (d.to !== d.from) onReorder(move(items, d.from, d.to));
  };

  return (
    <div
      className={`rail sortable ${drag ? 'dragging' : ''} ${snap ? 'snap' : ''} ${className}`}
      onClickCapture={(e) => {
        if (!dragged.current) return;
        dragged.current = false;
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {shown.map((id) => {
        const gone = leaving.includes(id) && !items.includes(id);
        let ty = 0;
        if (drag) {
          const i = items.indexOf(id);
          if (id === drag.id) ty = drag.dy;
          else if (drag.from < drag.to && i > drag.from && i <= drag.to) ty = -slotRef.current;
          else if (drag.to < drag.from && i >= drag.to && i < drag.from) ty = slotRef.current;
        }
        if (snap?.id === id) ty = snap.offset;
        const cls = [
          'rail-item',
          gone && 'leaving',
          entering.includes(id) && 'entering',
          pressing === id && 'pressing',
          drag?.id === id && 'lifted',
        ].filter(Boolean).join(' ');
        return (
          <div
            key={id}
            data-id={id}
            className={cls}
            ref={(el) => { if (el) els.current.set(id, el); else els.current.delete(id); }}
            style={ty ? { transform: `translateY(${ty}px)` } : undefined}
            onPointerDown={(e) => {
              if (e.button !== 0 || gone || items.length < 2) return;
              dragged.current = false;
              const el = e.currentTarget;
              const pointer = e.pointerId;
              const timer = window.setTimeout(() => {
                const p = press.current;
                if (!p) return;
                slotRef.current = p.slot;
                try { el.setPointerCapture(pointer); } catch { /* the pointer already left */ }
                setPressing(null);
                const from = items.indexOf(id);
                const d = { id, from, to: from, dy: 0 };
                dragRef.current = d;
                dragged.current = true;
                setDrag(d);
              }, HOLD_MS);
              press.current = { id, x: e.clientX, y: e.clientY, pointer, el, slot: slot(), timer };
              setPressing(id);
            }}
            onPointerMove={(e) => {
              const p = press.current;
              if (!p || e.pointerId !== p.pointer) return;
              const d = dragRef.current;
              if (!d) {
                if (Math.abs(e.clientY - p.y) > SLOP || Math.abs(e.clientX - p.x) > SLOP) end();
                return;
              }
              const s = slotRef.current;
              const dy = Math.max(-d.from * s - 8, Math.min((items.length - 1 - d.from) * s + 8, e.clientY - p.y));
              const to = Math.max(0, Math.min(items.length - 1, d.from + Math.round(dy / s)));
              const next = { ...d, dy, to };
              dragRef.current = next;
              setDrag(next);
            }}
            onPointerUp={end}
            onPointerCancel={end}
            onLostPointerCapture={() => { if (dragRef.current) end(); }}
          >
            {render(id, gone)}
          </div>
        );
      })}
      {children}
    </div>
  );
}
