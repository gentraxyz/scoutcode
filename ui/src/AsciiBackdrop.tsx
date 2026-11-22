// A field of dim letters behind the new-chat screen. Brightness comes from two layers of smooth noise that slide in
// different directions, so patches of letters drift and swirl slowly across the window; a few letters change each
// frame. Clicking the empty space sends a ring outward that lights and scrambles the letters it passes. It draws on a
// canvas at ~20 fps (60 while a ring spreads), stops while the window is hidden, and holds still for reduced motion.
import { useEffect, useRef } from 'react';
import { themeColor } from './theme';

const CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
const CELL_W = 7.2;
const CELL_H = 12;
const FONT = '600 10.5px "Cascadia Mono", "JetBrains Mono", Consolas, monospace';
const FRAME_MS = 50;
/** Distinct alpha steps, so letters can be batched by fill style. */
const LEVELS = 12;
/** Frame time while a ripple is moving, so the ring spreads smoothly. */
const RIPPLE_FRAME_MS = 16;
const RIPPLE_MS = 2200;
/** How far a ring has spread when it fades out, and how thick its crest is, in pixels. */
const RIPPLE_REACH = 900;
const RIPPLE_BAND = 26;
/** Clicks on these start no ripple: they do their own thing. */
const INTERACTIVE = 'button, a, input, textarea, select, label, [contenteditable], [role="button"], [role="menu"], .composer-column';

// Value noise on a 256-wide lattice, smoothly interpolated.
const PERM = new Uint8Array(512);
{
  const p = Array.from({ length: 256 }, (_, i) => i);
  for (let i = 255; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [p[i], p[j]] = [p[j], p[i]];
  }
  for (let i = 0; i < 512; i++) PERM[i] = p[i & 255];
}
const smooth = (t: number) => t * t * (3 - 2 * t);
function noise(x: number, y: number): number {
  const xi = Math.floor(x), yi = Math.floor(y);
  const xf = x - xi, yf = y - yi;
  const X = xi & 255, Y = yi & 255;
  const a = PERM[PERM[X] + Y] / 255, b = PERM[PERM[X + 1] + Y] / 255;
  const c = PERM[PERM[X] + Y + 1] / 255, d = PERM[PERM[X + 1] + Y + 1] / 255;
  const u = smooth(xf), v = smooth(yf);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}

export function AsciiBackdrop() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current!;
    const ctx = canvas.getContext('2d')!;
    const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let cols = 0, rows = 0, dpr = 1;
    let grid = new Uint8Array(0);
    let rgb = '237 237 237';
    let ripples: { x: number; y: number; start: number }[] = [];

    const readColor = () => {
      const hex = themeColor('strong') || '#ededed';
      const n = parseInt(hex.replace('#', '').slice(0, 6), 16);
      if (!Number.isNaN(n)) rgb = `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
    };

    const resize = () => {
      dpr = window.devicePixelRatio || 1;
      const { width, height } = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      cols = Math.ceil(width / CELL_W);
      rows = Math.ceil(height / CELL_H);
      grid = new Uint8Array(cols * rows);
      for (let i = 0; i < grid.length; i++) grid[i] = Math.floor(Math.random() * CHARS.length);
      readColor();
    };

    const buckets: number[][] = Array.from({ length: LEVELS }, () => []);
    const draw = (t: number) => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, cols * CELL_W, rows * CELL_H);
      ctx.font = FONT;
      ctx.textBaseline = 'top';
      // A handful of letters change each frame, so the field shimmers a little.
      for (let k = Math.ceil(grid.length / 220); k > 0; k--) grid[Math.floor(Math.random() * grid.length)] = Math.floor(Math.random() * CHARS.length);
      const s = t / 1000;
      // Rings still on screen, as [center x, center y, radius, strength] in pixels.
      ripples = ripples.filter((r) => t - r.start < RIPPLE_MS);
      const rings = ripples.map((r) => {
        const age = (t - r.start) / RIPPLE_MS;
        return [r.x, r.y, age * RIPPLE_REACH, (1 - age) * (1 - age)];
      });
      for (const b of buckets) b.length = 0;
      for (let y = 0; y < rows; y++) {
        const py = y * CELL_H + CELL_H / 2;
        for (let x = 0; x < cols; x++) {
          // Two noise layers sliding different ways; their product makes patches that form, drift and dissolve.
          const n1 = noise(x * 0.045 + s * 0.11, y * 0.07 - s * 0.04);
          const n2 = noise(x * 0.09 - s * 0.07 + 40, y * 0.12 + s * 0.06 + 40);
          let v = n1 * 0.7 + n2 * 0.5 - 0.7;
          let rippled = false;
          if (rings.length) {
            const px = x * CELL_W + CELL_W / 2;
            for (const [cx, cy, radius, strength] of rings) {
              const off = (Math.hypot(px - cx, py - cy) - radius) / RIPPLE_BAND;
              if (off > -3 && off < 3) {
                // A bright crest with a faint trough behind it, like a wave on water.
                const lift = strength * (Math.exp(-off * off) - 0.35 * Math.exp(-(off + 1.6) * (off + 1.6)));
                v += lift * 0.5;
                if (lift > 0.25) rippled = true;
              }
            }
          }
          if (v <= 0) continue;
          const i = y * cols + x;
          if (rippled && Math.random() < 0.3) grid[i] = Math.floor(Math.random() * CHARS.length);
          // The drifting field alone tops out around the middle levels; the brightest are left for ripple crests.
          const level = Math.min(LEVELS - 1, Math.floor(v * LEVELS));
          buckets[level].push(i);
        }
      }
      for (let l = 0; l < LEVELS; l++) {
        const cells = buckets[l];
        if (!cells.length) continue;
        ctx.fillStyle = `rgb(${rgb} / ${(0.035 + (l / (LEVELS - 1)) * 0.15).toFixed(3)})`;
        for (const i of cells) ctx.fillText(CHARS[grid[i]], (i % cols) * CELL_W, Math.floor(i / cols) * CELL_H);
      }
    };

    resize();
    const observer = new ResizeObserver(() => { resize(); draw(performance.now()); });
    observer.observe(canvas);

    let raf = 0, last = 0;
    const loop = (now: number) => {
      raf = requestAnimationFrame(loop);
      if (now - last < (ripples.length ? RIPPLE_FRAME_MS : FRAME_MS)) return;
      last = now;
      draw(now);
    };
    if (still) draw(0);
    else raf = requestAnimationFrame(loop);

    // The canvas sits under the chat and takes no clicks, so listen on the chat around it.
    const host = canvas.parentElement!;
    const onDown = (e: PointerEvent) => {
      if (still || e.button !== 0) return;
      if ((e.target as Element | null)?.closest(INTERACTIVE)) return;
      const box = canvas.getBoundingClientRect();
      ripples.push({ x: e.clientX - box.left, y: e.clientY - box.top, start: performance.now() });
      if (ripples.length > 6) ripples.shift();
    };
    host.addEventListener('pointerdown', onDown);

    // Theme changes recolor the letters on the next frame.
    const themeObserver = new MutationObserver(readColor);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });

    return () => {
      cancelAnimationFrame(raf);
      host.removeEventListener('pointerdown', onDown);
      observer.disconnect();
      themeObserver.disconnect();
    };
  }, []);

  return <canvas ref={ref} className="ascii-bg" aria-hidden />;
}
