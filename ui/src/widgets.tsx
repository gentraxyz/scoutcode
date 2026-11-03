import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { openExternal, type DiffLines, type Display, type Todo } from './api';

// ---------- icons: 16 by 16, stroked in the current color ----------

const PATHS: Record<string, string> = {
  plus: 'M8 3v10M3 8h10',
  close: 'M4 4l8 8M12 4l-8 8',
  check: 'M3.5 8.5l3 3 6-7',
  search: 'M7 12A5 5 0 1 0 7 2a5 5 0 0 0 0 10zM11 11l3 3',
  chevronRight: 'M6 3.5L10.5 8 6 12.5',
  chevronDown: 'M3.5 6L8 10.5 12.5 6',
  chevronUp: 'M3.5 10L8 5.5 12.5 10',
  more: 'M3.5 8h.01M8 8h.01M12.5 8h.01',
  sidebar: 'M2.5 3.5h11v9h-11zM6 3.5v9',
  panel: 'M2.5 3.5h11v9h-11zM10 3.5v9',
  folder: 'M2 4.5h4l1.5 1.5H14v6.5H2z',
  file: 'M4 2h5l3 3v9H4zM9 2v3h3',
  memory: 'M4.500 2.500h7v11L8 11l-3.500 2.500z',
  terminal: 'M2.5 3.5h11v9h-11zM5 6.5L7 8l-2 1.5M8.5 10H11',
  changes: 'M4 5.400a1.700 1.700 0 1 0 0-3.400 1.700 1.700 0 0 0 0 3.400zM12 14a1.700 1.700 0 1 0 0-3.400 1.700 1.700 0 0 0 0 3.400zM12 10.600V6a2 2 0 0 0-2-2H7.500M9.300 2.200L7.500 4l1.800 1.800M4 5.400V10a2 2 0 0 0 2 2h2.500M6.700 10.200L8.500 12l-1.800 1.800',
  files: 'M3 2.5h6l3 3v8H3zM9 2.5v3h3M5.5 9h4M5.5 11h4',
  gear: 'M8 10.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4zM8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6L5 5M11 11l1.4 1.4M12.4 3.6L11 5M5 11l-1.4 1.4',
  chart: 'M3 13V8M8 13V3M13 13V6',
  download: 'M8 2.5v7.5M4.8 7L8 10.2 11.2 7M3 13.5h10',
  user: 'M8 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM2.5 14c.6-3 2.8-4.5 5.5-4.5s4.900 1.500 5.500 4.500',
  bell: 'M4 7a4 4 0 1 1 8 0c0 3.500 1.500 4.500 1.500 4.500h-11S4 10.500 4 7zM6.800 14h2.400',
  chat: 'M2.500 3.500h11v7.500H7l-3 2.500V11H2.500z',
  pencil: 'M3 13l.700-2.800 7-7 2.100 2.100-7 7zM9.700 4.200l2.100 2.100',
  paperclip: 'M12.500 7.500l-4.800 4.800a2.800 2.800 0 0 1-4-4l5.200-5.200a1.900 1.900 0 0 1 2.700 2.700l-5 5a.9.9 0 0 1-1.300-1.300l4.500-4.500',
  mic: 'M8 10a2 2 0 0 0 2-2V4a2 2 0 1 0-4 0v4a2 2 0 0 0 2 2zM3.800 7.500a4.200 4.200 0 0 0 8.400 0M8 11.700V14',
  arrowUp: 'M8 13V3.500M3.800 7.500L8 3.300l4.200 4.200',
  refresh: 'M13 8a5 5 0 1 1-1.500-3.500M13 2.500v3h-3',
  back: 'M9.500 3.500L5 8l4.500 4.500',
  minimize: 'M3 8h10',
  maximize: 'M3.500 3.500h9v9h-9z',
  restore: 'M5.500 5.500h7v7h-7zM3.500 10.500v-7h7',
};

export function Icon({ name, size = 16 }: { name: keyof typeof PATHS | string; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.400" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={PATHS[name]} />
    </svg>
  );
}

export function Star({ on }: { on: boolean }) {
  return (
    <svg className="icon" width={14} height={14} viewBox="0 0 16 16" fill={on ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="1.200" strokeLinejoin="round" aria-hidden>
      <path d="M8 1.800l1.900 3.900 4.300.6-3.100 3 .7 4.300L8 11.600l-3.800 2 .7-4.300-3.100-3 4.300-.6z" />
    </svg>
  );
}

export function Spinner({ size = 12 }: { size?: number }) {
  return <span className="spinner" style={{ width: size, height: size }} aria-label="Working" />;
}

export function IconButton({ icon, tip, onClick, size = 28, active, disabled, className = '' }: { icon: string; tip: string; onClick?: (e: React.MouseEvent) => void; size?: number; active?: boolean; disabled?: boolean; className?: string }) {
  return (
    <button type="button" className={`icon-button ${active ? 'active' : ''} ${className}`} style={{ width: size, height: size }} title={tip} aria-label={tip} disabled={disabled} onClick={onClick}>
      <Icon name={icon} />
    </button>
  );
}

export function Link({ href, children, className = '' }: { href: string; children: ReactNode; className?: string }) {
  return (
    <a className={`link ${className}`} href={href} onClick={(e) => { e.preventDefault(); openExternal(href); }}>
      {children}
    </a>
  );
}

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (on: boolean) => void; label: string }) {
  return <button type="button" role="switch" aria-checked={on} aria-label={label} className={`toggle ${on ? 'on' : ''}`} onClick={() => onChange(!on)}><span /></button>;
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return <div className="section-label">{children}</div>;
}

// ---------- menus ----------

/** A chip or button that opens a floating panel. Esc or a click outside closes it. */
export function Popover({ trigger, children, align = 'start', side = 'auto', className = '', onOpen, width }: {
  trigger: (open: boolean, toggle: () => void) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: 'start' | 'end';
  side?: 'auto' | 'top';
  className?: string;
  onOpen?: () => void;
  width?: number;
}) {
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<CSSProperties>({ visibility: 'hidden' });
  const [above, setAbove] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const close = () => setOpen(false);
  const toggle = () => {
    if (!open) onOpen?.();
    setOpen(!open);
  };
  useLayoutEffect(() => {
    if (!open || !anchor.current || !panel.current) return;
    const a = anchor.current.getBoundingClientRect();
    const p = panel.current.getBoundingClientRect();
    const above = side === 'top' || a.bottom + p.height + 8 > window.innerHeight;
    const left = align === 'end' ? a.right - p.width : a.left;
    // Above the anchor it's held by its bottom edge, so it grows upward when what's in it gets longer.
    setAbove(above);
    setStyle({
      transformOrigin: `${above ? 'bottom' : 'top'} ${align === 'end' ? 'right' : 'left'}`,
      position: 'fixed',
      left: Math.max(8, Math.min(left, window.innerWidth - p.width - 8)),
      ...(above ? { bottom: window.innerHeight - a.top + 6, maxHeight: a.top - 14 } : { top: a.bottom + 6 }),
      width,
    });
  }, [open, align, side, width]);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      if (!panel.current?.contains(e.target as Node) && !anchor.current?.contains(e.target as Node)) close();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener('mousedown', down);
    window.addEventListener('keydown', key, true);
    return () => {
      document.removeEventListener('mousedown', down);
      window.removeEventListener('keydown', key, true);
    };
  }, [open]);
  return (
    <>
      <span ref={anchor} className="popover-anchor">{trigger(open, toggle)}</span>
      {open && <div ref={panel} className={`popover ${above ? 'above' : ''} ${className}`} style={{ ...style, width }}>{children(close)}</div>}
    </>
  );
}

export function MenuItem({ selected, onClick, icon, disabled, children }: { selected?: boolean; onClick?: () => void; icon?: string; disabled?: boolean; children: ReactNode }) {
  return (
    <button type="button" className={`menu-item ${selected ? 'selected' : ''}`} disabled={disabled} onClick={onClick}>
      {icon && <Icon name={icon} />}
      <span className="grow ellipsis">{children}</span>
      {selected && <Icon name="check" size={14} />}
    </button>
  );
}

export function Modal({ onClose, children, className = '' }: { onClose: () => void; children: ReactNode; className?: string }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onClose]);
  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${className}`} role="dialog" aria-modal>{children}</div>
    </div>
  );
}

// ---------- what tools show ----------

export function DiffBody({ lines }: { lines: DiffLines }) {
  return (
    <div className="diff-body">
      {lines.rows.map((l, i) =>
        l === null ? (
          <div key={i} className="diff-gap">⋯</div>
        ) : (
          <div key={i} className={`diff-line ${l.kind}`}>
            <span className="diff-no">{l.no ?? ''}</span>
            <span className="diff-text">{l.kind === 'add' ? '+' : l.kind === 'del' ? '-' : ' '} {l.text}</span>
          </div>
        ),
      )}
      {lines.more > 0 && <div className="diff-gap">… {lines.more} more lines</div>}
    </div>
  );
}

export function TodoList({ todos }: { todos: Todo[] }) {
  return (
    <ul className="todos">
      {todos.map((t, i) => (
        <li key={i} className={t.status}>
          <span className="todo-mark">{t.status === 'completed' ? <Icon name="check" size={13} /> : t.status === 'in_progress' ? <Spinner size={10} /> : <span className="todo-dot" />}</span>
          <span>{t.content}</span>
        </li>
      ))}
    </ul>
  );
}

export function ShowDisplay({ d }: { d: Display }) {
  switch (d.type) {
    case 'text':
      return <pre className="tool-text">{d.text}</pre>;
    case 'todos':
      return <TodoList todos={d.todos} />;
    case 'image':
      return <figure className="tool-image"><img src={d.preview} alt={d.path} /><figcaption className="mono small dim">{d.path}</figcaption></figure>;
    case 'diffs':
      return <div className="stack">{d.diffs.map((x, i) => <ShowDisplay key={i} d={x} />)}</div>;
    case 'diff':
      return (
        <div className="diff">
          <div className="diff-head">
            <span className="mono strong ellipsis">{d.path}</span>
            {d.created && <span className="small dim">new file</span>}
            <span className="grow" />
            <span className="mono green">+{d.add}</span>
            <span className="mono red">-{d.del}</span>
          </div>
          <DiffBody lines={d.lines} />
        </div>
      );
  }
}

export function CopyButton({ text, label = 'Copy', className = '' }: { text: string; label?: string; className?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button type="button" className={`button ${className}`} onClick={() => {
      navigator.clipboard.writeText(text);
      setDone(true);
      setTimeout(() => setDone(false), 1200);
    }}>{done ? 'Copied' : label}</button>
  );
}

export function CodeBlock({ code }: { code: string }) {
  return (
    <div className="code-block">
      <CopyButton text={code} className="code-copy" />
      <pre><code>{code}</code></pre>
    </div>
  );
}
