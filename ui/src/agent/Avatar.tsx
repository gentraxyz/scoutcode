// An agent's avatar: a soft body in the user's shape and colour with a pair of big, close-set eyes. The user only picks
// the shape and the colour; the face is alive and changes on its own every few seconds, drawing from a set of faces that
// suits what the agent is doing (focused while working, curious while thinking, eager while waiting). The body moves
// too: it glances gently while working, breathes while thinking, blinks now and then while idle, closes its eyes while
// asleep, goes x x when stuck, plus a small dot in the corner when it needs you (waiting) or is stuck (blocked / error).
import { useEffect, useState, type CSSProperties, type ReactNode } from 'react';
import type { Shape, Status } from './store';

type Eye = 'dot' | 'oval' | 'happy' | 'wide' | 'small' | 'closed' | 'x';
type Face = readonly [Eye, Eye];

const FACES = {
  dot: ['dot', 'dot'],
  oval: ['oval', 'oval'],
  happy: ['happy', 'happy'],
  wide: ['wide', 'wide'],
  small: ['small', 'small'],
  wink: ['dot', 'happy'],
} as const satisfies Record<string, Face>;
type FaceId = keyof typeof FACES;

/** The faces an agent cycles through, by what it's doing. */
const MOODS: Partial<Record<Status, FaceId[]>> = {
  idle: ['dot', 'happy', 'oval', 'wink', 'small', 'wide'],
  thinking: ['oval', 'small', 'dot'],
  working: ['wide', 'dot', 'oval'],
  waiting: ['wide', 'happy', 'wink'],
};

/** Where the eyes sit inside each body, in the 100×100 box, and how big they are. */
const BODIES: Record<Shape, { body: ReactNode; y: number; s: number }> = {
  circle: { body: <circle cx="50" cy="50" r="50" />, y: 47, s: 1 },
  blob: { body: <path d="M52 3C77 2 97 21 96 47C95 71 84 97 53 97C25 98 3 82 4 52C5 25 27 4 52 3Z" />, y: 47, s: 1 },
  square: { body: <rect x="4" y="4" width="92" height="92" rx="24" />, y: 46, s: 1 },
  triangle: { body: <path d="M50 10L93 86H7Z" strokeWidth="14" strokeLinejoin="round" />, y: 63, s: 0.86 },
  hexagon: { body: <path d="M50 6L88 28V72L50 94L12 72V28Z" strokeWidth="10" strokeLinejoin="round" />, y: 48, s: 0.95 },
  star: { body: <path d="M50 7L62.9 35.2L93.7 38.8L70.9 59.8L77 90.2L50 75L23 90.2L29.1 59.8L6.3 38.8L37.1 35.2Z" strokeWidth="9" strokeLinejoin="round" />, y: 53, s: 0.7 },
  diamond: { body: <path d="M50 5L95 50L50 95L5 50Z" strokeWidth="9" strokeLinejoin="round" />, y: 48, s: 0.88 },
  ghost: { body: <path d="M8 50A42 42 0 0 1 92 50V90Q85 98 78 90T64 90T50 90T36 90T22 90T8 90Z" />, y: 45, s: 0.95 },
};

/** Dark or light eyes, whichever reads on the colour. */
function ink(hex: string): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return '#fff';
  const n = parseInt(m[1], 16);
  const lum = (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.62 ? 'rgba(0,0,0,0.78)' : '#fff';
}

/** One eye centred on x, in the avatar's 100×100 box. */
function eye(kind: Eye, x: number): ReactNode {
  const y = 47;
  const line = { fill: 'none', stroke: 'var(--agent-ink)', strokeWidth: 6, strokeLinecap: 'round' as const };
  switch (kind) {
    case 'oval': return <ellipse cx={x} cy={y} rx={6.5} ry={10} />;
    case 'wide': return <circle cx={x} cy={y} r={10} />;
    case 'small': return <circle cx={x} cy={y} r={6.5} />;
    case 'happy': return <path d={`M${x - 8} ${y + 3}Q${x} ${y - 9} ${x + 8} ${y + 3}`} {...line} />;
    case 'closed': return <path d={`M${x - 8} ${y}Q${x} ${y + 7} ${x + 8} ${y}`} {...line} />;
    case 'x': return <path d={`M${x - 6} ${y - 6}L${x + 6} ${y + 6}M${x + 6} ${y - 6}L${x - 6} ${y + 6}`} {...line} />;
    default: return <circle cx={x} cy={y} r={8.5} />;
  }
}

/** The face right now: changes by itself every few seconds, from the faces that fit the status. */
function useFace(status: Status, seed: number): Face {
  const mood = MOODS[status];
  const [i, setI] = useState(seed);
  useEffect(() => {
    if (!mood) return;
    let t: ReturnType<typeof setTimeout>;
    const next = () => {
      // Never the same face twice in a row; a bit quicker while busy.
      const base = status === 'idle' ? 4000 : 2600;
      t = setTimeout(() => { setI((n) => n + 1 + Math.floor(Math.random() * (mood.length - 1))); next(); }, base + Math.random() * 3500);
    };
    next();
    return () => clearTimeout(t);
  }, [mood, status]);
  if (status === 'sleeping') return ['closed', 'closed'];
  if (status === 'blocked' || status === 'error') return ['x', 'x'];
  return FACES[(mood ?? MOODS.idle!)[i % (mood ?? MOODS.idle!).length]];
}

export function Avatar({ shape, color, status = 'idle', size = 40, title }: { shape: Shape; color: string; status?: Status; kind?: string | null; size?: number; title?: string }) {
  // A steady per-agent offset into its animations and faces, so several on screen never move or change in step.
  let h = 0;
  for (const c of shape + color) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const delay = `-${(h % 9000) / 1000}s`;
  const [left, right] = useFace(status, h % 97);
  const flag = status === 'waiting' ? 'wait' : status === 'blocked' || status === 'error' ? 'warn' : null;
  const b = BODIES[shape] ?? BODIES.circle;
  return (
    <span
      className={`agent-avatar st-${status}`}
      style={{ '--agent': color, '--agent-ink': ink(color), '--d': delay, width: size, height: size, fontSize: size * 0.34 } as CSSProperties}
      title={title}
      aria-hidden={!title}
    >
      <svg className="agent-body" viewBox="0 0 100 100">
        <g fill="var(--agent)" stroke="var(--agent)" strokeWidth="0">{b.body}</g>
        <g transform={`translate(50 ${b.y}) scale(${b.s}) translate(-50 -47)`}>
          <g className="agent-eyes" fill="var(--agent-ink)">
            <g key={left + right} className="agent-face">
              <g className="agent-eye">{eye(left, 37)}</g>
              <g className="agent-eye">{eye(right, 63)}</g>
            </g>
          </g>
        </g>
      </svg>
      {flag && <span className={`agent-flag ${flag}`} />}
      {status === 'sleeping' && <span className="agent-z">z</span>}
    </span>
  );
}

export const AGENT_SHAPES: Shape[] = ['circle', 'blob', 'square', 'triangle', 'hexagon', 'star', 'diamond', 'ghost'];
export const AGENT_COLORS = ['#7c5cff', '#4f8cff', '#22b8cf', '#2fbf71', '#a3d936', '#f5c518', '#ff9f43', '#ff6b6b', '#f368e0', '#c56cf0', '#8395a7', '#f1f2f6'];
