// Flappy Scout: tap to keep Scout's little bird aloft and fly it through the gaps between stacks of code.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';

const W = 280;
const H = 360;
const GROUND = 22;
const BIRD_X = 78;
const BIRD_R = 9;
const GRAVITY = 1450;
const FLAP = -400;
const STACK_W = 48;
const GAP = 114;
const SPACING = 170;
/** A block of code in a stack, and the gap between blocks. */
const BLOCK_H = 16;
const BLOCK_GAP = 3;
/** The prompt bar that caps each stack at the gap. */
const CAP_H = 15;
const SAVE_KEY = 'scout.flappy';

/** Each block shows one or two lines of "code": a width and a colour, picked once so the stack doesn't flicker. */
type Line = { indent: number; w: number; tone: number; w2: number; tone2: number };
type Stack = { x: number; gapY: number; passed: boolean; lines: Line[] };
type State = { phase: 'ready' | 'play' | 'over'; y: number; vy: number; stacks: Stack[]; score: number; t: number; flapAt: number };

function loadBest() {
  try { return Number(localStorage.getItem(SAVE_KEY)) || 0; } catch { return 0; }
}

function newState(): State {
  return { phase: 'ready', y: H / 2 - 20, vy: 0, stacks: [], score: 0, t: 0, flapAt: -1 };
}

function addStack(s: State, x: number) {
  const margin = 44;
  const gapY = margin + Math.random() * (H - GROUND - GAP - margin * 2);
  const lines = [...Array(24)].map(() => ({
    indent: [0, 0, 5, 10][Math.floor(Math.random() * 4)],
    w: 8 + Math.random() * 16, tone: Math.floor(Math.random() * 5),
    w2: Math.random() < 0.55 ? 5 + Math.random() * 10 : 0, tone2: Math.floor(Math.random() * 5),
  }));
  s.stacks.push({ x, gapY, passed: false, lines });
}

export function FlappyScout({ onEscape, autoFocus }: { onEscape: () => void; autoFocus: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const state = useRef<State>(newState());
  const [score, setScore] = useState(0);
  const [best, setBest] = useState(loadBest);
  const [phase, setPhase] = useState<State['phase']>('ready');
  const bestRef = useRef(best);

  const flap = () => {
    const s = state.current;
    if (s.phase === 'over') {
      // A short pause after a crash, so a flap in flight doesn't start the next run by accident.
      if (s.t < 0.45) return;
      state.current = newState();
      setScore(0);
      setPhase('ready');
      return;
    }
    if (s.phase === 'ready') {
      s.phase = 'play';
      setPhase('play');
      addStack(s, W + 10);
    }
    s.vy = FLAP;
    s.flapAt = s.t;
  };

  useEffect(() => {
    if (autoFocus) host.current?.focus();
  }, []);

  useEffect(() => {
    const el = canvas.current!;
    const ctx = el.getContext('2d')!;
    const css = getComputedStyle(el);
    const color = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
    const C = {
      bg: color('--bg', '#1f1f1f'), block: color('--surface-hi', '#2e2e2e'), edge: color('--raised', '#3a3a3a'),
      accent: color('--accent', '#f5a524'), accentHi: color('--accent-hi', '#ffb845'), ink: color('--accent-ink', '#1a1203'),
      text: color('--strong', '#ededed'), dim: color('--dim', '#7a7a7a'), ground: color('--surface', '#262626'),
      red: color('--red', '#e86464'), green: color('--green', '#4fbf7f'),
    };
    const TONES = [C.accent, color('--usage', '#5b9cf5'), C.green, color('--code', '#e5c07b'), '#c58af9'];
    let raf = 0;
    let last = performance.now();
    let groundX = 0;
    let drift = 0;

    const end = (s: State) => {
      s.phase = 'over';
      s.t = 0;
      setPhase('over');
      if (s.score > bestRef.current) {
        bestRef.current = s.score;
        setBest(s.score);
        try { localStorage.setItem(SAVE_KEY, String(s.score)); } catch {}
      }
    };

    const step = (dt: number) => {
      const s = state.current;
      s.t += dt;
      if (s.phase === 'ready') {
        s.y = H / 2 - 20 + Math.sin(s.t * 3) * 6;
        groundX = (groundX + 60 * dt) % 16;
        drift += 15 * dt;
        return;
      }
      if (s.phase === 'over' && s.y >= H - GROUND - BIRD_R) return;
      s.vy = Math.min(s.vy + GRAVITY * dt, 620);
      s.y += s.vy * dt;
      if (s.y < BIRD_R) { s.y = BIRD_R; s.vy = 0; }
      if (s.y >= H - GROUND - BIRD_R) {
        s.y = H - GROUND - BIRD_R;
        if (s.phase === 'play') end(s);
        return;
      }
      if (s.phase !== 'play') return;
      const speed = 118 + Math.min(s.score, 40) * 1.4;
      groundX = (groundX + speed * dt) % 16;
      drift += speed * 0.25 * dt;
      for (const p of s.stacks) {
        p.x -= speed * dt;
        if (!p.passed && p.x + STACK_W < BIRD_X) {
          p.passed = true;
          s.score++;
          setScore(s.score);
        }
        // Hit when the bird's circle overlaps the top or bottom stack.
        const nearX = Math.max(p.x, Math.min(BIRD_X, p.x + STACK_W));
        for (const [top, bottom] of [[0, p.gapY], [p.gapY + GAP, H - GROUND]]) {
          const nearY = Math.max(top, Math.min(s.y, bottom));
          if ((BIRD_X - nearX) ** 2 + (s.y - nearY) ** 2 < (BIRD_R - 1) ** 2) return end(s);
        }
      }
      s.stacks = s.stacks.filter((p) => p.x > -STACK_W - 4);
      const lastP = s.stacks[s.stacks.length - 1];
      if (!lastP || lastP.x < W - SPACING) addStack(s, (lastP?.x ?? W) + SPACING);
    };

    const rect = (x: number, y: number, w: number, h: number, r: number, fill: string) => {
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, r);
      ctx.fill();
    };

    const block = (x: number, y: number, line: Line) => {
      rect(x, y, STACK_W, BLOCK_H, 4, C.block);
      ctx.globalAlpha = 0.85;
      rect(x + 6 + line.indent, y + 6, line.w, 4, 2, TONES[line.tone]);
      if (line.w2) rect(x + 9 + line.indent + line.w, y + 6, line.w2, 4, 2, TONES[line.tone2]);
      ctx.globalAlpha = 1;
    };

    /** The prompt bar at the gap: a terminal's `>_`, with a blinking cursor. */
    const cap = (x: number, y: number, blink: boolean) => {
      rect(x - 3, y, STACK_W + 6, CAP_H, 4, C.edge);
      ctx.fillStyle = C.green;
      ctx.font = '700 10px "Cascadia Mono", Consolas, monospace';
      ctx.textAlign = 'left';
      ctx.fillText('>', x + 4, y + 11);
      if (blink) ctx.fillRect(x + 12, y + 4, 6, 8);
    };

    const stack = (p: Stack, blink: boolean) => {
      // The top stack hangs down to the gap, the bottom one rises from the ground to it; both start at the gap.
      let y = p.gapY - CAP_H;
      cap(p.x, y, blink);
      for (let i = 0; y > -BLOCK_H; i++) {
        y -= BLOCK_H + BLOCK_GAP;
        block(p.x, y, p.lines[i % p.lines.length]);
      }
      y = p.gapY + GAP;
      cap(p.x, y, blink);
      y += CAP_H + BLOCK_GAP;
      for (let i = 12; y < H - GROUND; i++) {
        block(p.x, y, p.lines[i % p.lines.length]);
        y += BLOCK_H + BLOCK_GAP;
      }
    };

    const bird = (s: State) => {
      const tilt = s.phase === 'ready' ? 0 : Math.max(-0.5, Math.min(1.2, s.vy / 500));
      const flapping = s.flapAt >= 0 && s.t - s.flapAt < 0.14;
      const wing = flapping ? -0.9 : s.phase === 'ready' ? Math.sin(s.t * 12) * 0.5 : s.phase === 'over' ? 0.4 : 0.25;
      ctx.save();
      ctx.translate(BIRD_X, s.y);
      ctx.rotate(tilt);
      // tail
      ctx.fillStyle = '#d98a12';
      ctx.beginPath();
      ctx.moveTo(-9, -2);
      ctx.lineTo(-16, -6);
      ctx.lineTo(-15, 1);
      ctx.lineTo(-16, 5);
      ctx.lineTo(-9, 3);
      ctx.fill();
      // body
      const g = ctx.createRadialGradient(-2, -4, 1, 0, 0, 12);
      g.addColorStop(0, C.accentHi);
      g.addColorStop(1, s.phase === 'over' ? C.red : C.accent);
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.ellipse(0, 0, 11, 9.5, 0, 0, Math.PI * 2);
      ctx.fill();
      // belly
      ctx.fillStyle = '#ffe1a6';
      ctx.beginPath();
      ctx.ellipse(2.5, 3.5, 6, 4.5, 0, 0, Math.PI * 2);
      ctx.fill();
      // scout neckerchief
      ctx.fillStyle = C.green;
      ctx.beginPath();
      ctx.moveTo(2, 2.5);
      ctx.lineTo(9, 2.5);
      ctx.lineTo(5, 8);
      ctx.fill();
      // wing, flapping from the shoulder
      ctx.save();
      ctx.translate(-2, 0);
      ctx.rotate(wing);
      ctx.fillStyle = '#e0920f';
      ctx.beginPath();
      ctx.ellipse(-4, 0, 7, 4, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      // beak
      ctx.fillStyle = '#ff7a3d';
      ctx.beginPath();
      ctx.moveTo(9, -3);
      ctx.lineTo(16, -0.5);
      ctx.lineTo(9, 2);
      ctx.fill();
      // eye
      ctx.fillStyle = '#fff';
      ctx.beginPath();
      ctx.arc(5, -3.5, 3.4, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = C.ink;
      ctx.beginPath();
      if (s.phase === 'over') {
        ctx.strokeStyle = C.ink;
        ctx.lineWidth = 1.4;
        ctx.moveTo(3.5, -5);
        ctx.lineTo(7, -2);
        ctx.moveTo(7, -5);
        ctx.lineTo(3.5, -2);
        ctx.stroke();
      } else {
        ctx.arc(6, -3.5, 1.7, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.restore();
    };

    const draw = () => {
      const s = state.current;
      ctx.fillStyle = C.bg;
      ctx.fillRect(0, 0, W, H);
      // faint dots drifting behind, for a sense of speed
      ctx.fillStyle = C.edge;
      ctx.globalAlpha = 0.4;
      for (let i = 0; i < 16; i++) {
        const x = ((((i * 53 - drift) % (W + 20)) + W + 20) % (W + 20)) - 10;
        ctx.fillRect(x, 24 + ((i * 97) % (H - 80)), 2, 2);
      }
      ctx.globalAlpha = 1;
      const blink = Math.floor(s.t * 2) % 2 === 0;
      for (const p of s.stacks) stack(p, blink);
      ctx.fillStyle = C.ground;
      ctx.fillRect(0, H - GROUND, W, GROUND);
      ctx.fillStyle = C.edge;
      for (let x = -groundX; x < W; x += 16) ctx.fillRect(x, H - GROUND + 9, 8, 2);
      ctx.fillRect(0, H - GROUND, W, 1);
      bird(s);

      ctx.textAlign = 'center';
      ctx.fillStyle = C.text;
      if (s.phase !== 'ready') {
        ctx.font = '700 34px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif';
        ctx.fillText(String(s.score), W / 2, 56);
      } else {
        ctx.font = '700 18px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif';
        ctx.fillText('Flappy Scout', W / 2, 96);
        ctx.fillStyle = C.dim;
        ctx.font = '13px "Segoe UI", system-ui, sans-serif';
        ctx.fillText('Click, tap or press Space to flap', W / 2, 118);
      }
    };

    const frame = (now: number) => {
      // Long gaps (the window was hidden) don't fling the bird across the screen.
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      step(dt);
      draw();
      raf = requestAnimationFrame(frame);
    };

    const fit = () => {
      const dpr = window.devicePixelRatio || 1;
      el.width = W * dpr;
      el.height = H * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    fit();
    raf = requestAnimationFrame(frame);
    window.addEventListener('resize', fit);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', fit);
    };
  }, []);

  const key = (e: KeyboardEvent) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      return onEscape();
    }
    if (e.key === ' ' || e.key === 'ArrowUp' || e.key === 'w' || e.key === 'W' || e.key === 'Enter') {
      e.preventDefault();
      if (!e.repeat) flap();
    }
  };

  return (
    <div className="game-flappy" ref={host} tabIndex={0} onKeyDown={key} aria-label="Flappy Scout. Press Space to flap through the gaps.">
      <div className="row game-scores">
        <div className="game-score"><span>Score</span><b>{score}</b></div>
        <div className="game-score"><span>Best</span><b key={best} className={phase === 'over' && best === score && score > 0 ? 'game-best' : ''}>{best}</b></div>
      </div>
      <div className="flappy-stage" onPointerDown={(e) => { e.preventDefault(); host.current?.focus(); flap(); }}>
        <canvas ref={canvas} />
        {phase === 'over' && (
          <div className="flappy-over">
            <b className="strong big">{score > 0 && score === best ? 'New best!' : 'Crashed'}</b>
            <span className="small dim">Click or press Space to go again</span>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Space, W, ↑ or a click to flap. Don't hit the code.</div>
    </div>
  );
}

/** The bird on its own, for the game picker. */
export function FlappyIcon() {
  return (
    <svg viewBox="0 0 40 40" width="34" height="34" aria-hidden>
      <path d="M10 18L3 14L4 21L3 26L10 23Z" fill="#d98a12" />
      <ellipse cx="19" cy="20" rx="11" ry="9.5" fill="#f5a524" />
      <ellipse cx="21.5" cy="23.5" rx="6" ry="4.5" fill="#ffe1a6" />
      <path d="M21 22.5H28L24 28Z" fill="#4fbf7f" />
      <ellipse cx="13" cy="19" rx="7" ry="4" fill="#e0920f" transform="rotate(-25 13 19)" />
      <path d="M28 17L35 19.5L28 22Z" fill="#ff7a3d" />
      <circle cx="24" cy="16.5" r="3.4" fill="#fff" />
      <circle cx="25" cy="16.5" r="1.7" fill="#1a1203" />
    </svg>
  );
}
