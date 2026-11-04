// Delta's small building blocks: icons, menus and pickers. Delta brings its own so it looks the same in the desktop app
// and on the website, which each have their own widgets.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { PRIORITIES, SCOUT, type Doc, type State, type StateType } from './model';

const PATHS: Record<string, string> = {
  plus: 'M8 3v10M3 8h10',
  search: 'M7 12A5 5 0 1 0 7 2a5 5 0 0 0 0 10zM14 14l-3.5-3.5',
  inbox: 'M2 9h3.5l1 2h3l1-2H14M3.5 3h9L14 9v4H2V9z',
  issues: 'M3 3.5h10M3 8h10M3 12.5h7',
  board: 'M2.5 2.5h3v11h-3zM6.5 2.5h3v7h-3zM10.5 2.5h3v9h-3z',
  list: 'M5.5 4h8M5.5 8h8M5.5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5',
  project: 'M2.5 4.5l5.5-2.5 5.5 2.5v7L8 14l-5.5-2.5zM2.5 4.5L8 7l5.5-2.5M8 7v7',
  cycle: 'M13.5 8A5.5 5.5 0 1 1 8 2.5c2 0 3.6 1 4.6 2.5M13 2.5V5h-2.5',
  roadmap: 'M2 4h7M5 8h9M2 12h6',
  gear: 'M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4',
  chevronDown: 'M4 6l4 4 4-4',
  chevronRight: 'M6 4l4 4-4 4',
  close: 'M4 4l8 8M12 4l-8 8',
  check: 'M3 8.5l3.2 3L13 4.5',
  user: 'M8 8a2.75 2.75 0 1 0 0-5.5A2.75 2.75 0 0 0 8 8zM2.5 14c.6-2.6 2.8-4 5.5-4s4.9 1.4 5.5 4',
  users: 'M6 7.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM1.5 13.5c.5-2.3 2.3-3.5 4.5-3.5s4 1.2 4.5 3.5M10.5 2.6a2.5 2.5 0 0 1 0 4.8M12 10.2c1.3.5 2.2 1.6 2.5 3.3',
  tag: 'M2.5 2.5h5l6 6-5 5-6-6zM5.25 5.25h.01',
  calendar: 'M2.5 3.5h11v10h-11zM2.5 6.5h11M5.5 2v3M10.5 2v3',
  trash: 'M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.5 9h6l.5-9',
  more: 'M3.5 8h.01M8 8h.01M12.5 8h.01',
  sparkle: 'M8 2l1.5 4.5L14 8l-4.5 1.5L8 14l-1.5-4.5L2 8l4.5-1.5z',
  cloud: 'M4.5 12.5a3 3 0 0 1-.4-6A4 4 0 0 1 11.8 6a3.25 3.25 0 0 1 .2 6.5z',
  laptop: 'M3.5 3.5h9v7h-9zM1.5 12.5h13',
  link: 'M6.5 9.5l3-3M7 4.5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11.5l-1 1A2.5 2.5 0 0 1 4.5 9l1-1',
  parent: 'M4 2.5v6.5a2 2 0 0 0 2 2h6M9.5 8.5L12 11l-2.5 2.5',
  estimate: 'M8 2.5l5 2.75v5.5L8 13.5l-5-2.75v-5.5zM8 8v5.5M3 5.25L8 8l5-2.75',
  back: 'M10 4L6 8l4 4',
  upload: 'M8 10.5V2.5M5 5.5l3-3 3 3M2.5 10.5v3h11v-3',
  download: 'M8 2.5v8M5 7.5l3 3 3-3M2.5 10.5v3h11v-3',
  copy: 'M5.5 5.5h8v8h-8zM2.5 10.5v-8h8',
  external: 'M9 2.5h4.5V7M13.5 2.5L7.5 8.5M11.5 9.5v4h-9v-9h4',
  comment: 'M2.5 3h11v8h-6l-3 2.5V11h-2z',
};

export function Icon({ name, size = 16, className = '' }: { name: string; size?: number; className?: string }) {
  return (
    <svg className={`d-icon ${className}`} width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name] ?? PATHS.more} />
    </svg>
  );
}

/** A workflow state's mark: an empty, dashed, part-filled or filled circle, as trackers show them. */
export function StateIcon({ state, size = 14 }: { state: Pick<State, 'type' | 'color'> | undefined; size?: number }) {
  const c = state?.color ?? '#8b8d98';
  const type: StateType = state?.type ?? 'unstarted';
  const r = 5.5;
  return (
    <svg className="d-icon" width={size} height={size} viewBox="0 0 14 14" aria-hidden="true">
      {type === 'backlog' && <circle cx="7" cy="7" r={r} fill="none" stroke={c} strokeWidth="1.5" strokeDasharray="2 1.6" />}
      {type === 'unstarted' && <circle cx="7" cy="7" r={r} fill="none" stroke={c} strokeWidth="1.5" />}
      {(type === 'started' || type === 'review') && (
        <>
          <circle cx="7" cy="7" r={r} fill="none" stroke={c} strokeWidth="1.5" />
          <path d={type === 'started' ? 'M7 3.5A3.5 3.5 0 0 1 10.5 7H7z' : 'M7 3.5A3.5 3.5 0 1 1 3.5 7H7z'} fill={c} />
        </>
      )}
      {type === 'completed' && (
        <>
          <circle cx="7" cy="7" r="6.25" fill={c} />
          <path d="M4.3 7.2l1.8 1.8 3.6-3.8" fill="none" stroke="var(--d-bg)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </>
      )}
      {type === 'canceled' && (
        <>
          <circle cx="7" cy="7" r="6.25" fill={c} />
          <path d="M4.9 4.9l4.2 4.2M9.1 4.9l-4.2 4.2" fill="none" stroke="var(--d-bg)" strokeWidth="1.5" strokeLinecap="round" />
        </>
      )}
    </svg>
  );
}

/** Bars for a priority; a filled square with "!" for urgent. */
export function PriorityIcon({ p, size = 14 }: { p: number; size?: number }) {
  if (p === 1) {
    return (
      <svg className="d-icon" width={size} height={size} viewBox="0 0 14 14" aria-hidden="true">
        <rect x="1" y="1" width="12" height="12" rx="3" fill="#f2994a" />
        <path d="M7 3.8v4M7 10.1v.1" stroke="var(--d-bg)" strokeWidth="1.7" strokeLinecap="round" />
      </svg>
    );
  }
  if (p === 0) {
    return (
      <svg className="d-icon" width={size} height={size} viewBox="0 0 14 14" aria-hidden="true">
        {[2.5, 6, 9.5].map((x) => <rect key={x} x={x} y="6.4" width="2" height="1.2" rx=".5" fill="var(--d-dim)" />)}
      </svg>
    );
  }
  const lit = 5 - p; // high 3, medium 2, low 1
  return (
    <svg className="d-icon" width={size} height={size} viewBox="0 0 14 14" aria-hidden="true">
      {[0, 1, 2].map((n) => <rect key={n} x={2 + n * 3.5} y={9 - n * 3} width="2.4" height={3 + n * 3} rx=".6" fill={n < lit ? 'var(--d-text)' : 'var(--d-faint)'} />)}
    </svg>
  );
}

const AVATAR_COLORS = ['#5e6ad2', '#26b5ce', '#f2c94c', '#4cb782', '#eb5757', '#bb87fc', '#f2994a', '#0f7488'];
export function Avatar({ name, size = 18 }: { name: string | null; size?: number }) {
  if (!name) {
    return (
      <span className="d-avatar none" style={{ width: size, height: size }} title="Unassigned">
        <Icon name="user" size={Math.round(size * 0.7)} />
      </span>
    );
  }
  if (name === SCOUT) {
    return <span className="d-avatar scout" style={{ width: size, height: size }} title="Scout"><Icon name="sparkle" size={Math.round(size * 0.68)} /></span>;
  }
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return (
    <span className="d-avatar" style={{ width: size, height: size, background: AVATAR_COLORS[h % AVATAR_COLORS.length], fontSize: Math.round(size * 0.5) }} title={name}>
      {name.charAt(0).toUpperCase()}
    </span>
  );
}

export const personName = (u: string | null) => (u == null ? 'Unassigned' : u === SCOUT ? 'Scout' : u);

export function LabelDot({ color }: { color: string }) {
  return <span className="d-dot" style={{ background: color }} />;
}

/** A menu that opens under (or over) its trigger and closes on a click outside, Escape or a pick. */
export function Menu({ trigger, children, width = 220, align = 'start', className = '' }: {
  trigger: (open: boolean, toggle: () => void) => ReactNode;
  children: (close: () => void) => ReactNode;
  width?: number;
  align?: 'start' | 'end';
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLSpanElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const [up, setUp] = useState(false);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', down);
    window.addEventListener('keydown', key, true);
    return () => {
      document.removeEventListener('mousedown', down);
      window.removeEventListener('keydown', key, true);
    };
  }, [open]);
  useLayoutEffect(() => {
    if (!open || !wrap.current || !pop.current) return;
    const r = wrap.current.getBoundingClientRect();
    const h = pop.current.offsetHeight;
    setUp(r.bottom + h + 8 > window.innerHeight && r.top > window.innerHeight - r.bottom);
  }, [open]);
  return (
    <span className={`d-menu-wrap ${className}`} ref={wrap}>
      {trigger(open, () => setOpen((o) => !o))}
      {open && (
        <div ref={pop} className={`d-menu ${up ? 'up' : ''} ${align}`} style={{ width }}>
          {children(() => setOpen(false))}
        </div>
      )}
    </span>
  );
}

export function MenuItem({ children, onClick, selected, danger, disabled }: { children: ReactNode; onClick?: () => void; selected?: boolean; danger?: boolean; disabled?: boolean }) {
  return (
    <button type="button" className={`d-menu-item ${danger ? 'danger' : ''}`} onClick={onClick} disabled={disabled}>
      {children}
      {selected && <Icon name="check" size={14} className="d-check" />}
    </button>
  );
}

/** A menu with a filter box, for picking one of many (or several, with `multi`). */
export function Picker<T extends string | number | null>({ trigger, options, value, onPick, multi, placeholder = 'Filter…', width = 240 }: {
  trigger: (open: boolean, toggle: () => void) => ReactNode;
  options: { value: T; label: string; icon?: ReactNode; hint?: string }[];
  value: T | T[];
  onPick: (v: T) => void;
  multi?: boolean;
  placeholder?: string;
  width?: number;
}) {
  return (
    <Menu width={width} trigger={trigger}>
      {(close) => <PickerBody options={options} value={value} multi={multi} placeholder={placeholder} onPick={(v) => { onPick(v); if (!multi) close(); }} />}
    </Menu>
  );
}

function PickerBody<T extends string | number | null>({ options, value, onPick, multi, placeholder }: {
  options: { value: T; label: string; icon?: ReactNode; hint?: string }[];
  value: T | T[];
  onPick: (v: T) => void;
  multi?: boolean;
  placeholder: string;
}) {
  const [q, setQ] = useState('');
  const [at, setAt] = useState(0);
  const shown = options.filter((o) => o.label.toLowerCase().includes(q.trim().toLowerCase()));
  const isOn = (v: T) => (Array.isArray(value) ? value.includes(v) : value === v);
  return (
    <>
      {options.length > 6 && (
        <input
          className="d-menu-filter"
          autoFocus
          placeholder={placeholder}
          value={q}
          onChange={(e) => { setQ(e.currentTarget.value); setAt(0); }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setAt((a) => Math.min(shown.length - 1, a + 1)); }
            if (e.key === 'ArrowUp') { e.preventDefault(); setAt((a) => Math.max(0, a - 1)); }
            if (e.key === 'Enter' && shown[at]) { e.preventDefault(); onPick(shown[at].value); }
          }}
        />
      )}
      <div className="d-menu-scroll">
        {shown.map((o, n) => (
          <button type="button" key={String(o.value)} className={`d-menu-item ${n === at && q ? 'hot' : ''}`} onClick={() => onPick(o.value)}>
            {multi && <span className={`d-box ${isOn(o.value) ? 'on' : ''}`}>{isOn(o.value) && <Icon name="check" size={11} />}</span>}
            {o.icon}
            <span className="d-grow d-ellipsis">{o.label}</span>
            {o.hint && <span className="d-tiny d-dim">{o.hint}</span>}
            {!multi && isOn(o.value) && <Icon name="check" size={14} className="d-check" />}
          </button>
        ))}
        {!shown.length && <div className="d-menu-empty">Nothing matches</div>}
      </div>
    </>
  );
}

export function priorityOptions() {
  return [0, 1, 2, 3, 4].map((p) => ({ value: p, label: PRIORITIES[p], icon: <PriorityIcon p={p} /> }));
}

export function stateOptions(states: State[]) {
  return states.map((s) => ({ value: s.id as string, label: s.name, icon: <StateIcon state={s} /> }));
}

export function peopleOptions(people: string[], withScout = true) {
  return [
    { value: null as string | null, label: 'Unassigned', icon: <Avatar name={null} /> },
    ...people.map((u) => ({ value: u as string | null, label: u, icon: <Avatar name={u} /> })),
    ...(withScout ? [{ value: SCOUT as string | null, label: 'Scout', icon: <Avatar name={SCOUT} />, hint: 'agent' }] : []),
  ];
}

export function labelOptions(doc: Doc) {
  return Object.values(doc.label).sort((a, b) => a.name.localeCompare(b.name)).map((l) => ({ value: l.id as string, label: l.name, icon: <LabelDot color={l.color} /> }));
}

export function Modal({ children, onClose, className = '' }: { children: ReactNode; onClose: () => void; className?: string }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onClose]);
  return (
    <div className="d-modal-back" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className={`d-modal ${className}`} role="dialog">{children}</div>
    </div>
  );
}

/** "3d ago", "just now". */
export function ago(ts: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 45) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.round(s / 86400)}d ago`;
  return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

/** "Nov 4", with the year when it isn't this one. */
export function shortDay(day: string | null): string {
  if (!day) return '';
  const d = new Date(`${day}T12:00:00`);
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', ...(d.getFullYear() !== new Date().getFullYear() ? { year: 'numeric' } : {}) });
}

export const COLORS = ['#5e6ad2', '#26b5ce', '#4cb782', '#f2c94c', '#f2994a', '#eb5757', '#bb87fc', '#8b8d98', '#e5484d', '#12a594'];

export function ColorPicker({ value, onPick }: { value: string; onPick: (c: string) => void }) {
  return (
    <Menu width={176} trigger={(_, toggle) => <button type="button" className="d-swatch" style={{ background: value }} onClick={toggle} aria-label="Color" />}>
      {(close) => (
        <div className="d-swatches">
          {COLORS.map((c) => <button type="button" key={c} className={`d-swatch ${c === value ? 'on' : ''}`} style={{ background: c }} onClick={() => { onPick(c); close(); }} aria-label={c} />)}
        </div>
      )}
    </Menu>
  );
}

/** Text that saves when it loses focus or on Enter, and goes back on Escape. */
export function InlineText({ value, onSave, className = '', placeholder, multiline }: { value: string; onSave: (v: string) => void; className?: string; placeholder?: string; multiline?: boolean }) {
  const [draft, setDraft] = useState(value);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => setDraft(value), [value]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el) {
      el.style.height = '0px';
      el.style.height = `${el.scrollHeight}px`;
    }
  }, [draft]);
  const commit = () => {
    if (draft !== value && (multiline || draft.trim())) onSave(multiline ? draft : draft.trim());
    else if (!multiline && !draft.trim()) setDraft(value);
  };
  return (
    <textarea
      ref={ref}
      rows={1}
      className={`d-inline-text ${className}`}
      value={draft}
      placeholder={placeholder}
      onChange={(e) => setDraft(e.currentTarget.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          setDraft(value);
          e.currentTarget.blur();
        }
        if (e.key === 'Enter' && (!multiline || e.ctrlKey || e.metaKey)) {
          e.preventDefault();
          e.currentTarget.blur();
        }
      }}
    />
  );
}

/** Plain text with links made clickable and `code` marked, kept as written otherwise. */
export function RichText({ text, className = '' }: { text: string; className?: string }) {
  const parts = text.split(/(https?:\/\/[^\s)]+|`[^`\n]+`)/g);
  return (
    <div className={`d-rich ${className}`}>
      {parts.map((p, n) =>
        /^https?:\/\//.test(p) ? <a key={n} href={p} target="_blank" rel="noreferrer">{p}</a> : /^`[^`]+`$/.test(p) ? <code key={n}>{p.slice(1, -1)}</code> : <span key={n}>{p}</span>,
      )}
    </div>
  );
}
