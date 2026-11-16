// What the canvas games share: saved bests, little synth sounds, the theme's colours and a crisp canvas.
import { useState } from 'react';
import { IconButton } from './widgets';

export function loadNum(key: string) {
  try { return Number(localStorage.getItem(key)) || 0; } catch { return 0; }
}

export function saveNum(key: string, n: number) {
  try { localStorage.setItem(key, String(n)); } catch {}
}

// ---------- sound ----------

const SOUND_KEY = 'scout.games.sound';
let soundOn = (() => { try { return localStorage.getItem(SOUND_KEY) !== '0'; } catch { return true; } })();
let audio: AudioContext | null = null;

export const isSoundOn = () => soundOn;

export function setSound(on: boolean) {
  soundOn = on;
  try { localStorage.setItem(SOUND_KEY, on ? '1' : '0'); } catch {}
}

function ac() {
  if (!soundOn) return null;
  try {
    audio ??= new AudioContext();
    if (audio.state === 'suspended') void audio.resume();
    return audio;
  } catch {
    return null;
  }
}

/** A short tone sliding from `f0` to `f1` Hz, starting `delay` seconds from now. */
export function tone(f0: number, f1: number, dur: number, type: OscillatorType = 'square', vol = 0.04, delay = 0) {
  const a = ac();
  if (!a) return;
  const t = a.currentTime + delay;
  const o = a.createOscillator();
  const g = a.createGain();
  o.type = type;
  o.frequency.setValueAtTime(f0, t);
  o.frequency.exponentialRampToValueAtTime(Math.max(20, f1), t + dur);
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(vol, t + 0.008);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  o.connect(g).connect(a.destination);
  o.start(t);
  o.stop(t + dur + 0.03);
}

/** A burst of filtered noise: thuds and whooshes. */
export function noise(dur: number, vol = 0.08, freq = 900) {
  const a = ac();
  if (!a) return;
  const t = a.currentTime;
  const buf = a.createBuffer(1, Math.ceil(a.sampleRate * dur), a.sampleRate);
  const d = buf.getChannelData(0);
  for (let i = 0; i < d.length; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / d.length);
  const src = a.createBufferSource();
  const f = a.createBiquadFilter();
  const g = a.createGain();
  src.buffer = buf;
  f.type = 'lowpass';
  f.frequency.value = freq;
  g.gain.value = vol;
  src.connect(f).connect(g).connect(a.destination);
  src.start(t);
}

// ---------- drawing ----------

/** The theme's colours, read off the element so the games follow light and dark. */
export function themeColors(el: Element) {
  const css = getComputedStyle(el);
  const c = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    bg: c('--bg', '#151515'), sidebar: c('--sidebar', '#111111'), surface: c('--surface', '#1d1d1d'), surfaceHi: c('--surface-hi', '#262626'),
    raised: c('--raised', '#2f2f2f'), edge: c('--edge', '#3d3d3d'), text: c('--strong', '#ededed'), dim: c('--dim', '#7a7a7a'), faint: c('--faint', '#4d4d4d'),
    accent: c('--accent', '#f5a524'), accentHi: c('--accent-hi', '#ffb845'), ink: c('--accent-ink', '#1a1203'),
    blue: c('--usage', '#5b9cf5'), green: c('--green', '#4fbf7f'), red: c('--red', '#e86464'), code: c('--code', '#e5c07b'),
  };
}
export type Colors = ReturnType<typeof themeColors>;

/** Sizes the canvas for the screen's pixel density and keeps it sized. Returns the cleanup. */
export function crisp(el: HTMLCanvasElement, ctx: CanvasRenderingContext2D, w: number, h: number) {
  const fit = () => {
    const dpr = Math.min(window.devicePixelRatio || 1, 3);
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  };
  fit();
  window.addEventListener('resize', fit);
  return () => window.removeEventListener('resize', fit);
}

/** Mixes two #rrggbb colours; anything else comes back as `a`. */
export function mix(a: string, b: string, t: number) {
  const pa = /^#([0-9a-f]{6})$/i.exec(a);
  const pb = /^#([0-9a-f]{6})$/i.exec(b);
  if (!pa || !pb) return a;
  const na = parseInt(pa[1], 16);
  const nb = parseInt(pb[1], 16);
  const ch = (s: number) => Math.round(((na >> s) & 255) * (1 - t) + ((nb >> s) & 255) * t);
  return `rgb(${ch(16)} ${ch(8)} ${ch(0)})`;
}

export const FONT = '"Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif';

/** A fixed-step loop: steady physics whatever the frame rate. Long gaps (a hidden window) are dropped, not replayed. */
export function loop(step: (dt: number) => void, draw: (now: number) => void) {
  const DT = 1 / 120;
  let acc = 0;
  let last = performance.now();
  let raf = 0;
  const frame = (now: number) => {
    acc += Math.min((now - last) / 1000, 0.1);
    last = now;
    while (acc >= DT) {
      step(DT);
      acc -= DT;
    }
    draw(now);
    raf = requestAnimationFrame(frame);
  };
  raf = requestAnimationFrame(frame);
  return () => cancelAnimationFrame(raf);
}

/** Turns the games' sounds on and off, for every game at once. */
export function SoundToggle() {
  const [on, setOn] = useState(isSoundOn);
  return <IconButton icon={on ? 'volume' : 'volumeOff'} tip={on ? 'Mute' : 'Sound on'} size={26} onClick={() => { setSound(!on); setOn(!on); }} />;
}
