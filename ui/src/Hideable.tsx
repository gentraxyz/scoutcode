// Parts of the window you can hide with a right-click, and the menu that does it. Kept on this PC.
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { MenuItem } from './widgets';

/** Every part that can be hidden, in the order Settings lists them. */
export const HIDEABLE = [
  ['top.files', 'Files button', 'Top bar'],
  ['top.changes', 'Changes button', 'Top bar'],
  ['top.terminal', 'Terminal button', 'Top bar'],
  ['top.panel', 'Panel button', 'Top bar'],
  ['top.game', 'Game button', 'Top bar'],
  ['top.music', 'Music button', 'Top bar'],
  ['top.project', 'Project folder', 'Top bar'],
  ['side.delta', 'Delta', 'Sidebar'],
  ['side.search', 'Search chats', 'Sidebar'],
  ['side.newChat', 'New chat button', 'Sidebar'],
] as const;
export type HideableId = (typeof HIDEABLE)[number][0];

const KEY = 'scout.hiddenUi';
const subs = new Set<() => void>();
let hidden: HideableId[] = (() => {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((id): id is HideableId => HIDEABLE.some(([h]) => h === id)) : [];
  } catch {
    return [];
  }
})();

function save(next: HideableId[]) {
  hidden = next;
  localStorage.setItem(KEY, JSON.stringify(next));
  subs.forEach((f) => f());
}
export const hideUi = (id: HideableId) => { if (!hidden.includes(id)) save([...hidden, id]); };
export const showUi = (id: HideableId) => save(hidden.filter((h) => h !== id));
export const showAllUi = () => save([]);

export function useHidden(): HideableId[] {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f); }, () => hidden);
}

// One right-click menu for the whole window.
type Menu = { id: HideableId; x: number; y: number };
let menu: Menu | null = null;
const menuSubs = new Set<() => void>();
const setMenu = (m: Menu | null) => { menu = m; menuSubs.forEach((f) => f()); };

/** Wraps a part of the window so a right-click on it offers to hide it, and leaves it out once hidden. */
export function Hideable({ id, children }: { id: HideableId; children: ReactNode }) {
  const isHidden = useHidden().includes(id);
  if (isHidden) return null;
  return (
    <span className="hideable" onContextMenu={(e) => {
      // Text fields keep their own menu, for copy and paste.
      if ((e.target as HTMLElement).closest('input, textarea')) return;
      e.preventDefault(); e.stopPropagation(); setMenu({ id, x: e.clientX, y: e.clientY }); }}>
      {children}
    </span>
  );
}

export function HideMenu() {
  const m = useSyncExternalStore((f) => { menuSubs.add(f); return () => menuSubs.delete(f); }, () => menu);
  const panel = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    if (!m || !panel.current) return setPos(null);
    const p = panel.current.getBoundingClientRect();
    setPos({ left: Math.max(8, Math.min(m.x, window.innerWidth - p.width - 8)), top: Math.max(8, Math.min(m.y, window.innerHeight - p.height - 8)) });
  }, [m]);
  useEffect(() => {
    if (!m) return;
    const down = (e: MouseEvent) => { if (!panel.current?.contains(e.target as Node)) setMenu(null); };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setMenu(null); } };
    const away = () => setMenu(null);
    document.addEventListener('mousedown', down, true);
    window.addEventListener('keydown', key, true);
    window.addEventListener('blur', away);
    window.addEventListener('resize', away);
    return () => {
      document.removeEventListener('mousedown', down, true);
      window.removeEventListener('keydown', key, true);
      window.removeEventListener('blur', away);
      window.removeEventListener('resize', away);
    };
  }, [m]);
  if (!m) return null;
  const name = HIDEABLE.find(([h]) => h === m.id)![1];
  return (
    <div ref={panel} className="popover hide-menu" style={{ width: 220, ...(pos ?? { left: m.x, top: m.y, visibility: 'hidden' }) }} onContextMenu={(e) => e.preventDefault()}>
      <MenuItem icon="eyeOff" onClick={() => { hideUi(m.id); setMenu(null); }}>Hide {name}</MenuItem>
      <div className="tiny dim hide-menu-note">Bring it back in Settings › Appearance.</div>
    </div>
  );
}
