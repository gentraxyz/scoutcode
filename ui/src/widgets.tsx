import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
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
  folderPlus: 'M2 4.5h4l1.5 1.5H14v6.5H2zM8 7.600v3.300M6.350 9.250h3.300',
  home: 'M2.500 7.500L8 3l5.500 4.500M4 6.500V13h3V9.500h2V13h3V6.500',
  desktop: 'M2.500 3h11v7.500h-11zM6 13.500h4M8 10.500v3',
  drive: 'M2.500 9.500l1.500-6h8l1.500 6v3.500h-11zM2.500 9.500h11M10.500 11.250h.01',
  forward: 'M6.500 3.500L11 8l-4.500 4.500',
  eye: 'M1.800 8S4 3.800 8 3.800 14.200 8 14.200 8 12 12.200 8 12.200 1.800 8 1.800 8zM8 9.800a1.800 1.800 0 1 0 0-3.600 1.800 1.800 0 0 0 0 3.600z',
  eyeOff: 'M1.800 8S4 3.800 8 3.800 14.200 8 14.200 8 12 12.200 8 12.200 1.800 8 1.800 8zM8 9.800a1.800 1.800 0 1 0 0-3.600 1.800 1.800 0 0 0 0 3.600zM2.500 2.500l11 11',
  file: 'M4 2h5l3 3v9H4zM9 2v3h3',
  memory: 'M4.500 2.500h7v11L8 11l-3.500 2.500z',
  terminal: 'M2.5 3.5h11v9h-11zM5 6.5L7 8l-2 1.5M8.5 10H11',
  changes: 'M4 5.400a1.700 1.700 0 1 0 0-3.400 1.700 1.700 0 0 0 0 3.400zM12 14a1.700 1.700 0 1 0 0-3.400 1.700 1.700 0 0 0 0 3.400zM12 10.600V6a2 2 0 0 0-2-2H7.500M9.300 2.200L7.500 4l1.800 1.800M4 5.400V10a2 2 0 0 0 2 2h2.500M6.700 10.200L8.500 12l-1.800 1.800',
  route: 'M2.5 4.5h4l3 7h4M11.5 9.5l2 2-2 2M2.5 11.5h4l1-2.3M11.5 2.5l2 2-2 2M9.5 4.5h4',
  files: 'M3 2.5h6l3 3v8H3zM9 2.5v3h3M5.5 9h4M5.5 11h4',
  gear: 'M8 10.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4zM8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6L5 5M11 11l1.4 1.4M12.4 3.6L11 5M5 11l-1.4 1.4',
  chart: 'M3 13V8M8 13V3M13 13V6',
  palette: 'M8 2a6 6 0 0 0 0 12c1 0 1.500-.700 1.200-1.500-.400-1 .300-2 1.300-2H12a2 2 0 0 0 2-2C14 4.700 11.300 2 8 2zM5 7.500h.01M6.500 5h.01M9.500 5h.01M11 7.500h.01',
  download: 'M8 2.5v7.5M4.8 7L8 10.2 11.2 7M3 13.5h10',
  user: 'M8 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM2.5 14c.6-3 2.8-4.5 5.5-4.5s4.900 1.500 5.500 4.500',
  bell: 'M4 7a4 4 0 1 1 8 0c0 3.500 1.500 4.500 1.500 4.500h-11S4 10.500 4 7zM6.800 14h2.400',
  chat: 'M2.500 3.500h11v7.500H7l-3 2.500V11H2.500z',
  pencil: 'M3 13l.700-2.800 7-7 2.100 2.100-7 7zM9.700 4.200l2.100 2.100',
  paperclip: 'M12.500 7.500l-4.800 4.800a2.800 2.800 0 0 1-4-4l5.200-5.200a1.900 1.900 0 0 1 2.700 2.700l-5 5a.9.9 0 0 1-1.300-1.300l4.500-4.500',
  mic: 'M8 10a2 2 0 0 0 2-2V4a2 2 0 1 0-4 0v4a2 2 0 0 0 2 2zM3.800 7.500a4.200 4.200 0 0 0 8.400 0M8 11.700V14',
  copy: 'M6.5 5.5h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-5a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1zM3.5 10.5v-6a1 1 0 0 1 1-1h6',
  arrowUp: 'M8 13V3.500M3.800 7.500L8 3.300l4.200 4.200',
  refresh: 'M13 8a5 5 0 1 1-1.500-3.500M13 2.500v3h-3',
  back: 'M9.500 3.500L5 8l4.500 4.500',
  minimize: 'M3 8h10',
  maximize: 'M3.500 3.500h9v9h-9z',
  restore: 'M5.500 5.500h7v7h-7zM3.500 10.500v-7h7',
  game: 'M5 5h6a3 3 0 0 1 3 3v1a2.300 2.300 0 0 1-4.100 1.400L9.200 9.800H6.800l-.700.600A2.300 2.300 0 0 1 2 9V8a3 3 0 0 1 3-3zM5 6.700v2.200M3.900 7.800h2.200M10.300 7.400h.01M11.700 8.600h.01',
  music: 'M6 12.500V3.800l7-1.300v8.700M6 6.200l7-1.300M4.500 14a1.800 1.800 0 1 0 0-3.600 1.800 1.800 0 0 0 0 3.600zM11.500 12.700a1.800 1.800 0 1 0 0-3.600 1.800 1.800 0 0 0 0 3.600z',
  play: 'M5 3.500l8 4.500-8 4.500z',
  pause: 'M5.500 3.500v9M10.500 3.500v9',
  skipBack: 'M3.500 4v8M12.500 4L6.500 8l6 4z',
  skipForward: 'M12.500 4v8M3.500 4l6 4-6 4z',
  volume: 'M3 6.500h2.500L9 3.500v9l-3.500-3H3zM11.200 5.800a3.200 3.200 0 0 1 0 4.400M12.800 4a5.500 5.500 0 0 1 0 8',
  volumeOff: 'M3 6.500h2.500L9 3.500v9l-3.500-3H3zM11.500 6.500l3 3M14.500 6.500l-3 3',
  repeat: 'M3.500 6.500V5a1.500 1.500 0 0 1 1.500-1.500h7.500M10.500 1.800L12.500 3.500l-2 1.700M12.500 9.500V11a1.500 1.500 0 0 1-1.500 1.500H3.500M5.500 14.200L3.500 12.500l2-1.700',
  upload: 'M8 10.500V3M4.800 6.200L8 3l3.200 3.200M3 13.500h10',
  sparkle: 'M7 2.500l1.300 3.200 3.200 1.300-3.200 1.300L7 11.500 5.700 8.300 2.500 7l3.200-1.300zM12 10v4M10 12h4',
  layers: 'M8 2.500l5.500 3L8 8.500l-5.500-3zM2.500 8L8 11l5.500-3M2.500 10.500L8 13.500l5.500-3',
  shield: 'M8 1.800l5 1.900V8c0 3-2.200 5-5 6.200C5.200 13 3 11 3 8V3.700zM5.800 8l1.600 1.600 2.900-3',
  board: 'M2.500 2.500h3v8h-3zM6.500 2.500h3v5h-3zM10.500 2.500h3v11h-3z',
  phone: 'M5 1.500h6v13H5zM7.300 12.300h1.400',
  globe: 'M8 14A6 6 0 1 0 8 2a6 6 0 0 0 0 12zM2 8h12M8 2c1.700 1.700 2.500 3.700 2.500 6S9.700 12.300 8 14M8 2C6.300 3.700 5.500 5.700 5.500 8S6.300 12.300 8 14',
  key: 'M5 13.500a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM7.200 8.300L13 2.500M11 4.500l1.500 1.500M9.500 6l1.200 1.200',
  browser: 'M2 3h12v10H2zM2 5.800h12M4 4.400h.01M5.600 4.400h.01',
  arrowRight: 'M3 8h10M9 4l4 4-4 4',
  star: 'M8 1.800l1.900 3.900 4.300.6-3.100 3 .7 4.300L8 11.600l-3.800 2 .7-4.300-3.100-3 4.300-.6z',
  sliders: 'M2.500 4.500h11M2.500 8h11M2.500 11.500h11M5.500 3v3M10.500 6.500v3M6.800 10v3',
  bolt: 'M8.800 1.800L3.500 9h4.200l-.500 5.200L12.500 7H8.300z',
  credit: 'M2 4h12v8H2zM2 6.800h12M4.500 9.600h3',
  gift: 'M2.500 6.500h11v2h-11zM3.500 8.500h9v5h-9zM8 6.500v7M8 6.500H5.800a1.500 1.500 0 1 1 1.400-2L8 6.500zM8 6.500h2.200a1.500 1.500 0 1 0-1.400-2L8 6.500z',
  lock: 'M4 7.500h8v6H4zM5.500 7.500V5.300a2.500 2.500 0 0 1 5 0v2.200M8 10v1.500',
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

export function IconButton({ icon, tip, onClick, size = 30, active, disabled, className = '' }: { icon: string; tip: string; onClick?: (e: React.MouseEvent) => void; size?: number; active?: boolean; disabled?: boolean; className?: string }) {
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
export const Switch = Toggle;

export function SectionLabel({ children }: { children: ReactNode }) {
  return <div className="section-label">{children}</div>;
}

// ---------- menus ----------

type OpenPopover = { anchor: () => HTMLSpanElement | null; panel: () => HTMLDivElement | null };
const openPopovers: OpenPopover[] = [];

/** A chip or button that opens a floating panel. Esc or a click outside closes it. */
export function Popover({ trigger, children, align = 'start', side = 'auto', className = '', onOpen, width }: {
  trigger: (open: boolean, toggle: () => void) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: 'start' | 'end';
  side?: 'auto' | 'top';
  className?: string;
  onOpen?: () => void;
  /** Pixels, or 'anchor' to be as wide as the trigger. */
  width?: number | 'anchor';
}) {
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<CSSProperties>({ visibility: 'hidden' });
  const [above, setAbove] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const close = () => {
    setOpen(false);
    setStyle({ visibility: 'hidden' });
  };
  const toggle = () => {
    if (open) {
      close();
    } else {
      onOpen?.();
      setOpen(true);
    }
  };
  useLayoutEffect(() => {
    if (!open || !anchor.current || !panel.current) return;
    const a = anchor.current.getBoundingClientRect();
    const p = panel.current.getBoundingClientRect();
    const roomAbove = a.top - 14;
    const roomBelow = window.innerHeight - a.bottom - 14;
    // Opens on the asked side when it fits there, else on the side with more room.
    const above = side === 'top' ? p.height <= roomAbove || roomAbove >= roomBelow : p.height > roomBelow && roomAbove > roomBelow;
    const room = Math.max(0, Math.floor(above ? roomAbove : roomBelow));
    const left = align === 'end' ? a.right - p.width : a.left;
    // Above the anchor it's held by its bottom edge, so it grows upward when what's in it gets longer.
    setAbove(above);
    // --room is how much height there is on that side; the model menu shrinks to it.
    const next: CSSProperties & { '--room': string } = {
      '--room': `${room}px`,
      transformOrigin: `${above ? 'bottom' : 'top'} ${align === 'end' ? 'right' : 'left'}`,
      position: 'fixed',
      left: Math.max(8, Math.min(left, window.innerWidth - p.width - 8)),
      ...(above ? { bottom: window.innerHeight - a.top + 6, maxHeight: room } : { top: a.bottom + 6, maxHeight: room }),
      width: width === 'anchor' ? a.width : width,
    };
    setStyle(next);
  }, [open, align, side, width]);
  useEffect(() => {
    if (!open) return;
    const entry: OpenPopover = { anchor: () => anchor.current, panel: () => panel.current };
    openPopovers.push(entry);
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      const at = openPopovers.indexOf(entry);
      const inside = openPopovers.slice(at >= 0 ? at : 0).some((o) => o.panel()?.contains(t) || o.anchor()?.contains(t));
      if (!inside) close();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && openPopovers[openPopovers.length - 1] === entry) {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        close();
      }
    };
    document.addEventListener('mousedown', down);
    window.addEventListener('keydown', key, true);
    return () => {
      const at = openPopovers.indexOf(entry);
      if (at >= 0) openPopovers.splice(at, 1);
      document.removeEventListener('mousedown', down);
      window.removeEventListener('keydown', key, true);
    };
  }, [open]);
  return (
    <>
      <span ref={anchor} className="popover-anchor">{trigger(open, toggle)}</span>
      {open && createPortal(
        <div ref={panel} className={`popover ${above ? 'above' : ''} ${className}`} style={{ width: width === 'anchor' ? anchor.current?.offsetWidth : width, ...style }}>{children(close)}</div>,
        document.body,
      )}
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

function userFacingText(text: string): string {
  if (text.startsWith('The user denied this')) {
    const fb = text.includes(' They said: ') ? text.split(' They said: ')[1] : '';
    const noun = text.includes('bash call') || text.includes('command') ? 'command' : text.includes('edit') || text.includes('write_file') ? 'edit' : 'call';
    return fb ? `You denied this ${noun}: ${fb}` : `You denied this ${noun}.`;
  }
  return text;
}

export function ShowDisplay({ d }: { d: Display }) {
  switch (d.type) {
    case 'text':
      return <pre className="tool-text">{userFacingText(d.text)}</pre>;
    case 'todos':
      return <TodoList todos={d.todos} />;
    case 'image':
      return <figure className="tool-image"><img src={d.preview} alt={d.path} /><figcaption className="mono small dim">{d.path}</figcaption></figure>;
    case 'diffs':
      return <div className="stack">{d.diffs.map((x, i) => <ShowDisplay key={i} d={x} />)}</div>;
    case 'agent':
      return <pre className="tool-text">{d.answer}</pre>;
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

export function CopyIconButton({ text, tip = 'Copy', size = 26, className = '' }: { text: string; tip?: string; size?: number; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <IconButton
      icon={copied ? 'check' : 'copy'}
      tip={copied ? 'Copied' : tip}
      size={size}
      className={`${className} ${copied ? 'copied' : ''}`}
      onClick={(e) => {
        e.stopPropagation();
        navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    />
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
