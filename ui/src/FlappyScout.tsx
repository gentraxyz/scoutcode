// Flappy Scout: tap to keep Scout's bird aloft through the gaps between editor panes, over a city of code at night.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { crisp, FONT, loadNum, loop, noise, saveNum, SoundToggle, themeColors, tone, type Colors } from './arcade';

const W = 280;
const H = 360;
const GROUND = 30;
const FLOOR = H - GROUND;
const BIRD_X = 80;
/** The hit circle is a little smaller than the drawn bird, so grazes feel fair. */
const BIRD_R = 9;
const GRAVITY = 1500;
const FLAP = -430;
const MAX_FALL = 560;
const PANE_W = 52;
const LIP_W = 60;
const LIP_H = 16;
const SPACING = 165;
const SAVE_KEY = 'scout.flappy';
const MEDALS: [number, string][] = [[50, 'platinum'], [30, 'gold'], [20, 'silver'], [10, 'bronze']];

type Line = { indent: number; w: number; tone: number; w2: number; tone2: number };
type Pane = { x: number; gapY: number; gap: number; passed: boolean; lines: Line[] };
type Bit = { x: number; y: number; vx: number; vy: number; life: number; max: number; size: number; rot: number; vr: number; color: number };
type Phase = 'ready' | 'play' | 'dying' | 'over';
type State = {
  phase: Phase; y: number; vy: number; rot: number; panes: Pane[]; score: number; t: number; flapAt: number;
  scroll: number; shake: number; flash: number; popAt: number; bits: Bit[]; overAt: number;
};

const rand = (a: number, b: number) => a + Math.random() * (b - a);
const medalFor = (score: number) => MEDALS.find(([n]) => score >= n)?.[1] ?? null;

function newState(): State {
  return { phase: 'ready', y: H / 2 - 24, vy: 0, rot: 0, panes: [], score: 0, t: 0, flapAt: -1, scroll: 0, shake: 0, flash: 0, popAt: -1, bits: [], overAt: 0 };
}

function addPane(s: State, x: number) {
  // The gap narrows as the score climbs, and never jumps too far from the last one, so every run is beatable.
  const gap = Math.max(98, 132 - s.score * 1.2);
  const prev = s.panes[s.panes.length - 1]?.gapY ?? (FLOOR - gap) / 2;
  const lo = 40;
  const hi = FLOOR - gap - 40;
  const gapY = Math.max(lo, Math.min(hi, prev + rand(-105, 105)));
  const lines = [...Array(40)].map(() => ({
    indent: [0, 0, 6, 12][Math.floor(Math.random() * 4)],
    w: rand(8, 18), tone: Math.floor(Math.random() * 5),
    w2: Math.random() < 0.55 ? rand(4, 10) : 0, tone2: Math.floor(Math.random() * 5),
  }));
  s.panes.push({ x, gapY, gap, passed: false, lines });
}

/** The far skyline: buildings with lit windows, laid out once and tiled. */
const CITY = (() => {
  const out: { x: number; w: number; h: number; lit: boolean[] }[] = [];
  let x = 0;
  while (x < 520) {
    const w = Math.round(rand(16, 38));
    const h = Math.round(rand(28, 112));
    const cols = Math.floor((w - 4) / 6);
    const rows = Math.floor((h - 6) / 8);
    out.push({ x, w, h, lit: [...Array(cols * rows)].map(() => Math.random() < 0.22) });
    x += w + Math.round(rand(1, 5));
  }
  return { blocks: out, width: x };
})();

export function FlappyScout({ onEscape, autoFocus }: { onEscape: () => void; autoFocus: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const state = useRef<State>(newState());
  const [score, setScore] = useState(0);
  const [best, setBest] = useState(() => loadNum(SAVE_KEY));
  const [phase, setPhase] = useState<Phase>('ready');
  const [newBest, setNewBest] = useState(false);
  const bestRef = useRef(best);

  const flap = () => {
    const s = state.current;
    if (s.phase === 'dying') return;
    if (s.phase === 'over') {
      // A short pause after the card shows, so a flap in flight doesn't start the next run by accident.
      if (s.t - s.overAt < 0.4) return;
      state.current = newState();
      setScore(0);
      setNewBest(false);
      setPhase('ready');
      return;
    }
    if (s.phase === 'ready') {
      s.phase = 'play';
      setPhase('play');
      addPane(s, W + 40);
    }
    s.vy = FLAP;
    s.flapAt = s.t;
    tone(420, 720, 0.09, 'triangle', 0.035);
    // A puff of feathers behind the bird.
    for (let i = 0; i < 3; i++) {
      s.bits.push({ x: BIRD_X - 8, y: s.y + rand(-3, 5), vx: rand(-90, -40), vy: rand(-20, 60), life: 0, max: rand(0.35, 0.6), size: rand(2, 3.5), rot: rand(0, 6), vr: rand(-8, 8), color: 0 });
    }
  };

  useEffect(() => {
    if (autoFocus) host.current?.focus();
  }, []);

  useEffect(() => {
    const el = canvas.current!;
    const ctx = el.getContext('2d')!;
    let C: Colors = themeColors(el);
    let TONES = [C.accent, C.blue, C.green, C.code, '#c58af9'];
    const recolor = () => {
      C = themeColors(el);
      TONES = [C.accent, C.blue, C.green, C.code, '#c58af9'];
    };
    // The theme can change while the game is open.
    const watch = new MutationObserver(recolor);
    watch.observe(document.documentElement, { attributes: true });

    const crash = (s: State) => {
      s.phase = 'dying';
      s.shake = 1;
      s.flash = 1;
      s.vy = Math.min(s.vy, -120);
      setPhase('dying');
      noise(0.22, 0.14, 700);
      tone(260, 70, 0.32, 'sawtooth', 0.035);
      for (let i = 0; i < 10; i++) {
        const a = rand(0, Math.PI * 2);
        const v = rand(60, 170);
        s.bits.push({ x: BIRD_X, y: s.y, vx: Math.cos(a) * v, vy: Math.sin(a) * v - 40, life: 0, max: rand(0.5, 0.9), size: rand(2, 4), rot: rand(0, 6), vr: rand(-10, 10), color: i % 3 });
      }
      if (s.score > bestRef.current) {
        bestRef.current = s.score;
        setBest(s.score);
        setNewBest(true);
        saveNum(SAVE_KEY, s.score);
      }
    };

    const step = (dt: number) => {
      const s = state.current;
      s.t += dt;
      s.shake = Math.max(0, s.shake - dt * 3);
      s.flash = Math.max(0, s.flash - dt * 4);
      for (const b of s.bits) {
        b.life += dt;
        b.vy += 420 * dt;
        b.x += b.vx * dt;
        b.y += b.vy * dt;
        b.rot += b.vr * dt;
      }
      s.bits = s.bits.filter((b) => b.life < b.max);

      if (s.phase === 'ready') {
        s.y = H / 2 - 24 + Math.sin(s.t * 3.2) * 5;
        s.rot = 0;
        s.scroll += 70 * dt;
        return;
      }
      if (s.phase === 'over') return;

      s.vy = Math.min(s.vy + GRAVITY * dt, MAX_FALL);
      s.y += s.vy * dt;
      // Nose up on a flap, then a lazy dive once it's really falling.
      const target = s.vy < 180 ? -0.42 : Math.min(1.5, ((s.vy - 180) / 300) * 1.5);
      s.rot += (target - s.rot) * Math.min(1, dt * (target > s.rot ? 6 : 14));
      if (s.y < BIRD_R) {
        s.y = BIRD_R;
        s.vy = 0;
      }
      if (s.y >= FLOOR - BIRD_R) {
        s.y = FLOOR - BIRD_R;
        if (s.phase === 'play') crash(s);
        s.phase = 'over';
        s.overAt = s.t;
        s.rot = 1.5;
        noise(0.1, 0.08, 260);
        setPhase('over');
        return;
      }
      if (s.phase !== 'play') return;

      const speed = 128 + Math.min(s.score, 40) * 1.3;
      s.scroll += speed * dt;
      for (const p of s.panes) {
        p.x -= speed * dt;
        if (!p.passed && p.x + PANE_W / 2 < BIRD_X) {
          p.passed = true;
          s.score++;
          s.popAt = s.t;
          setScore(s.score);
          tone(990, 990, 0.06, 'square', 0.022);
          tone(1480, 1480, 0.12, 'square', 0.022, 0.06);
        }
        // Hit when the bird's circle touches a pane or its lip.
        for (const [x0, x1, y0, y1] of [
          [p.x, p.x + PANE_W, -50, p.gapY - LIP_H],
          [p.x - (LIP_W - PANE_W) / 2, p.x + (LIP_W + PANE_W) / 2, p.gapY - LIP_H, p.gapY],
          [p.x - (LIP_W - PANE_W) / 2, p.x + (LIP_W + PANE_W) / 2, p.gapY + p.gap, p.gapY + p.gap + LIP_H],
          [p.x, p.x + PANE_W, p.gapY + p.gap + LIP_H, FLOOR],
        ]) {
          const nx = Math.max(x0, Math.min(BIRD_X, x1));
          const ny = Math.max(y0, Math.min(s.y, y1));
          if ((BIRD_X - nx) ** 2 + (s.y - ny) ** 2 < BIRD_R ** 2) return crash(s);
        }
      }
      s.panes = s.panes.filter((p) => p.x > -LIP_W);
      const lastP = s.panes[s.panes.length - 1];
      if (!lastP || lastP.x < W - SPACING) addPane(s, (lastP?.x ?? W) + SPACING);
    };

    const rect = (x: number, y: number, w: number, h: number, r: number, fill: string) => {
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, r);
      ctx.fill();
    };

    const sky = (s: State) => {
      ctx.fillStyle = C.sidebar;
      ctx.fillRect(0, 0, W, H);
      // A warm glow low on the horizon.
      const glow = ctx.createRadialGradient(W * 0.6, FLOOR, 10, W * 0.6, FLOOR, H * 0.9);
      glow.addColorStop(0, C.accent);
      glow.addColorStop(1, 'transparent');
      ctx.globalAlpha = 0.13;
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, W, H);
      // stars, barely moving
      ctx.fillStyle = C.text;
      for (let i = 0; i < 26; i++) {
        const x = (((i * 71 - s.scroll * 0.04) % W) + W) % W;
        const y = 10 + ((i * 53) % 170);
        ctx.globalAlpha = 0.15 + 0.25 * Math.abs(Math.sin(s.t * 0.8 + i));
        ctx.fillRect(x, y, i % 4 === 0 ? 2 : 1, i % 4 === 0 ? 2 : 1);
      }
      // the far city, then a nearer, darker one
      for (const [k, depth, alpha, lift] of [[0.12, 0.7, 0.55, 26], [0.3, 1, 0.9, 0]] as const) {
        const off = (s.scroll * k) % CITY.width;
        for (let rep = -1; rep < 2; rep++) {
          for (const b of CITY.blocks) {
            const bx = b.x + rep * CITY.width - off;
            if (bx > W || bx + b.w < 0) continue;
            const h = b.h * depth;
            const top = FLOOR - h - lift;
            ctx.globalAlpha = alpha;
            ctx.fillStyle = k < 0.2 ? C.surface : C.bg;
            ctx.fillRect(bx, top, b.w, h + lift);
            if (k > 0.2) {
              const cols = Math.floor((b.w - 4) / 6);
              ctx.fillStyle = C.code;
              ctx.globalAlpha = 0.5;
              b.lit.forEach((on, i) => {
                const wy = top + 5 + Math.floor(i / cols) * 8;
                if (on && wy < FLOOR - 4) ctx.fillRect(bx + 3 + (i % cols) * 6, wy, 2, 3);
              });
            }
          }
        }
      }
      ctx.globalAlpha = 1;
    };

    /** An editor pane: a gutter with line numbers, code, and a prompt lip at the gap. */
    const pane = (p: Pane, blink: boolean) => {
      const top = p.gapY - LIP_H;
      const bottom = p.gapY + p.gap + LIP_H;
      for (const [y0, y1, dir] of [[-4, top, -1], [bottom, FLOOR + 4, 1]] as const) {
        rect(p.x, y0, PANE_W, y1 - y0, 3, C.surfaceHi);
        ctx.fillStyle = C.surface;
        ctx.fillRect(p.x + 2, y0, 9, y1 - y0);
        // a sheen down the left edge
        ctx.fillStyle = C.text;
        ctx.globalAlpha = 0.06;
        ctx.fillRect(p.x + 11, y0, 3, y1 - y0);
        ctx.globalAlpha = 1;
        // code lines run away from the gap, anchored to it so they don't crawl
        const start = dir < 0 ? top - 8 : bottom + 8;
        for (let i = 0; i < p.lines.length; i++) {
          const y = start + dir * i * 11;
          if (dir < 0 ? y < -4 : y > FLOOR) break;
          const l = p.lines[i];
          ctx.fillStyle = C.faint;
          ctx.fillRect(p.x + 4, y, 4, 2);
          ctx.globalAlpha = 0.8;
          rect(p.x + 16 + l.indent, y - 1, Math.min(l.w, PANE_W - 22 - l.indent), 4, 2, TONES[l.tone]);
          if (l.w2 && 16 + l.indent + l.w + 3 + l.w2 < PANE_W - 4) rect(p.x + 19 + l.indent + l.w, y - 1, l.w2, 4, 2, TONES[l.tone2]);
          ctx.globalAlpha = 1;
        }
      }
      // the lips, with an accent edge facing the gap
      for (const [y, edge] of [[top, top + LIP_H - 2], [p.gapY + p.gap, p.gapY + p.gap]] as const) {
        const lx = p.x - (LIP_W - PANE_W) / 2;
        rect(lx, y, LIP_W, LIP_H, 4, C.edge);
        ctx.fillStyle = C.accent;
        ctx.fillRect(lx + 2, edge, LIP_W - 4, 2);
        ctx.fillStyle = C.green;
        ctx.font = `700 10px "Cascadia Mono", Consolas, monospace`;
        ctx.textAlign = 'left';
        ctx.fillText('>', lx + 6, y + 11.5);
        if (blink) ctx.fillRect(lx + 14, y + 4, 5, 8);
      }
    };

    const ground = (s: State) => {
      ctx.fillStyle = C.surface;
      ctx.fillRect(0, FLOOR, W, GROUND);
      ctx.fillStyle = C.surfaceHi;
      ctx.fillRect(0, FLOOR, W, 8);
      // slanted stripes along the top band, at the panes' speed
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, FLOOR, W, 8);
      ctx.clip();
      ctx.fillStyle = C.edge;
      for (let x = -(s.scroll % 14) - 14; x < W + 14; x += 14) {
        ctx.beginPath();
        ctx.moveTo(x, FLOOR + 8);
        ctx.lineTo(x + 7, FLOOR + 8);
        ctx.lineTo(x + 13, FLOOR);
        ctx.lineTo(x + 6, FLOOR);
        ctx.fill();
      }
      ctx.restore();
      ctx.fillStyle = C.accent;
      ctx.globalAlpha = 0.6;
      ctx.fillRect(0, FLOOR, W, 1);
      ctx.globalAlpha = 1;
    };

    const bird = (s: State) => {
      const since = s.flapAt >= 0 ? s.t - s.flapAt : 9;
      const dead = s.phase === 'dying' || s.phase === 'over';
      // Wings beat hard right after a flap, then settle into a glide.
      const wing = dead ? 0.5 : s.phase === 'ready' ? Math.sin(s.t * 14) * 0.6 : since < 0.22 ? Math.sin((since / 0.22) * Math.PI * 2) * 0.9 - 0.3 : 0.2;
      const squash = !dead && since < 0.1 ? 1 - Math.sin((since / 0.1) * Math.PI) * 0.12 : 1;
      ctx.save();
      ctx.translate(BIRD_X, s.y);
      ctx.rotate(s.rot);
      ctx.scale(1 / squash, squash);
      // tail
      ctx.fillStyle = '#d98a12';
      ctx.beginPath();
      ctx.moveTo(-9, -2);
      ctx.lineTo(-17, -7);
      ctx.lineTo(-15, 0);
      ctx.lineTo(-17, 6);
      ctx.lineTo(-9, 3);
      ctx.fill();
      // body
      const g = ctx.createRadialGradient(-3, -5, 1, 0, 0, 13);
      g.addColorStop(0, '#ffc766');
      g.addColorStop(1, dead ? '#c9563a' : '#f5a524');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.ellipse(0, 0, 12, 10.5, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = 'rgb(0 0 0 / 25%)';
      ctx.lineWidth = 1;
      ctx.stroke();
      // belly
      ctx.fillStyle = '#ffe1a6';
      ctx.beginPath();
      ctx.ellipse(2.5, 4, 6.5, 4.8, 0, 0, Math.PI * 2);
      ctx.fill();
      // scout neckerchief
      ctx.fillStyle = '#3fae6e';
      ctx.beginPath();
      ctx.moveTo(1, 2);
      ctx.lineTo(10, 2);
      ctx.lineTo(5.5, 9);
      ctx.fill();
      ctx.fillStyle = '#2f8a55';
      ctx.beginPath();
      ctx.arc(5.5, 3, 1.6, 0, Math.PI * 2);
      ctx.fill();
      // wing, from the shoulder
      ctx.save();
      ctx.translate(-2, 0);
      ctx.rotate(wing);
      ctx.fillStyle = '#e0920f';
      ctx.beginPath();
      ctx.ellipse(-4, 0, 8, 4.5, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      // beak
      ctx.fillStyle = '#ff7a3d';
      ctx.beginPath();
      ctx.moveTo(10, -3.5);
      ctx.lineTo(18, -0.5);
      ctx.lineTo(10, 2.5);
      ctx.fill();
      // eye
      ctx.fillStyle = '#fff';
      ctx.beginPath();
      ctx.arc(5.5, -4, 3.8, 0, Math.PI * 2);
      ctx.fill();
      if (dead) {
        ctx.strokeStyle = '#1a1203';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(3.8, -5.8);
        ctx.lineTo(7.6, -2.2);
        ctx.moveTo(7.6, -5.8);
        ctx.lineTo(3.8, -2.2);
        ctx.stroke();
      } else {
        ctx.fillStyle = '#1a1203';
        ctx.beginPath();
        ctx.arc(6.6, -4, 1.9, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = '#fff';
        ctx.fillRect(6.8, -5.4, 1, 1);
      }
      ctx.restore();
    };

    const bits = (s: State) => {
      const colors = ['#ffe1a6', '#f5a524', '#3fae6e'];
      for (const b of s.bits) {
        ctx.globalAlpha = 1 - b.life / b.max;
        ctx.save();
        ctx.translate(b.x, b.y);
        ctx.rotate(b.rot);
        ctx.fillStyle = colors[b.color];
        ctx.beginPath();
        ctx.ellipse(0, 0, b.size, b.size * 0.5, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }
      ctx.globalAlpha = 1;
    };

    /** Big outlined text, the way arcade scores look. */
    const shout = (text: string, x: number, y: number, size: number, fill = C.text) => {
      ctx.font = `800 ${size}px ${FONT}`;
      ctx.textAlign = 'center';
      ctx.lineJoin = 'round';
      ctx.lineWidth = Math.max(3, size / 7);
      ctx.strokeStyle = 'rgb(0 0 0 / 55%)';
      ctx.strokeText(text, x, y);
      ctx.fillStyle = fill;
      ctx.fillText(text, x, y);
    };

    const draw = () => {
      const s = state.current;
      ctx.save();
      if (s.shake > 0) ctx.translate(rand(-1, 1) * 6 * s.shake * s.shake, rand(-1, 1) * 6 * s.shake * s.shake);
      sky(s);
      const blink = Math.floor(s.t * 2) % 2 === 0;
      for (const p of s.panes) pane(p, blink);
      ground(s);
      bits(s);
      bird(s);
      ctx.restore();

      if (s.phase === 'ready') {
        shout('Flappy Scout', W / 2, 84, 26, C.accentHi);
        ctx.font = `600 12px ${FONT}`;
        ctx.fillStyle = C.dim;
        ctx.textAlign = 'center';
        ctx.fillText('GET READY', W / 2, 106);
        // a pulsing tap ring beside the bird
        const k = (s.t * 1.2) % 1;
        ctx.strokeStyle = C.text;
        ctx.globalAlpha = 1 - k;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(BIRD_X + 62, s.y + 30, 6 + k * 12, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = 1;
        ctx.fillStyle = C.text;
        ctx.beginPath();
        ctx.arc(BIRD_X + 62, s.y + 30, 4, 0, Math.PI * 2);
        ctx.fill();
        ctx.font = `600 12px ${FONT}`;
        ctx.fillStyle = C.dim;
        ctx.fillText('Tap, click or Space', W / 2, FLOOR - 34);
      } else if (s.phase === 'play' || s.phase === 'dying') {
        const since = s.t - s.popAt;
        const pop = s.popAt >= 0 && since < 0.18 ? 1 + Math.sin((since / 0.18) * Math.PI) * 0.28 : 1;
        shout(String(s.score), W / 2, 58, Math.round(38 * pop));
      }
      if (s.flash > 0) {
        ctx.fillStyle = '#fff';
        ctx.globalAlpha = s.flash * 0.7;
        ctx.fillRect(0, 0, W, H);
        ctx.globalAlpha = 1;
      }
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
    if (e.key === ' ' || e.key === 'ArrowUp' || e.key === 'w' || e.key === 'W' || e.key === 'Enter') {
      e.preventDefault();
      if (!e.repeat) flap();
    }
  };

  const medal = medalFor(score);
  return (
    <div className="game-flappy" ref={host} tabIndex={0} onKeyDown={key} aria-label="Flappy Scout. Press Space to flap through the gaps.">
      <div className="row game-scores">
        <div className="game-score"><span>Score</span><b>{score}</b></div>
        <div className="game-score"><span>Best</span><b key={best} className={newBest ? 'game-best' : ''}>{best}</b></div>
        <span className="grow" />
        <SoundToggle />
      </div>
      <div className="flappy-stage" onPointerDown={(e) => { if (e.button > 0) return; e.preventDefault(); host.current?.focus(); flap(); }}>
        <canvas ref={canvas} />
        {phase === 'over' && (
          <div className="arcade-over">
            <b className="arcade-title">Game over</b>
            <div className="arcade-card">
              <span className={`arcade-medal ${medal ?? 'none'}`} title={medal ? `${medal} medal` : 'Medals start at 10'}>{medal ? '' : '?'}</span>
              <div className="arcade-nums">
                <span>Score</span><b>{score}</b>
                <span>Best {newBest && <i className="arcade-new">New</i>}</span><b>{best}</b>
              </div>
            </div>
            <span className="small dim">Space or click to fly again</span>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Space, W, ↑ or a click to flap. Medals at 10, 20, 30 and 50.</div>
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
      <path d="M21 22.5H28L24 28Z" fill="#3fae6e" />
      <ellipse cx="13" cy="19" rx="7" ry="4" fill="#e0920f" transform="rotate(-25 13 19)" />
      <path d="M28 17L35 19.5L28 22Z" fill="#ff7a3d" />
      <circle cx="24" cy="16.5" r="3.4" fill="#fff" />
      <circle cx="25" cy="16.5" r="1.7" fill="#1a1203" />
    </svg>
  );
}
