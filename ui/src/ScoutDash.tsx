// Scout Dash: an endless three-lane run down a data highway. Switch lanes round server racks, run up ramps onto
// their roofs, jump bug barriers, slide under firewalls, and grab tokens and power-ups on the way. Bump a rack's
// side and a bug swarm gives chase; bump another before it gives up and it catches you.
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { crisp, FONT, loadNum, loop, noise, saveNum, SoundToggle, themeColors, tone, type Colors } from './arcade';

const W = 280;
const H = 360;
/** The camera: the horizon's height on screen, how high and how far behind the runner it sits, and its focal length. */
const HOR = 112;
const CAM_H = 3;
const CAM_Z = 4;
const FOCAL = 280;
const Z_FAR = 72;
const Z_NEAR = -3.3;
/** Lanes are one unit apart; the road runs from -1.5 to 1.5. */
const ROAD = 1.5;
const SEG = 2;
const GRAVITY = 26;
const JUMP = 8.4;
/** With spring boots on, a jump clears a rack and lands on its roof. */
const BOOTS_JUMP = 11.6;
const SLIDE_S = 0.62;
const STAND_H = 0.9;
const SLIDE_H = 0.4;
/** A rack's height, which is also its roof you can run along, and the ramp up to it. */
const TOP = 2.1;
const RAMP = 3.2;
/** How close to a roof you must be to land on it rather than hit its face. */
const ON_TOP = TOP - 0.45;
const CART_SPEED = 7;
const CHASE_S = 4;
const SAVE_KEY = 'scout.dash';

type Kind = 'block' | 'low' | 'high';
/** `ramp` racks can be run up; `vz` racks are carts rolling toward the runner. */
type Obstacle = { kind: Kind; lane: number; z: number; len: number; seed: number; ramp: boolean; vz: number };
type Token = { x: number; y: number; z: number; taken: boolean; spin: number };
type PowerKind = 'magnet' | 'boots' | 'x2' | 'board';
type Power = { kind: PowerKind; lane: number; z: number; taken: boolean };
type Bit = { x: number; y: number; z: number; vx: number; vy: number; vz: number; life: number; max: number; color: string; size: number };
type Phase = 'ready' | 'play' | 'over';
type Move = 'left' | 'right' | 'jump' | 'slide' | 'board';
type State = {
  phase: Phase; t: number; runT: number; dist: number; speed: number;
  lane: number; fromLane: number; x: number; y: number; vy: number; grounded: boolean; slide: number; queuedJump: number; queuedSlide: boolean;
  stumbleAt: number; chase: number; caught: boolean; obstacles: Obstacle[]; tokens: Token[]; powers: Power[]; bits: Bit[]; nextRow: number; nextPower: number;
  coins: number; points: number; score: number; shake: number; flash: number; overAt: number;
  /** Seconds left on each power-up; `boards` is how many hoverboards are in hand. */
  magnet: number; boots: number; x2: number; board: number; boards: number; invuln: number;
};

const POWER_S: Record<PowerKind, number> = { magnet: 9, boots: 10, x2: 12, board: 20 };

const MOVES: Record<string, Move | undefined> = {
  ArrowLeft: 'left', a: 'left', ArrowRight: 'right', d: 'right', ArrowUp: 'jump', w: 'jump', ' ': 'jump', Enter: 'jump',
  ArrowDown: 'slide', s: 'slide', Shift: 'board', e: 'board',
};

const rand = (a: number, b: number) => a + Math.random() * (b - a);
const pick = <T,>(xs: T[]) => xs[Math.floor(Math.random() * xs.length)];

function newState(): State {
  return {
    phase: 'ready', t: 0, runT: 0, dist: 0, speed: 0,
    lane: 0, fromLane: 0, x: 0, y: 0, vy: 0, grounded: true, slide: 0, queuedJump: -1, queuedSlide: false,
    stumbleAt: -9, chase: 0, caught: false, obstacles: [], tokens: [], powers: [], bits: [], nextRow: 34, nextPower: 180,
    coins: 0, points: 0, score: 0, shake: 0, flash: 0, overAt: 0,
    magnet: 0, boots: 0, x2: 0, board: 0, boards: 1, invuln: 0,
  };
}

/** Whether a rack in `lane` takes up any of the road between z0 and z1. */
function rackAt(s: State, lane: number, z0: number, z1: number) {
  return s.obstacles.some((o) => o.lane === lane && o.kind === 'block' && o.z < z1 && o.z + o.len > z0);
}

/** One row of trouble across the lanes. At least one lane is always open or passable, so every row can be beaten. */
function addRow(s: State, z: number) {
  const lanes = [-1, 0, 1].sort(() => Math.random() - 0.5);
  const late = s.dist > 150;
  // A long rack from an earlier row may still fill a lane here; it counts as a wall.
  const covered = lanes.filter((l) => rackAt(s, l, z - 2, z + 2));
  let walls = covered.length;
  let placed = 0;
  for (const lane of lanes) {
    if (covered.includes(lane)) continue;
    let kind: Kind | null = null;
    if (walls < 2 && Math.random() < (late ? 0.55 : 0.35)) {
      kind = 'block';
      walls++;
    } else if (Math.random() < 0.4) kind = pick<Kind>(['low', 'high']);
    if (!kind) continue;
    placed++;
    let len = 0.35;
    let ramp = false;
    let vz = 0;
    if (kind === 'block') {
      const long = late && Math.random() < 0.5;
      len = long ? rand(9, 16) : pick([2.4, 2.4, 4]);
      ramp = long && Math.random() < 0.7;
      // A cart rolls toward you, so it only goes where the lane in front of it is clear.
      if (!long && late && Math.random() < 0.3 && !s.obstacles.some((o) => o.lane === lane && o.z > z - 45 && o.z < z)) {
        len = 4;
        vz = -CART_SPEED;
      }
      if (ramp) {
        for (let k = RAMP + 1; k < len - 0.6; k += 1.2) s.tokens.push({ x: lane, y: TOP + 0.45, z: z + k, taken: false, spin: Math.random() * 6 });
      }
    }
    s.obstacles.push({ kind, lane, z, len, seed: Math.random() * 1000, ramp, vz });
    // tokens arc over some bug barriers
    if (kind === 'low' && Math.random() < 0.6) {
      for (let k = -2; k <= 2; k++) s.tokens.push({ x: lane, y: 0.5 + 1.0 * (1 - (k / 2.6) ** 2), z: z + k * 0.9, taken: false, spin: Math.random() * 6 });
    }
  }
  if (!placed && !covered.length) {
    s.obstacles.push({ kind: pick<Kind>(['low', 'high']), lane: lanes[0], z, len: 0.35, seed: Math.random() * 1000, ramp: false, vz: 0 });
  }
  // a line of tokens down an open lane, leading into the row
  const open = lanes.filter((l) => !rackAt(s, l, z - 12, z + 1) && !s.obstacles.some((o) => o.lane === l && Math.abs(o.z - z) < 1));
  if (open.length && Math.random() < 0.75) {
    const lane = pick(open);
    const n = 4 + Math.floor(Math.random() * 4);
    for (let k = 0; k < n; k++) s.tokens.push({ x: lane, y: 0.45, z: z - 3 - k * 1.1, taken: false, spin: Math.random() * 6 });
  }
}

/** The height of whatever the runner stands on: the road, a ramp, or a rack's roof. */
function groundAt(s: State) {
  let g = 0;
  for (const o of s.obstacles) {
    if (o.kind !== 'block' || Math.abs(s.x - o.lane) > 0.5) continue;
    const rz = o.z - s.dist;
    const start = o.ramp ? rz + RAMP : rz;
    if (o.ramp && rz <= 0 && start > 0) g = Math.max(g, (-rz / RAMP) * TOP);
    else if (start <= 0.3 && rz + o.len > -0.25 && s.y >= ON_TOP) g = Math.max(g, TOP);
  }
  return g;
}

export function ScoutDash({ onEscape, autoFocus }: { onEscape: () => void; autoFocus: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const state = useRef<State>(newState());
  const swipe = useRef<{ x: number; y: number; done: boolean } | null>(null);
  const lastTap = useRef(0);
  const [score, setScore] = useState(0);
  const [coins, setCoins] = useState(0);
  const [best, setBest] = useState(() => loadNum(SAVE_KEY));
  const [phase, setPhase] = useState<Phase>('ready');
  const [newBest, setNewBest] = useState(false);
  const [caught, setCaught] = useState(false);
  const bestRef = useRef(best);

  /** Starts a run from the ready screen, or a new one once the card has been up a moment. True when it did. */
  const begin = () => {
    const s = state.current;
    if (s.phase === 'over') {
      if (s.t - s.overAt < 0.5) return true;
      state.current = newState();
      setScore(0);
      setCoins(0);
      setNewBest(false);
      setCaught(false);
      setPhase('ready');
      return true;
    }
    if (s.phase === 'ready') {
      s.phase = 'play';
      setPhase('play');
      tone(330, 660, 0.14, 'triangle', 0.035);
      return true;
    }
    return false;
  };

  const control = (move: Move) => {
    if (begin()) return;
    const s = state.current;
    if (move === 'left' || move === 'right') {
      const next = Math.max(-1, Math.min(1, s.lane + (move === 'left' ? -1 : 1)));
      if (next === s.lane) {
        // Into the road's edge: a little bump, nothing more.
        s.shake = Math.max(s.shake, 0.25);
        return;
      }
      s.fromLane = s.lane;
      s.lane = next;
      noise(0.08, 0.035, 1800);
    } else if (move === 'jump') {
      if (s.grounded) {
        s.vy = s.boots > 0 ? BOOTS_JUMP : JUMP;
        s.grounded = false;
        s.slide = 0;
        tone(300, s.boots > 0 ? 900 : 620, 0.12, 'triangle', 0.03);
      } else s.queuedJump = s.t;
    } else if (move === 'slide') {
      if (!s.grounded) {
        // Down in the air drops fast, into a slide.
        s.vy = Math.min(s.vy, -15);
        s.queuedSlide = true;
      } else if (s.slide <= 0) {
        s.slide = SLIDE_S;
        noise(0.16, 0.04, 1200);
      }
    } else if (s.boards > 0 && s.board <= 0) {
      s.boards--;
      s.board = POWER_S.board;
      tone(440, 880, 0.18, 'sine', 0.04);
      tone(660, 1320, 0.18, 'sine', 0.03, 0.07);
    }
  };

  useEffect(() => {
    if (autoFocus) host.current?.focus();
  }, []);

  useEffect(() => {
    const el = canvas.current!;
    const ctx = el.getContext('2d')!;
    let C: Colors = themeColors(el);
    const watch = new MutationObserver(() => { C = themeColors(el); });
    watch.observe(document.documentElement, { attributes: true });
    let camX = 0;
    let camY = 0;

    const burst = (s: State, x: number, y: number, n: number, colors: string[], power: number) => {
      for (let i = 0; i < n; i++) {
        s.bits.push({ x, y, z: 0, vx: rand(-1, 1) * power, vy: rand(0.4, 1.2) * power, vz: rand(-0.5, 1) * power, life: 0, max: rand(0.35, 0.8), color: pick(colors), size: rand(0.04, 0.09) });
      }
    };

    const over = (s: State, byBugs = false) => {
      s.phase = 'over';
      s.overAt = s.t;
      s.shake = 1;
      s.flash = 1;
      s.caught = byBugs;
      if (byBugs) s.chase = CHASE_S;
      setPhase('over');
      setCaught(byBugs);
      noise(0.3, 0.16, 500);
      tone(220, 55, 0.4, 'sawtooth', 0.04);
      burst(s, s.x, s.y + 0.5, 16, ['#f5a524', '#ffe1a6', '#3fae6e'], 3);
      if (s.score > bestRef.current) {
        bestRef.current = s.score;
        setBest(s.score);
        setNewBest(true);
        saveNum(SAVE_KEY, s.score);
      }
    };

    /** A hit the hoverboard takes instead of you: it breaks, and you get a moment to recover. */
    const breakBoard = (s: State) => {
      s.board = 0;
      s.invuln = 1.2;
      s.shake = 0.7;
      noise(0.25, 0.12, 900);
      tone(600, 150, 0.3, 'square', 0.03);
      burst(s, s.x, 0.15, 12, [C.blue, '#ffffff', C.accent], 2.2);
    };

    const step = (dt: number) => {
      const s = state.current;
      s.t += dt;
      s.shake = Math.max(0, s.shake - dt * 2.5);
      s.flash = Math.max(0, s.flash - dt * 3);
      for (const b of s.bits) {
        b.life += dt;
        b.vy -= 9 * dt;
        b.x += b.vx * dt;
        b.y = Math.max(0, b.y + b.vy * dt);
        b.z += (b.vz - (s.phase === 'play' ? s.speed : 0)) * dt;
      }
      s.bits = s.bits.filter((b) => b.life < b.max);
      if (s.phase === 'over') return;

      if (s.phase === 'ready') {
        s.dist += 3 * dt;
        return;
      }

      s.runT += dt;
      s.speed = 13 + Math.min(15, s.runT * 0.22);
      s.dist += s.speed * dt;
      const mult = s.x2 > 0 ? 2 : 1;
      s.points += (s.speed * dt * mult) / 3;
      for (const k of ['magnet', 'boots', 'x2', 'board', 'invuln', 'chase'] as const) s[k] = Math.max(0, s[k] - dt);
      for (const o of s.obstacles) o.z += o.vz * dt;

      // running, ramps and roofs, jumping, sliding
      s.x += (s.lane - s.x) * Math.min(1, dt * 16);
      const g = groundAt(s);
      if (s.vy > 0 || s.y > g + 0.01) {
        s.grounded = false;
        s.vy -= GRAVITY * dt;
        s.y += s.vy * dt;
        if (s.y <= g) {
          s.y = g;
          s.vy = 0;
          s.grounded = true;
          burst(s, s.x, g + 0.02, 4, [C.dim], 1);
          if (s.queuedSlide) {
            s.slide = SLIDE_S;
            s.queuedSlide = false;
          } else if (s.t - s.queuedJump < 0.14) {
            s.vy = s.boots > 0 ? BOOTS_JUMP : JUMP;
            s.grounded = false;
            tone(300, 620, 0.12, 'triangle', 0.03);
          }
        }
      } else {
        s.y = g;
        s.vy = 0;
        s.grounded = true;
      }
      s.slide = Math.max(0, s.slide - dt);

      // spawning, far ahead
      while (s.nextRow < s.dist + Z_FAR) {
        const z = s.nextRow;
        addRow(s, z);
        if (z > s.nextPower) {
          const free = [-1, 0, 1].filter((l) => !rackAt(s, l, z - 9, z - 3));
          if (free.length) s.powers.push({ kind: pick<PowerKind>(['magnet', 'boots', 'x2', 'board', 'magnet', 'x2']), lane: pick(free), z: z - 6, taken: false });
          s.nextPower += rand(220, 380);
        }
        s.nextRow += Math.max(14, s.speed * 0.95) + rand(0, 9);
      }

      // hits
      const top = s.y + (s.slide > 0 ? SLIDE_H : STAND_H);
      if (s.invuln <= 0) {
        for (const o of s.obstacles) {
          if (Math.abs(s.x - o.lane) > 0.7) continue;
          const rz = o.z - s.dist;
          let side = false;
          if (o.kind === 'block') {
            const start = o.ramp ? rz + RAMP : rz;
            if (start > 0.3 || rz + o.len < -0.3 || s.y >= ON_TOP) continue;
            side = start < -0.1;
          } else {
            if (rz > 0.3 || rz + o.len < -0.3) continue;
            if (o.kind === 'low' ? s.y >= 0.55 : top <= 0.62 || s.y >= 1.75) continue;
          }
          if (side && s.lane === o.lane) {
            // Swerved into a rack's side rather than its front: bounce back, and the bugs give chase.
            if (s.runT - s.stumbleAt < 0.35) continue;
            if (s.chase > 0) return over(s, true);
            s.stumbleAt = s.runT;
            s.chase = CHASE_S;
            s.lane = s.fromLane;
            s.shake = 0.6;
            noise(0.15, 0.1, 400);
            tone(90, 70, 0.5, 'sawtooth', 0.03);
            continue;
          }
          if (s.board > 0) {
            breakBoard(s);
            break;
          }
          return over(s);
        }
      }

      // tokens and power-ups
      for (const k of s.tokens) {
        if (k.taken) continue;
        let rz = k.z - s.dist;
        if (s.magnet > 0 && rz < 10 && rz > -1) {
          const pull = Math.min(1, dt * 9);
          k.x += (s.x - k.x) * pull;
          k.y += (s.y + 0.45 - k.y) * pull;
          k.z -= rz * pull * 0.5;
          rz = k.z - s.dist;
        }
        if (Math.abs(rz) < 0.5 && Math.abs(k.x - s.x) < 0.55 && Math.abs(k.y - (s.y + 0.45)) < 0.75) {
          k.taken = true;
          s.coins++;
          s.points += 5 * mult;
          setCoins(s.coins);
          tone(mult > 1 ? 1760 : 1320, mult > 1 ? 2350 : 1760, 0.07, 'square', 0.018);
          burst(s, k.x, k.y, 3, [C.accentHi, '#fff'], 1.2);
        }
      }
      for (const p of s.powers) {
        const rz = p.z - s.dist;
        if (p.taken || Math.abs(rz) > 0.6 || Math.abs(p.lane - s.x) > 0.6 || Math.abs(s.y + 0.4 - 0.6) > 0.9) continue;
        p.taken = true;
        if (p.kind === 'board') s.boards++;
        else s[p.kind] = POWER_S[p.kind];
        tone(520, 1040, 0.25, 'triangle', 0.04);
        tone(780, 1560, 0.25, 'triangle', 0.03, 0.08);
        burst(s, p.lane, 0.6, 8, [C.blue, '#ffffff'], 1.6);
      }
      const score = Math.floor(s.points);
      if (score !== s.score) {
        s.score = score;
        setScore(score);
      }
      s.obstacles = s.obstacles.filter((o) => o.z + o.len - s.dist > Z_NEAR - 1);
      s.tokens = s.tokens.filter((k) => !k.taken && k.z - s.dist > Z_NEAR - 1);
      s.powers = s.powers.filter((p) => !p.taken && p.z - s.dist > Z_NEAR - 1);
    };

    // ---------- drawing ----------

    const P = (x: number, y: number, z: number) => {
      const sc = FOCAL / (Math.max(z, Z_NEAR) + CAM_Z);
      return { x: W / 2 + (x - camX) * sc, y: HOR + (CAM_H + camY - y) * sc, sc };
    };

    const poly = (pts: { x: number; y: number }[], fill: string | CanvasGradient) => {
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (const p of pts.slice(1)) ctx.lineTo(p.x, p.y);
      ctx.closePath();
      ctx.fill();
    };

    /** A flat strip of the floor between two x's, from z0 (near) to z1 (far). */
    const strip = (x0: number, x1: number, z0: number, z1: number, fill: string) => {
      poly([P(x0, 0, z0), P(x1, 0, z0), P(x1, 0, z1), P(x0, 0, z1)], fill);
    };

    /** A box; its front face is handed back so it can be decorated. */
    const box = (cx: number, w: number, y0: number, y1: number, z0: number, z1: number, front: string, top: string, side: string) => {
      if (z1 < Z_NEAR) return null;
      z0 = Math.max(z0, Z_NEAR);
      const l = cx - w / 2;
      const r = cx + w / 2;
      // the side that faces the camera
      if (r < camX) poly([P(r, y0, z0), P(r, y1, z0), P(r, y1, z1), P(r, y0, z1)], side);
      if (l > camX) poly([P(l, y0, z0), P(l, y1, z0), P(l, y1, z1), P(l, y0, z1)], side);
      if (y1 < CAM_H + camY) poly([P(l, y1, z0), P(r, y1, z0), P(r, y1, z1), P(l, y1, z1)], top);
      const a = P(l, y1, z0);
      const b = P(r, y0, z0);
      ctx.fillStyle = front;
      ctx.fillRect(a.x, a.y, b.x - a.x, b.y - a.y);
      return { x: a.x, y: a.y, w: b.x - a.x, h: b.y - a.y, sc: a.sc };
    };

    const world = (s: State) => {
      // sky
      const sky = ctx.createLinearGradient(0, 0, 0, HOR);
      sky.addColorStop(0, C.sidebar);
      sky.addColorStop(1, C.surface);
      ctx.fillStyle = sky;
      ctx.fillRect(0, 0, W, H);
      const glow = ctx.createRadialGradient(W / 2, HOR, 4, W / 2, HOR, 170);
      glow.addColorStop(0, C.accent);
      glow.addColorStop(1, 'transparent');
      ctx.globalAlpha = 0.18;
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, W, HOR);
      ctx.globalAlpha = 1;
      const hor = P(0, 0, 1e6).y;
      // the skyline on the horizon, sliding a touch as the runner changes lanes
      ctx.fillStyle = C.bg;
      for (let i = -2; i < 22; i++) {
        const bw = 12 + ((i * 37) % 17);
        const bh = 10 + ((i * 53) % 34);
        ctx.fillRect(i * 15 - camX * 6, hor - bh, bw, bh);
      }
      // the floor beyond the road
      ctx.fillRect(0, hor, W, H - hor);

      // the road, in bands from far to near, so it streams past
      const first = Math.floor((s.dist + Z_FAR) / SEG);
      for (let i = first; i * SEG >= s.dist + Z_NEAR - SEG; i--) {
        const z0 = Math.max(i * SEG - s.dist, Z_NEAR);
        const z1 = (i + 1) * SEG - s.dist;
        if (z1 <= Z_NEAR) continue;
        const even = i % 2 === 0;
        strip(-ROAD - 0.6, -ROAD, z0, z1, even ? C.surface : C.sidebar);
        strip(ROAD, ROAD + 0.6, z0, z1, even ? C.surface : C.sidebar);
        strip(-ROAD, ROAD, z0, z1, even ? C.surfaceHi : C.surface);
        if (even) {
          strip(-0.53, -0.47, z0, z1 - 0.8, C.faint);
          strip(0.47, 0.53, z0, z1 - 0.8, C.faint);
        }
      }
      // glowing edges
      for (const x of [-ROAD, ROAD]) {
        ctx.globalAlpha = 0.85;
        strip(x - 0.04, x + 0.04, Z_NEAR, Z_FAR, C.accent);
        ctx.globalAlpha = 0.18;
        strip(x - 0.16, x + 0.16, Z_NEAR, Z_FAR, C.accent);
        ctx.globalAlpha = 1;
      }
      // a fade into the distance
      const haze = ctx.createLinearGradient(0, hor, 0, hor + 40);
      haze.addColorStop(0, C.surface);
      haze.addColorStop(1, 'transparent');
      ctx.fillStyle = haze;
      ctx.fillRect(0, hor, W, 40);
    };

    const pylon = (x: number, z: number, s: State) => {
      const f = box(x, 0.3, 0, 2.6, z, z + 0.3, C.surfaceHi, C.edge, C.surface);
      if (!f) return;
      const lit = Math.floor(s.t * 3 + z) % 4 !== 0;
      ctx.fillStyle = lit ? C.accentHi : C.faint;
      ctx.globalAlpha = lit ? 1 : 0.6;
      ctx.fillRect(f.x + f.w * 0.25, f.y + f.sc * 0.08, f.w * 0.5, Math.max(1, f.sc * 0.08));
      ctx.fillStyle = C.green;
      for (let k = 0; k < 4; k++) ctx.fillRect(f.x + f.w * 0.3, f.y + f.sc * (0.5 + k * 0.4), Math.max(1, f.w * 0.15), Math.max(1, f.sc * 0.06));
      ctx.globalAlpha = 1;
    };

    /** The ramp up to a rack's roof: a slope with chevrons pointing up it. */
    const ramp = (lane: number, rz: number) => {
      const z0 = Math.max(rz, Z_NEAR);
      const z1 = rz + RAMP;
      if (z1 < Z_NEAR) return;
      const h0 = ((z0 - rz) / RAMP) * TOP;
      const l = lane - 0.46;
      const r = lane + 0.46;
      if (r < camX) poly([P(r, 0, z0), P(r, h0, z0), P(r, TOP, z1), P(r, 0, z1)], C.surface);
      if (l > camX) poly([P(l, 0, z0), P(l, h0, z0), P(l, TOP, z1), P(l, 0, z1)], C.surface);
      poly([P(l, h0, z0), P(r, h0, z0), P(r, TOP, z1), P(l, TOP, z1)], C.edge);
      ctx.fillStyle = C.accent;
      for (let k = 0.5; k < RAMP; k += 0.8) {
        const za = rz + k;
        const zb = za + 0.25;
        if (za < z0) continue;
        const ya = (k / RAMP) * TOP;
        const yb = ((k + 0.25) / RAMP) * TOP;
        ctx.globalAlpha = 0.8;
        poly([P(lane - 0.3, ya, za), P(lane, yb, zb), P(lane + 0.3, ya, za), P(lane, ya + (yb - ya) * 0.4, za + 0.1)], C.accent);
        ctx.globalAlpha = 1;
      }
    };

    const obstacle = (o: Obstacle, rz: number, s: State) => {
      if (o.kind === 'block') {
        // a server rack, LEDs blinking down its face; a cart has headlights and rolls toward you
        const start = o.ramp ? rz + RAMP : rz;
        const f = box(o.lane, 0.92, 0, TOP, start, rz + o.len, C.surfaceHi, o.len > 6 ? C.raised : C.edge, C.surface);
        if (f && f.w >= 3 && start > Z_NEAR) {
          ctx.strokeStyle = C.faint;
          ctx.lineWidth = Math.max(0.5, f.sc * 0.02);
          ctx.strokeRect(f.x + 0.5, f.y + 0.5, f.w - 1, f.h - 1);
          const rows = 6;
          for (let k = 0; k < rows; k++) {
            const y = f.y + f.h * (0.08 + k * 0.15);
            ctx.fillStyle = C.sidebar;
            ctx.fillRect(f.x + f.w * 0.1, y, f.w * 0.8, f.h * 0.1);
            const on = Math.sin(s.t * 5 + o.seed + k * 1.7) > -0.2;
            ctx.fillStyle = k % 3 === 0 ? C.blue : C.green;
            ctx.globalAlpha = on ? 1 : 0.25;
            ctx.fillRect(f.x + f.w * 0.16, y + f.h * 0.03, Math.max(1, f.w * 0.06), Math.max(1, f.h * 0.04));
            ctx.fillRect(f.x + f.w * 0.28, y + f.h * 0.03, Math.max(1, f.w * 0.06), Math.max(1, f.h * 0.04));
            ctx.globalAlpha = 1;
          }
          if (o.vz) {
            ctx.fillStyle = C.red;
            ctx.fillRect(f.x, f.y + f.h * 0.92, f.w, f.h * 0.08);
            for (const hx of [0.25, 0.75]) {
              const g = ctx.createRadialGradient(f.x + f.w * hx, f.y + f.h * 0.86, 0, f.x + f.w * hx, f.y + f.h * 0.86, f.w * 0.35);
              g.addColorStop(0, '#fff6c8');
              g.addColorStop(0.3, 'rgb(255 230 140 / 60%)');
              g.addColorStop(1, 'transparent');
              ctx.fillStyle = g;
              ctx.fillRect(f.x + f.w * (hx - 0.35), f.y + f.h * 0.5, f.w * 0.7, f.h * 0.7);
            }
          }
        }
        if (o.ramp) ramp(o.lane, rz);
      } else if (o.kind === 'low') {
        // a bug barrier: hazard stripes, jump it
        const f = box(o.lane, 0.92, 0, 0.55, rz, rz + o.len, C.red, '#ff9a8a', '#a23f3f');
        if (!f || f.w < 3) return;
        ctx.save();
        ctx.beginPath();
        ctx.rect(f.x, f.y, f.w, f.h);
        ctx.clip();
        ctx.fillStyle = '#2a1414';
        const sw = Math.max(3, f.w / 6);
        for (let x = f.x - f.h; x < f.x + f.w; x += sw * 2) {
          ctx.beginPath();
          ctx.moveTo(x, f.y + f.h);
          ctx.lineTo(x + sw, f.y + f.h);
          ctx.lineTo(x + sw + f.h, f.y);
          ctx.lineTo(x + f.h, f.y);
          ctx.fill();
        }
        ctx.restore();
        if (f.w > 30) {
          ctx.fillStyle = '#fff';
          ctx.font = `800 ${Math.round(f.h * 0.5)}px ${FONT}`;
          ctx.textAlign = 'center';
          ctx.fillText('BUG', f.x + f.w / 2, f.y + f.h * 0.68);
        }
      } else {
        // a firewall: a hanging panel on two posts, slide under it
        for (const dx of [-0.42, 0.42]) box(o.lane + dx, 0.07, 0, 1.75, rz, rz + 0.07, C.edge, C.edge, C.surface);
        const f = box(o.lane, 0.96, 0.62, 1.75, rz, rz + o.len, C.accent, C.accentHi, '#b87512');
        if (!f || f.w < 3) return;
        const g = ctx.createLinearGradient(0, f.y, 0, f.y + f.h);
        g.addColorStop(0, C.accentHi);
        g.addColorStop(1, '#d9661e');
        ctx.fillStyle = g;
        ctx.fillRect(f.x, f.y, f.w, f.h);
        ctx.fillStyle = 'rgb(0 0 0 / 25%)';
        for (let k = 1; k < 4; k++) ctx.fillRect(f.x, f.y + (f.h * k) / 4, f.w, Math.max(1, f.h * 0.03));
        if (f.w > 26) {
          ctx.fillStyle = C.ink;
          ctx.font = `800 ${Math.round(f.h * 0.3)}px ${FONT}`;
          ctx.textAlign = 'center';
          ctx.fillText('403', f.x + f.w / 2, f.y + f.h * 0.62);
        }
      }
    };

    const token = (k: Token, rz: number, s: State) => {
      const p = P(k.x, k.y, rz);
      const r = 0.2 * p.sc;
      if (r < 0.8) return;
      const w = Math.abs(Math.cos(s.t * 5 + k.spin)) * r;
      ctx.fillStyle = '#b87512';
      ctx.beginPath();
      ctx.ellipse(p.x, p.y, Math.max(w, r * 0.18) + r * 0.1, r, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = C.accent;
      ctx.beginPath();
      ctx.ellipse(p.x, p.y, Math.max(w, r * 0.12), r * 0.92, 0, 0, Math.PI * 2);
      ctx.fill();
      if (w > r * 0.45) {
        ctx.fillStyle = C.ink;
        ctx.font = `800 ${Math.round(r * 1.1)}px ${FONT}`;
        ctx.textAlign = 'center';
        ctx.fillText('S', p.x, p.y + r * 0.38);
      }
    };

    // ---------- power-up icons, drawn at (x, y) with radius r ----------

    const magnetIcon = (x: number, y: number, r: number) => {
      ctx.lineCap = 'butt';
      ctx.lineWidth = r * 0.55;
      ctx.strokeStyle = C.red;
      ctx.beginPath();
      ctx.arc(x, y, r, Math.PI, 0, true);
      ctx.stroke();
      ctx.fillStyle = '#e6e6e6';
      ctx.fillRect(x - r - r * 0.275, y - r * 0.55, r * 0.55, r * 0.5);
      ctx.fillRect(x + r - r * 0.275, y - r * 0.55, r * 0.55, r * 0.5);
    };

    const bootsIcon = (x: number, y: number, r: number) => {
      ctx.strokeStyle = C.green;
      ctx.lineWidth = Math.max(1, r * 0.28);
      ctx.lineJoin = 'round';
      ctx.beginPath();
      ctx.moveTo(x - r * 0.6, y + r * 0.9);
      for (let k = 0; k < 4; k++) ctx.lineTo(x + (k % 2 ? -r * 0.6 : r * 0.6), y + r * (0.5 - k * 0.4));
      ctx.stroke();
      ctx.fillStyle = C.green;
      ctx.beginPath();
      ctx.roundRect(x - r * 0.9, y - r * 1.05, r * 1.8, r * 0.5, r * 0.2);
      ctx.fill();
    };

    const x2Icon = (x: number, y: number, r: number) => {
      ctx.fillStyle = '#9b6cf0';
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = '#fff';
      ctx.font = `800 ${Math.max(6, Math.round(r * 1.05))}px ${FONT}`;
      ctx.textAlign = 'center';
      ctx.fillText('×2', x, y + r * 0.37);
    };

    const boardIcon = (x: number, y: number, r: number) => {
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(-0.35);
      ctx.fillStyle = C.blue;
      ctx.beginPath();
      ctx.roundRect(-r * 1.2, -r * 0.38, r * 2.4, r * 0.76, r * 0.38);
      ctx.fill();
      ctx.fillStyle = '#fff';
      ctx.fillRect(-r * 0.7, -r * 0.08, r * 1.4, r * 0.16);
      ctx.restore();
    };

    const ICON: Record<PowerKind, (x: number, y: number, r: number) => void> = { magnet: magnetIcon, boots: bootsIcon, x2: x2Icon, board: boardIcon };

    const power = (p: Power, rz: number, s: State) => {
      const q = P(p.lane, 0.6 + Math.sin(s.t * 4) * 0.08, rz);
      const r = 0.2 * q.sc;
      if (r < 1) return;
      ctx.globalAlpha = 0.25 + 0.15 * Math.sin(s.t * 6);
      ctx.fillStyle = C.blue;
      ctx.beginPath();
      ctx.arc(q.x, q.y, r * 2.1, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
      ICON[p.kind](q.x, q.y + (p.kind === 'magnet' ? r * 0.3 : 0), r);
    };

    /** Scout's bird, seen from behind as it runs, on a hoverboard when it has one going. */
    const runner = (s: State) => {
      const riding = s.board > 0;
      const lift = riding ? 0.16 : 0;
      const shadow = P(s.x, s.grounded ? s.y : groundAt(s), 0);
      const above = s.y - (s.grounded ? s.y : groundAt(s));
      ctx.fillStyle = 'rgb(0 0 0 / 35%)';
      ctx.beginPath();
      ctx.ellipse(shadow.x, shadow.y, (shadow.sc * 0.34) / (1 + above * 0.6), (shadow.sc * 0.1) / (1 + above * 0.6), 0, 0, Math.PI * 2);
      ctx.fill();

      // Flickers while it recovers from a broken board.
      if (s.invuln > 0 && Math.floor(s.t * 16) % 2 === 0) ctx.globalAlpha = 0.45;
      const air = !s.grounded;
      const sliding = s.slide > 0;
      const dead = s.phase === 'over';
      const cycle = s.phase === 'play' && !air && !sliding && !dead && !riding ? s.t * 15 : 0;
      const bob = riding ? Math.sin(s.t * 6) * 0.03 : Math.abs(Math.sin(cycle)) * 0.05;
      const tilt = (s.lane - s.x) * 0.5 + (dead ? 0.9 : 0);

      if (riding) {
        const b = P(s.x, s.y + 0.06 + bob, 0);
        ctx.save();
        ctx.translate(b.x, b.y);
        ctx.rotate(tilt * 0.8);
        const glow = ctx.createRadialGradient(0, 0, 0, 0, 0, b.sc * 0.5);
        glow.addColorStop(0, 'rgb(91 156 245 / 55%)');
        glow.addColorStop(1, 'transparent');
        ctx.fillStyle = glow;
        ctx.fillRect(-b.sc * 0.5, -b.sc * 0.3, b.sc, b.sc * 0.6);
        ctx.fillStyle = C.blue;
        ctx.beginPath();
        ctx.roundRect(-b.sc * 0.36, -b.sc * 0.05, b.sc * 0.72, b.sc * 0.1, b.sc * 0.05);
        ctx.fill();
        ctx.fillStyle = '#ffffff';
        ctx.fillRect(-b.sc * 0.2, -b.sc * 0.012, b.sc * 0.4, b.sc * 0.024);
        ctx.restore();
      }

      const base = P(s.x, s.y + bob + lift, 0);
      const u = base.sc;
      const ry = (sliding ? 0.22 : 0.4) * u;
      const rx = (sliding ? 0.4 : 0.33) * u;
      ctx.save();
      ctx.translate(base.x, base.y - ry - 0.05 * u);
      ctx.rotate(tilt);
      // feet, with springs under them in spring boots
      if (!sliding) {
        for (const [dx, ph] of [[-0.12, 0], [0.12, Math.PI]] as const) {
          const up = air || riding ? 0.04 : Math.max(0, Math.sin(cycle + ph)) * 0.08;
          if (s.boots > 0) {
            ctx.strokeStyle = C.green;
            ctx.lineWidth = Math.max(1, 0.025 * u);
            ctx.beginPath();
            ctx.moveTo(dx * u - 0.04 * u, ry + (0.06 - up) * u);
            ctx.lineTo(dx * u + 0.04 * u, ry + (0.09 - up) * u);
            ctx.lineTo(dx * u - 0.04 * u, ry + (0.12 - up) * u);
            ctx.stroke();
          }
          ctx.fillStyle = '#ff7a3d';
          ctx.beginPath();
          ctx.ellipse(dx * u, ry + (0.02 - up) * u, 0.06 * u, 0.035 * u, 0, 0, Math.PI * 2);
          ctx.fill();
        }
      }
      // wings: pumping as it runs, spread in the air, out for balance on the board
      const flap = air ? -0.9 + Math.sin(s.t * 20) * 0.3 : sliding ? 0.2 : riding ? -0.3 + Math.sin(s.t * 5) * 0.1 : Math.sin(cycle) * 0.35;
      for (const side of [-1, 1]) {
        ctx.save();
        ctx.translate(side * rx * 0.85, -ry * 0.1);
        ctx.rotate(side * (0.5 - flap * 0.6));
        ctx.fillStyle = '#e0920f';
        ctx.beginPath();
        ctx.ellipse(side * 0.08 * u, 0.04 * u, 0.1 * u, 0.22 * u, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }
      // body
      const g = ctx.createRadialGradient(-rx * 0.3, -ry * 0.4, 1, 0, 0, Math.max(rx, ry) * 1.1);
      g.addColorStop(0, '#ffc766');
      g.addColorStop(1, dead ? '#c9563a' : '#e8961c');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.ellipse(0, 0, rx, ry, 0, 0, Math.PI * 2);
      ctx.fill();
      // tail feathers
      ctx.fillStyle = '#d98a12';
      for (const a of [-0.35, 0, 0.35]) {
        ctx.save();
        ctx.translate(0, ry * 0.55);
        ctx.rotate(a);
        ctx.beginPath();
        ctx.ellipse(0, 0.1 * u, 0.05 * u, 0.13 * u, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }
      // the neckerchief, knotted at the back, its ends flying
      const flutter = Math.sin(s.t * 18) * 0.06 * u;
      ctx.fillStyle = '#3fae6e';
      ctx.beginPath();
      ctx.moveTo(-0.17 * u, -ry * 0.45);
      ctx.lineTo(0.17 * u, -ry * 0.45);
      ctx.lineTo(0, -ry * 0.45 + 0.18 * u);
      ctx.fill();
      ctx.fillStyle = '#2f8a55';
      ctx.beginPath();
      ctx.moveTo(-0.02 * u, -ry * 0.42);
      ctx.lineTo(-0.16 * u + flutter, -ry * 0.42 + 0.22 * u);
      ctx.lineTo(-0.08 * u + flutter, -ry * 0.42 + 0.24 * u);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(0, -ry * 0.44, 0.04 * u, 0, Math.PI * 2);
      ctx.fill();
      // a tuft on top
      ctx.fillStyle = '#f5a524';
      ctx.beginPath();
      ctx.ellipse(-0.04 * u, -ry - 0.03 * u, 0.04 * u, 0.09 * u, -0.4, 0, Math.PI * 2);
      ctx.ellipse(0.05 * u, -ry - 0.02 * u, 0.035 * u, 0.08 * u, 0.4, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      ctx.globalAlpha = 1;
    };

    /** The bug swarm behind the runner while it's chasing: little red beetles scuttling at its heels. */
    const swarm = (s: State) => {
      if (s.chase <= 0) return;
      const fade = s.caught ? 1 : Math.min(1, s.chase / 0.6, (CHASE_S - s.chase) / 0.3);
      // Closer while it's fresh, falling back as it gives up; on a catch it swarms the runner.
      const back = s.caught ? 0.1 : -0.6 - (1 - s.chase / CHASE_S) * 2;
      ctx.globalAlpha = fade;
      for (let i = 0; i < 9; i++) {
        const ox = Math.sin(i * 2.3 + s.t * 7) * 0.5 + (i - 4) * 0.12;
        const oz = back - (i % 3) * 0.35 + Math.sin(s.t * 11 + i) * 0.08;
        const p = P(s.x + ox, s.caught ? s.y + 0.3 + (i % 3) * 0.2 : s.y + 0.05, oz);
        const r = 0.09 * p.sc;
        ctx.strokeStyle = '#2a1414';
        ctx.lineWidth = Math.max(1, r * 0.25);
        ctx.beginPath();
        for (const side of [-1, 1]) {
          for (const k of [-0.5, 0, 0.5]) {
            ctx.moveTo(p.x, p.y + k * r);
            ctx.lineTo(p.x + side * r * 1.6, p.y + k * r + Math.sin(s.t * 30 + i + k) * r * 0.4);
          }
        }
        ctx.stroke();
        ctx.fillStyle = C.red;
        ctx.beginPath();
        ctx.ellipse(p.x, p.y, r, r * 1.2, 0, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = '#fff';
        ctx.fillRect(p.x - r * 0.5, p.y - r * 0.8, r * 0.3, r * 0.3);
        ctx.fillRect(p.x + r * 0.2, p.y - r * 0.8, r * 0.3, r * 0.3);
      }
      ctx.globalAlpha = 1;
    };

    const bits = (s: State) => {
      for (const b of s.bits) {
        const p = P(b.x, b.y, b.z);
        ctx.globalAlpha = 1 - b.life / b.max;
        ctx.fillStyle = b.color;
        const r = Math.max(1, b.size * p.sc);
        ctx.fillRect(p.x - r / 2, p.y - r / 2, r, r);
      }
      ctx.globalAlpha = 1;
    };

    const shout = (text: string, x: number, y: number, size: number, align: CanvasTextAlign, fill = C.text) => {
      ctx.font = `800 ${size}px ${FONT}`;
      ctx.textAlign = align;
      ctx.lineJoin = 'round';
      ctx.lineWidth = Math.max(3, size / 6);
      ctx.strokeStyle = 'rgb(0 0 0 / 55%)';
      ctx.strokeText(text, x, y);
      ctx.fillStyle = fill;
      ctx.fillText(text, x, y);
    };

    /** Each running power-up's icon and the time it has left, down the right side. */
    const timers = (s: State) => {
      let y = 18;
      for (const k of ['board', 'magnet', 'boots', 'x2'] as const) {
        if (s[k] <= 0) continue;
        ICON[k](W - 16, y + (k === 'magnet' ? 3 : 0), 6);
        ctx.fillStyle = 'rgb(0 0 0 / 40%)';
        ctx.fillRect(W - 76, y - 2, 46, 5);
        ctx.fillStyle = k === 'boots' ? C.green : k === 'x2' ? '#9b6cf0' : C.blue;
        ctx.fillRect(W - 76, y - 2, 46 * (s[k] / POWER_S[k]), 5);
        y += 20;
      }
      // hoverboards in hand
      if (s.boards > 0 && s.board <= 0) {
        boardIcon(18, 18, 6);
        ctx.font = `700 11px ${FONT}`;
        ctx.textAlign = 'left';
        ctx.fillStyle = C.text;
        ctx.fillText(`×${s.boards}  Shift`, 30, 22);
      }
    };

    const draw = () => {
      const s = state.current;
      camX += (s.x * 0.55 - camX) * 0.18;
      // The camera rises with the runner onto roofs, so the road ahead stays in view.
      camY += (Math.min(s.y, TOP) * 0.55 - camY) * 0.1;
      ctx.save();
      if (s.shake > 0) ctx.translate(rand(-1, 1) * 7 * s.shake * s.shake, rand(-1, 1) * 7 * s.shake * s.shake);
      world(s);

      // everything that stands on the road, far to near, with the runner slotted in at its depth
      const items: [number, () => void][] = [];
      const off = s.dist % 7;
      for (let z = 7 * Math.ceil(Z_FAR / 7) - off; z > Z_NEAR - 1; z -= 7) {
        items.push([z, () => pylon(-ROAD - 0.9, z, s)], [z, () => pylon(ROAD + 0.9, z, s)]);
      }
      for (const o of s.obstacles) {
        const rz = o.z - s.dist;
        // A rack the runner is standing on goes behind it, however far back its far end is.
        const onIt = o.kind === 'block' && Math.abs(o.lane - s.x) < 0.5 && rz < 0.3 && rz + o.len > 0;
        if (rz < Z_FAR) items.push([onIt ? 0.01 : rz + Math.min(o.len, 3) / 2, () => obstacle(o, rz, s)]);
      }
      for (const k of s.tokens) {
        const rz = k.z - s.dist;
        if (rz < Z_FAR * 0.7) items.push([rz, () => token(k, rz, s)]);
      }
      for (const p of s.powers) {
        const rz = p.z - s.dist;
        if (rz < Z_FAR) items.push([rz, () => power(p, rz, s)]);
      }
      items.push([0, () => runner(s)]);
      items.push([-0.5, () => swarm(s)]);
      items.sort((a, b) => b[0] - a[0]);
      for (const [, f] of items) f();
      bits(s);
      ctx.restore();

      if (s.phase === 'play') timers(s);
      else if (s.phase === 'ready') {
        shout('Scout Dash', W / 2, 52, 28, 'center', C.accentHi);
        ctx.font = `600 12px ${FONT}`;
        ctx.fillStyle = C.text;
        ctx.textAlign = 'center';
        ctx.fillText('← →  switch lanes', W / 2, 76);
        ctx.fillText('↑  jump bugs  ·  ↓  slide under firewalls', W / 2, 93);
        ctx.fillText('Shift or double-tap  ride a hoverboard', W / 2, 110);
        ctx.fillStyle = C.dim;
        ctx.globalAlpha = 0.6 + 0.4 * Math.sin(s.t * 4);
        ctx.fillText('Press any arrow, Space or tap to run', W / 2, H - 14);
        ctx.globalAlpha = 1;
      }
      if (s.flash > 0) {
        ctx.fillStyle = '#fff';
        ctx.globalAlpha = s.flash * 0.6;
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
    const move = MOVES[e.key.length === 1 ? e.key.toLowerCase() : e.key];
    if (!move) return;
    e.preventDefault();
    if (!e.repeat) control(move);
  };

  // Swipes steer; a tap jumps, and a quick second tap rides a hoverboard. A swipe acts as soon as it's long enough.
  const down = (e: PointerEvent) => {
    if (e.button > 0) return;
    e.preventDefault();
    host.current?.focus();
    swipe.current = { x: e.clientX, y: e.clientY, done: false };
  };
  const drag = (e: PointerEvent) => {
    const from = swipe.current;
    if (!from || from.done) return;
    const dx = e.clientX - from.x;
    const dy = e.clientY - from.y;
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 22) return;
    from.done = true;
    control(Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? 'right' : 'left') : dy > 0 ? 'slide' : 'jump');
  };
  const up = () => {
    if (swipe.current && !swipe.current.done) {
      const now = performance.now();
      control(now - lastTap.current < 300 && state.current.phase === 'play' ? 'board' : 'jump');
      lastTap.current = now;
    }
    swipe.current = null;
  };

  return (
    <div className="game-dash" ref={host} tabIndex={0} onKeyDown={key} aria-label="Scout Dash. Arrow keys to switch lanes, jump and slide; Shift for a hoverboard.">
      <div className="row game-scores">
        <div className="game-score"><span>Score</span><b>{score}</b></div>
        <div className="game-score"><span>Tokens</span><b>{coins}</b></div>
        <div className="game-score"><span>Best</span><b key={best} className={newBest ? 'game-best' : ''}>{best}</b></div>
        <span className="grow" />
        <SoundToggle />
      </div>
      <div className="flappy-stage" onPointerDown={down} onPointerMove={drag} onPointerUp={up} onPointerCancel={() => { swipe.current = null; }}>
        <canvas ref={canvas} />
        {phase === 'over' && (
          <div className="arcade-over">
            <b className="arcade-title">{caught ? 'The bugs got you' : 'Crashed'}</b>
            <div className="arcade-card">
              <div className="arcade-nums">
                <span>Score</span><b>{score}</b>
                <span>Tokens</span><b>{coins}</b>
                <span>Best {newBest && <i className="arcade-new">New</i>}</span><b>{best}</b>
              </div>
            </div>
            <span className="small dim">Space or tap to run again</span>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Arrows/WASD or swipe; Shift or double-tap for a hoverboard. Run up ramps onto the racks.</div>
    </div>
  );
}

/** A server rack and a token, for the game picker. */
export function DashIcon() {
  return (
    <svg viewBox="0 0 40 40" width="34" height="34" aria-hidden>
      <path d="M14 6h12l10 30H4Z" fill="#2a2a2a" />
      <path d="M19.4 6h1.2l1 30h-3.2Z" fill="#4d4d4d" />
      <path d="M14 6L4 36" stroke="#f5a524" strokeWidth="1.6" />
      <path d="M26 6l10 30" stroke="#f5a524" strokeWidth="1.6" />
      <ellipse cx="20" cy="27" rx="6" ry="4" fill="#e8961c" />
      <ellipse cx="20" cy="23" rx="5.5" ry="6.5" fill="#f5a524" />
      <path d="M16.5 20h7L20 24Z" fill="#3fae6e" />
      <circle cx="29" cy="13" r="3.4" fill="#f5a524" stroke="#b87512" strokeWidth="1" />
    </svg>
  );
}
