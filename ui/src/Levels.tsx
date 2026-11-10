import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { Icon, Popover } from './widgets';

/** One stop on a level slider, from least to most. */
export type Level = { id: string; label: string; about?: string };

/**
 * A chip that opens a notched slider: left for less, right for more. The last stop is the maximum and takes the
 * slider's `tone` color, on the chip too, so it shows at a glance.
 */
export function LevelPicker({ title, tip, levels, value, onChange, less, more, tone, align = 'start' }: {
  title: string;
  tip: string;
  levels: Level[];
  value: string;
  onChange: (id: string) => void;
  /** Words under the two ends of the slider. */
  less: string;
  more: string;
  /** The color at the maximum: `reason` (accent) or `free` (red, nothing is asked). */
  tone: 'reason' | 'free';
  align?: 'start' | 'end';
}) {
  const last = Math.max(0, levels.length - 1);
  // Shown at once while dragging, until the core reports the same.
  const [picked, setPicked] = useState<string | null>(null);
  useEffect(() => setPicked((p) => (p === value ? null : p)), [value]);
  const pick = (id: string) => {
    if (id === (picked ?? value)) return;
    setPicked(id);
    onChange(id);
  };
  const shown = picked ?? value;
  const at = Math.max(0, levels.findIndex((l) => l.id === shown));
  const max = last > 0 && at === last;
  const cur = levels[at];
  const look = `${tone} ${max ? 'max' : ''}`;
  return (
    <Popover align={align} side="top" width={268} trigger={(open, toggle) => (
      <button type="button" className={`chip level-chip ${look} ${open ? 'open' : ''}`} title={tip} onClick={toggle}>
        <LevelBars count={last} filled={at} />
        <span className="ellipsis">{cur?.label ?? shown}</span>
        <Icon name="chevronDown" size={12} />
      </button>
    )}>
      {(close) => (
        <div className={`level-panel ${look}`}>
          <div className="level-head">
            <span className="level-title">{title}</span>
            <b key={cur?.id} className="level-name">{cur?.label ?? shown}</b>
          </div>
          {levels.some((l) => l.about) && <div key={cur?.id} className="level-about tiny dim ellipsis">{cur?.about}</div>}
          <div className="level-track" style={{ '--f': last ? at / last : 0 } as CSSProperties}>
            <span className="level-rail" />
            <span className={`level-fill ${at > 0 ? 'on' : ''}`} />
            {levels.map((l, i) => (
              <span
                key={l.id}
                className={`level-tick ${i < at ? 'on' : ''} ${i === at ? 'here' : ''}`}
                style={{ '--t': last ? i / last : 0 } as CSSProperties}
              />
            ))}
            <span className="level-thumb" />
            <LevelRange title={title} last={last} at={at} label={cur?.label} onPick={(i) => { const l = levels[i]; if (l) pick(l.id); }} onDone={close} />
          </div>
          <div className="level-ends tiny dim"><span>{less}</span><span>{more}</span></div>
        </div>
      )}
    </Popover>
  );
}

/** The real slider: invisible, over the drawn one, so dragging, clicking and the arrow keys all work natively. */
function LevelRange({ title, last, at, label, onPick, onDone }: { title: string; last: number; at: number; label?: string; onPick: (i: number) => void; onDone: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  // Focused once the popover has a place on screen, so the arrow keys work right away.
  useEffect(() => {
    const id = requestAnimationFrame(() => input.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(id);
  }, []);
  return (
    <input
      ref={input}
      type="range"
      className="level-range"
      min={0}
      max={last}
      step={1}
      value={at}
      aria-label={title}
      aria-valuetext={label}
      onChange={(e) => onPick(Number(e.target.value))}
      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onDone(); } }}
    />
  );
}

/** Little bars of rising height, `filled` of `count` lit: the level at a glance on the chip. */
function LevelBars({ count, filled }: { count: number; filled: number }) {
  return (
    <span className="level-bars" aria-hidden>
      {Array.from({ length: count }, (_, i) => <span key={i} className={i < filled ? 'on' : ''} style={{ height: 5 + (i * 7) / Math.max(1, count - 1) }} />)}
    </span>
  );
}
