// Snake: steer the snake round a grid, eat the food to grow, and don't hit a wall or its own body. Each bite speeds it up.
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { crisp, FONT, loadNum, loop, noise, saveNum, SoundToggle, themeColors, tone, type Colors } from './arcade';

const COLS = 20;
const ROWS = 20;
const CELL = 14;
const W = COLS * CELL;
const H = ROWS * CELL;
const START_LEN = 3;
/** Seconds per step: starts slow, speeds up with each bite, never faster than the floor. */
const stepSeconds = (score: number) => Math.max(0.07, 0.14 - score * 0.003);
const SAVE_KEY = 'scout.snake';

type Cell = { c: number; r: number };
type Dir = 'up' | 'down' | 'left' | 'right';
type Phase = 'ready' | 'play' | 'paused' | 'over';
/** `queue` holds turns the player has pressed but the snake hasn't taken yet, so quick double-turns still register. */
type State = {
  phase: Phase;
  body: Cell[];
  dir: Dir;
  queue: Dir[];
  food: Cell | null;
  score: number;
  acc: number;
  pop: number;
  shake: number;
  overAt: number;
};

const DIRS: Record<Dir, Cell> = { up: { c: 0, r: -1 }, down: { c: 0, r: 1 }, left: { c: -1, r: 0 }, right: { c: 1, r: 0 } };
const OPPOSITE: Record<Dir, Dir> = { up: 'down', down: 'up', left: 'right', right: 'left' };
const KEYS: Record<string, Dir> = {
  ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
  w: 'up', s: 'down', a: 'left', d: 'right', W: 'up', S: 'down', A: 'left', D: 'right',
};

const rand = (a: number, b: number) => a + Math.random() * (b - a);
const same = (a: Cell, b: Cell) => a.c === b.c && a.r === b.r;

function placeFood(body: Cell[]): Cell | null {
  const taken = new Set(body.map((b) => b.r * COLS + b.c));
  const free = [...Array(COLS * ROWS).keys()].filter((i) => !taken.has(i));
  if (!free.length) return null;
  const at = free[Math.floor(Math.random() * free.length)];
  return { c: at % COLS, r: Math.floor(at / COLS) };
}

function newState(): State {
  const row = Math.floor(ROWS / 2);
  const col = Math.floor(COLS / 2);
  const body = [...Array(START_LEN)].map((_, i) => ({ c: col - i, r: row }));
  return { phase: 'ready', body, dir: 'right', queue: [], food: placeFood(body), score: 0, acc: 0, pop: 0, shake: 0, overAt: 0 };
}

/** Moves the snake one cell. Returns what happened, so the caller can play the right sound. */
function advance(s: State): 'move' | 'eat' | 'crash' {
  const dir = s.queue.shift() ?? s.dir;
  s.dir = dir;
  const head = s.body[0];
  const next = { c: head.c + DIRS[dir].c, r: head.r + DIRS[dir].r };
  const eats = !!s.food && same(next, s.food);
  // The tail leaves its cell as the head arrives, so the head may follow it round; a bite keeps the tail in place.
  const body = eats ? s.body : s.body.slice(0, -1);
  if (next.c < 0 || next.r < 0 || next.c >= COLS || next.r >= ROWS || body.some((b) => same(b, next))) {
    s.phase = 'over';
    s.shake = 1;
    return 'crash';
  }
  s.body = [next, ...body];
  if (!eats) return 'move';
  s.score += 1;
  s.pop = 1;
  s.food = placeFood(s.body);
  return 'eat';
}

export function Snake({ onEscape, autoFocus }: { onEscape: () => void; autoFocus: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const state = useRef<State>(newState());
  const swipe = useRef<{ x: number; y: number } | null>(null);
  const [score, setScore] = useState(0);
  const [best, setBest] = useState(() => loadNum(SAVE_KEY));
  const [phase, setPhase] = useState<Phase>('ready');
  const [newBest, setNewBest] = useState(false);
  const bestRef = useRef(best);

  const turn = (dir: Dir) => {
    const s = state.current;
    if (s.phase === 'over') return;
    // Before the first step, the snake may start facing any way but back the way it faces.
    if (s.phase === 'ready' && dir !== OPPOSITE[s.dir]) s.dir = dir;
    if (s.phase !== 'play') {
      s.phase = 'play';
      setPhase('play');
    }
    const last = s.queue[s.queue.length - 1] ?? s.dir;
    if (dir === last || dir === OPPOSITE[last] || s.queue.length >= 2) return;
    s.queue.push(dir);
  };

  const restart = () => {
    state.current = newState();
    setScore(0);
    setNewBest(false);
    setPhase('ready');
  };

  /** Space or a tap: start, pause, carry on, or play again once the game-over card has shown a moment. */
  const tap = () => {
    const s = state.current;
    if (s.phase === 'over') {
      if (performance.now() - s.overAt > 400) restart();
      return;
    }
    if (s.phase === 'play') {
      s.phase = 'paused';
      setPhase('paused');
      return;
    }
    turn(s.dir);
  };

  useEffect(() => {
    if (autoFocus) host.current?.focus();
  }, []);

  useEffect(() => {
    const el = canvas.current!;
    const ctx = el.getContext('2d')!;
    let C: Colors = themeColors(el);
    // The theme can change while the game is open.
    const recolor = () => {
      C = themeColors(el);
    };
    const watch = new MutationObserver(recolor);
    watch.observe(document.documentElement, { attributes: true });

    const step = (dt: number) => {
      const s = state.current;
      s.shake = Math.max(0, s.shake - dt * 4);
      s.pop = Math.max(0, s.pop - dt * 4);
      if (s.phase !== 'play') return;
      const interval = stepSeconds(s.score);
      s.acc += dt;
      if (s.acc < interval) return;
      s.acc -= interval;
      const what = advance(s);
      if (what === 'eat') {
        tone(520, 880, 0.07, 'triangle', 0.04);
        setScore(s.score);
      } else if (what === 'crash') {
        noise(0.2, 0.12, 700);
        tone(300, 80, 0.3, 'sawtooth', 0.035);
        s.overAt = performance.now();
        const top = s.score > bestRef.current;
        if (top) {
          bestRef.current = s.score;
          saveNum(SAVE_KEY, s.score);
          setBest(s.score);
        }
        setNewBest(top);
        setPhase('over');
      }
    };

    const shout = (ctx2: CanvasRenderingContext2D, text: string, x: number, y: number, size: number, fill: string) => {
      ctx2.font = `800 ${size}px ${FONT}`;
      ctx2.textAlign = 'center';
      ctx2.textBaseline = 'middle';
      ctx2.lineJoin = 'round';
      ctx2.lineWidth = Math.max(3, size / 7);
      ctx2.strokeStyle = 'rgb(0 0 0 / 55%)';
      ctx2.strokeText(text, x, y);
      ctx2.fillStyle = fill;
      ctx2.fillText(text, x, y);
    };

    const draw = () => {
      const s = state.current;
      ctx.save();
      if (s.shake > 0) ctx.translate(rand(-1, 1) * 3 * s.shake, rand(-1, 1) * 3 * s.shake);
      ctx.fillStyle = C.surface;
      ctx.fillRect(0, 0, W, H);

      // A faint grid, so the cells read as cells.
      ctx.strokeStyle = C.raised;
      ctx.globalAlpha = 0.35;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let c = 1; c < COLS; c++) {
        ctx.moveTo(c * CELL + 0.5, 0);
        ctx.lineTo(c * CELL + 0.5, H);
      }
      for (let r = 1; r < ROWS; r++) {
        ctx.moveTo(0, r * CELL + 0.5);
        ctx.lineTo(W, r * CELL + 0.5);
      }
      ctx.stroke();
      ctx.globalAlpha = 1;

      // The food beats, and swells when eaten.
      if (s.food) {
        const beat = 1 + Math.sin(performance.now() / 160) * 0.08 + s.pop * 0.3;
        ctx.fillStyle = C.red;
        ctx.beginPath();
        ctx.arc(s.food.c * CELL + CELL / 2, s.food.r * CELL + CELL / 2, (CELL / 2 - 2) * beat, 0, Math.PI * 2);
        ctx.fill();
      }

      // The body fades toward the tail; the head is brighter and has eyes.
      const n = s.body.length;
      s.body.forEach((b, i) => {
        ctx.globalAlpha = i === 0 ? 1 : 1 - (i / n) * 0.45;
        ctx.fillStyle = i === 0 ? C.accentHi : C.green;
        ctx.fillRect(b.c * CELL + 1, b.r * CELL + 1, CELL - 2, CELL - 2);
      });
      ctx.globalAlpha = 1;
      const head = s.body[0];
      const d = DIRS[s.dir];
      const hx = head.c * CELL + CELL / 2;
      const hy = head.r * CELL + CELL / 2;
      ctx.fillStyle = C.bg;
      for (const side of [-1, 1]) {
        ctx.beginPath();
        ctx.arc(hx + d.c * 2.5 - d.r * side * 3, hy + d.r * 2.5 + d.c * side * 3, 1.5, 0, Math.PI * 2);
        ctx.fill();
      }

      if (s.phase === 'ready' || s.phase === 'paused') {
        ctx.fillStyle = C.sidebar;
        ctx.globalAlpha = 0.55;
        ctx.fillRect(0, 0, W, H);
        ctx.globalAlpha = 1;
        shout(ctx, s.phase === 'paused' ? 'Paused' : 'Snake', W / 2, H / 2 - 10, 26, C.accentHi);
        ctx.font = `600 12px ${FONT}`;
        ctx.fillStyle = C.dim;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'alphabetic';
        ctx.fillText(s.phase === 'paused' ? 'Space to carry on' : 'Arrow keys or swipe to start', W / 2, H / 2 + 18);
      }
      ctx.restore();
    };

    const stopFit = crisp(el, ctx, W, H);
    const stopLoop = loop(step, draw);
    return () => {
      stopLoop();
      stopFit();
      watch.disconnect();
    };
  }, []);

  const key = (e: KeyboardEvent) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      return onEscape();
    }
    const dir = KEYS[e.key];
    if (dir) {
      e.preventDefault();
      return turn(dir);
    }
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      if (!e.repeat) tap();
    }
  };

  // Leaving the game mid-run pauses it, so the snake never runs on unseen.
  const leave = () => {
    const s = state.current;
    if (s.phase !== 'play') return;
    s.phase = 'paused';
    setPhase('paused');
  };

  const down = (e: PointerEvent) => {
    swipe.current = { x: e.clientX, y: e.clientY };
    host.current?.focus();
  };
  const up = (e: PointerEvent) => {
    const from = swipe.current;
    swipe.current = null;
    if (!from) return;
    const dx = e.clientX - from.x;
    const dy = e.clientY - from.y;
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 16) return tap();
    turn(Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? 'right' : 'left') : dy > 0 ? 'down' : 'up');
  };

  return (
    <div className="game-snake" ref={host} tabIndex={0} onKeyDown={key} onBlur={leave} aria-label="Snake. Steer with the arrow keys, WASD or swipe. Eat the food to grow.">
      <div className="row game-scores">
        <div className="game-score"><span>Score</span><b>{score}</b></div>
        <div className="game-score"><span>Best</span><b key={best} className={newBest ? 'game-best' : ''}>{best}</b></div>
        <span className="grow" />
        <SoundToggle />
      </div>
      <div className="snake-stage" onPointerDown={down} onPointerUp={up} onPointerCancel={() => { swipe.current = null; }}>
        <canvas ref={canvas} />
        {phase === 'over' && (
          <div className="arcade-over">
            <b className="arcade-title">Game over</b>
            <div className="arcade-card">
              <div className="arcade-nums">
                <span>Score</span><b>{score}</b>
                <span>Best {newBest && <i className="arcade-new">New</i>}</span><b>{best}</b>
              </div>
            </div>
            <span className="small dim">Space or a tap to play again</span>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Arrow keys, WASD or swipe to steer. Space pauses.</div>
    </div>
  );
}

export function SnakeIcon() {
  return (
    <svg viewBox="0 0 40 40" width="34" height="34" aria-hidden>
      <path d="M8 32V18Q8 10 16 10H24Q32 10 32 18" fill="none" stroke="#3fae6e" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="32" cy="18" r="4" fill="#7ee2a3" />
      <circle cx="31" cy="16.5" r="1" fill="#1d1d1d" />
      <circle cx="33.2" cy="16.5" r="1" fill="#1d1d1d" />
      <circle cx="31" cy="31" r="3.2" fill="#e86464" />
    </svg>
  );
}
