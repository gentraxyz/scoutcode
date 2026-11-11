// The welcome screen's backdrop, drawn in letters: Saturn turning in a starry sky with its rings, and a wheel station
// drifting far off in front. The planet's bands and storms slide round with its spin, the rings shimmer, stars twinkle,
// the station's ring turns and now and then the whole station turns round, its beacons blink, and a slow pulse climbs
// from its mast to the planet. Both are shaded as lit solids under the same sun.
// Like AsciiBackdrop it draws on a canvas at ~20 fps and holds still for reduced motion.
import { useEffect, useRef } from 'react';
import { themeColor } from './theme';

const CELL_W = 7.2;
const CELL_H = 12;
const FONT = '600 10.5px "Cascadia Mono", "JetBrains Mono", Consolas, monospace';
const FRAME_MS = 50;
/** Distinct alpha steps, so letters can be batched by fill style. */
const LEVELS = 16;
const alphaOf = (level: number) => 0.04 + (level / (LEVELS - 1)) * 0.58;
/** A brightness from 0 to 1 as an alpha level, the same for the planet and the station. */
const levelOf = (v: number) => Math.min(LEVELS - 1, Math.floor((0.12 + Math.min(1, v) * 0.88) * LEVELS));

/** Dark to bright. */
const PLANET_RAMP = '.:-=+*#%@';
const RING_RAMP = '.:-=#';
const rampCh = (v: number) => PLANET_RAMP.charCodeAt(Math.min(PLANET_RAMP.length - 1, Math.max(0, Math.floor(v * PLANET_RAMP.length))));

/** The ring plane's roll on screen (right end lower) and how far it is opened toward the viewer. */
const ROLL = 0.3;
const OPEN = 0.42;
const COS_R = Math.cos(ROLL), SIN_R = Math.sin(ROLL);
const COS_B = Math.cos(OPEN), SIN_B = Math.sin(OPEN);
/** Toward the sun, in screen space (x right, y down, z at the viewer): upper left, a little in front. */
const LIGHT = (() => {
  const v = [-0.62, -0.3, 0.72];
  const n = Math.hypot(v[0], v[1], v[2]);
  return [v[0] / n, v[1] / n, v[2] / n];
})();
/** The same, in the planet's rolled frame. */
const LIGHT_U = LIGHT[0] * COS_R + LIGHT[1] * SIN_R;
const LIGHT_V = -LIGHT[0] * SIN_R + LIGHT[1] * COS_R;
/** Seconds per radian: one turn of the planet takes about 24 s. */
const SPIN = 0.26;
const STORMS = [
  { lon: 1.0, lat: -0.34, r: 0.24 },
  { lon: 3.6, lat: 0.3, r: 0.17 },
  { lon: 5.2, lat: -0.08, r: 0.13 },
];

const hash = (x: number, y: number) => {
  const s = Math.sin(x * 127.1 + y * 311.7) * 43758.5453;
  return s - Math.floor(s);
};
const wrap = (a: number) => a - Math.PI * 2 * Math.round(a / (Math.PI * 2));
const rgbOf = (hex: string, fallback: string) => {
  const n = parseInt(hex.replace('#', '').slice(0, 6), 16);
  return Number.isNaN(n) || !hex ? fallback : `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
};

/** How dense the rings are at `rho` planet radii from the center: C ring, B ring, the Cassini gap, A ring. */
function ringDensity(rho: number) {
  if (rho < 1.22 || rho > 2.28) return 0;
  if (rho < 1.5) return 0.3;
  if (rho < 1.93) return 0.82 + 0.16 * Math.sin(rho * 40);
  if (rho < 2.04) return 0;
  return 0.6;
}

// ---------- The station ----------
// A wheel station seen from far off, modelled in 3-D and shaded like the planet: lit by the same sun, with soft shadows
// it casts on itself. A segmented ring on three spokes turns about a central hub; a mast above the hub carries a bar
// of solar panels that tip to follow the sun, and a capsule is docked below. Every so often the whole station turns
// slowly round. Its units are the ring's radius, with the hub's axis along y.

const TAU = Math.PI * 2;
/** Row-major 3x3 rotations. */
type M3 = number[];
const mul = (a: M3, b: M3): M3 => {
  const o: M3 = [];
  for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) o.push(a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c]);
  return o;
};
const rotX = (a: number): M3 => { const c = Math.cos(a), s = Math.sin(a); return [1, 0, 0, 0, c, -s, 0, s, c]; };
const rotY = (a: number): M3 => { const c = Math.cos(a), s = Math.sin(a); return [c, 0, s, 0, 1, 0, -s, 0, c]; };
const rotZ = (a: number): M3 => { const c = Math.cos(a), s = Math.sin(a); return [c, -s, 0, s, c, 0, 0, 0, 1]; };
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
const fract = (v: number) => v - Math.floor(v);
/** Distance to a capped cylinder along y, from radial distance `r` and height `y` off its middle. */
const cyl = (r: number, y: number, rad: number, half: number) => {
  const dx = r - rad, dy = Math.abs(y) - half;
  return Math.min(Math.max(dx, dy), 0) + Math.hypot(Math.max(dx, 0), Math.max(dy, 0));
};
/** Everything that fits inside this many ring radii of the hub. */
const BOUND = 1.32;

/** The ring's spin and the panels' tip, set each frame. */
let spinC = 1, spinS = 0, panC = 1, panS = 0;
/** Which part the last `sdf` call was nearest: 0 ring, 1 spoke, 2 hub, 3 panel, 4 mast, 5 dock. */
let hitId = 0;
/** How far a point is from the station's surface. */
function sdf(x: number, y: number, z: number) {
  // The ring, its spokes and the hub's collar turn with the spin.
  const rx = x * spinC - z * spinS, rz = x * spinS + z * spinC;
  const rr = Math.hypot(rx, rz), ang = Math.atan2(rz, rx);
  const seg = (ang / TAU) * 12, joint = Math.max(0, 1 - Math.abs(seg - Math.round(seg)) / 0.07);
  let d = Math.hypot(rr - 1, y) - (0.12 - 0.035 * joint), id = 0;
  const sec = TAU / 3, a = ang - sec * Math.round(ang / sec);
  const px = rr * Math.cos(a), pz = rr * Math.sin(a);
  let k = Math.hypot(px - clamp(px, 0.16, 0.9), y, pz) - 0.032;
  if (k < d) { d = k; id = 1; }
  k = Math.hypot(rr - 0.25, y) - 0.06;
  if (k < d) { d = k; id = 2; }
  // The rest holds still.
  const r = Math.hypot(x, z);
  k = cyl(r, y, 0.15, 0.36);
  if (k < d) { d = k; id = 2; }
  k = Math.min(cyl(r, y - 0.62, 0.028, 0.28), Math.hypot(x - clamp(x, -0.96, 0.96), y - 0.86, z) - 0.022);
  if (k < d) { d = k; id = 4; }
  const ly = y - 0.86, qy = ly * panC + z * panS, qz = -ly * panS + z * panC;
  const bx = Math.abs(Math.abs(x) - 0.6) - 0.33, by = Math.abs(qy) - 0.012, bz = Math.abs(qz) - 0.22;
  k = Math.hypot(Math.max(bx, 0), Math.max(by, 0), Math.max(bz, 0)) + Math.min(Math.max(bx, by, bz), 0);
  if (k < d) { d = k; id = 3; }
  k = Math.min(cyl(r, y + 0.5, 0.09, 0.15), Math.hypot(x, y - clamp(y, -0.88, -0.7), z) - 0.085);
  if (k < d) { d = k; id = 5; }
  hitId = id;
  return d;
}

/** How shiny the last `material` call's surface is, and whether it is a window. */
let shine = 0, isWindow = false;
/** The albedo of part `id` at a point on it. */
function material(x: number, y: number, z: number, id: number) {
  shine = 0.15;
  isWindow = false;
  if (id === 0) {
    const rx = x * spinC - z * spinS, rz = x * spinS + z * spinC;
    const rr = Math.hypot(rx, rz), ang = Math.atan2(rz, rx);
    const seg = (ang / TAU) * 12, joint = Math.max(0, 1 - Math.abs(seg - Math.round(seg)) / 0.09);
    isWindow = rr > 1.07 && Math.abs(y) < 0.05 && fract((ang / TAU) * 48) < 0.45 && joint === 0;
    return 0.82 - 0.35 * joint;
  }
  if (id === 2) {
    const band = fract(y * 9) < 0.2;
    isWindow = Math.hypot(x, z) > 0.14 && Math.abs(y) < 0.3 && !band && fract((Math.atan2(z, x) / TAU) * 10) < 0.3;
    return band ? 0.5 : 0.78;
  }
  if (id === 3) {
    // Cells of a panel, split by a frame; the glass is dark but glints in the sun.
    const ly = y - 0.86, qz = -ly * panS + z * panC;
    const ux = Math.abs(x) - 0.6 + 0.33, uz = qz + 0.22;
    shine = 1.1;
    return fract(ux * 9) < 0.14 || fract(uz * 7) < 0.16 ? 0.62 : 0.3;
  }
  if (id === 5) { shine = 0.4; return 0.86; }
  return 0.58;
}

export function SaturnScene() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current!;
    const ctx = canvas.getContext('2d')!;
    const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let cols = 0, rows = 0, dpr = 1;
    let rgb = '237 237 237', accent = '232 232 232';
    /** 0 for no star, else its twinkle phase + 1. */
    let star = new Float32Array(0);
    // The station, drawn afresh each frame since it drifts: cells it covers, and the letter (0 for none) and alpha
    // level in each.
    let occ = new Uint8Array(0), occCh = new Uint8Array(0), occLevel = new Uint8Array(0);
    let covered: number[] = [];
    let planet = { x: 0, y: 0, r: 0 };
    let place = { x: 0, y: 0, S: 1 };

    const readColors = () => {
      rgb = rgbOf(themeColor('strong'), rgb);
      accent = rgbOf(themeColor('primary'), accent);
    };

    const resize = () => {
      dpr = window.devicePixelRatio || 1;
      const { width, height } = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      cols = Math.ceil(width / CELL_W);
      rows = Math.ceil(height / CELL_H);
      planet = { x: width * 0.73, y: height * 0.27, r: Math.max(70, Math.min(190, Math.min(width, height) * 0.19)) };
      // The station lower left of the words, small and far off: `S` is its ring's radius in pixels.
      const S = Math.max(52, Math.min(110, Math.min(width, height) * 0.135));
      place = { S, x: Math.max(BOUND * S + 12, width * 0.21), y: Math.min(height - BOUND * S - 12, height * 0.69) };
      readColors();
      const n = cols * rows;
      star = new Float32Array(n);
      occ = new Uint8Array(n);
      occCh = new Uint8Array(n);
      occLevel = new Uint8Array(n);
      covered = [];
      for (let i = 0; i < n; i++) if (hash(i % cols, Math.floor(i / cols) + 91) < 0.016) star[i] = 1 + hash(i, 3) * 6.28;
    };

    const glow: number[] = [];
    let feed = { x: -1, y: -1 };

    /** Renders the station for time `s`: fills `occ` and pushes its lit windows and beacons onto `glow`. */
    const station = (s: number) => {
      for (const i of covered) occ[i] = 0;
      covered.length = 0;
      const S = place.S;
      // Drifting a little round a slow loop.
      const cx = place.x + Math.sin(s * 0.05) * S * 0.08, cy = place.y + Math.sin(s * 0.07 + 1) * S * 0.05;
      // Seen three-quarters on, rocking gently; once every 48 s it turns right round over about 10 s.
      const f = fract(s / 48);
      const turn = f < 0.78 ? 0 : TAU * (1 - Math.cos(((f - 0.78) / 0.22) * Math.PI)) / 2;
      const A = mul(rotY(turn + 0.5), mul(rotZ(-0.35), mul(rotX(0.95 + 0.06 * Math.sin(s * 0.09)), rotY(0.4 * Math.sin(s * 0.06)))));
      const spin = s * 0.33, tip = 0.5 + 0.35 * Math.sin(s * 0.1);
      spinC = Math.cos(spin); spinS = Math.sin(spin); panC = Math.cos(tip); panS = Math.sin(tip);
      /** World (x right, y up, z at the viewer) to the station's frame: A transposed. */
      const toObj = (wx: number, wy: number, wz: number, o: number[]) => {
        o[0] = A[0] * wx + A[3] * wy + A[6] * wz;
        o[1] = A[1] * wx + A[4] * wy + A[7] * wz;
        o[2] = A[2] * wx + A[5] * wy + A[8] * wz;
      };
      const dir = [0, 0, 0], sun = [0, 0, 0], o = [0, 0, 0];
      toObj(0, 0, -1, dir);
      toObj(LIGHT[0], -LIGHT[1], LIGHT[2], sun);
      // Halfway between the sun and the viewer, for the glints.
      const hx = sun[0] - dir[0], hy = sun[1] - dir[1], hz = sun[2] - dir[2], hn = Math.hypot(hx, hy, hz);

      /** The brightness one ray sees, or -1 if it misses; sets `lamp` when it lands on a window in the dark. */
      let lamp = false;
      const ray = (X: number, Y: number) => {
        const b = X * X + Y * Y;
        if (b >= BOUND * BOUND) return -1;
        const ze = Math.sqrt(BOUND * BOUND - b);
        toObj(X, Y, ze, o);
        let t = 0, hit = false;
        for (let k = 0; k < 64 && t < ze * 2; k++) {
          const d = sdf(o[0] + dir[0] * t, o[1] + dir[1] * t, o[2] + dir[2] * t);
          if (d < 0.002) { hit = true; break; }
          t += d * 0.9;
        }
        if (!hit) return -1;
        const x = o[0] + dir[0] * t, y = o[1] + dir[1] * t, z = o[2] + dir[2] * t;
        const id = hitId, e = 0.004;
        let nx = sdf(x + e, y, z) - sdf(x - e, y, z), ny = sdf(x, y + e, z) - sdf(x, y - e, z), nz = sdf(x, y, z + e) - sdf(x, y, z - e);
        const nn = Math.hypot(nx, ny, nz) || 1;
        nx /= nn; ny /= nn; nz /= nn;
        const albedo = material(x, y, z, id);
        const diffuse = Math.max(0, nx * sun[0] + ny * sun[1] + nz * sun[2]);
        // Soft shadow: march toward the sun and see how close the ray passes to the rest of the station.
        let shade = 1;
        if (diffuse > 0) {
          let u = 0.03;
          for (let k = 0; k < 24 && u < 2.6; k++) {
            const d = sdf(x + nx * 0.01 + sun[0] * u, y + ny * 0.01 + sun[1] * u, z + nz * 0.01 + sun[2] * u);
            if (d < 0.001) { shade = 0; break; }
            shade = Math.min(shade, (8 * d) / u);
            u += Math.max(0.02, d);
          }
        }
        const light = diffuse * shade;
        const glint = Math.pow(Math.max(0, (nx * hx + ny * hy + nz * hz) / hn), 28) * shine * shade;
        if (isWindow && light < 0.3) lamp = true;
        return albedo * (0.1 + 0.9 * light) + glint;
      };

      // Two rays a cell, one high and one low since the cells are tall; a cell half covered comes out dimmer.
      const reach = BOUND * S;
      const c0 = Math.max(0, Math.floor((cx - reach) / CELL_W)), c1 = Math.min(cols - 1, Math.ceil((cx + reach) / CELL_W));
      const r0 = Math.max(0, Math.floor((cy - reach) / CELL_H)), r1 = Math.min(rows - 1, Math.ceil((cy + reach) / CELL_H));
      for (let r = r0; r <= r1; r++) {
        for (let c = c0; c <= c1; c++) {
          const X = ((c + 0.5) * CELL_W - cx) / S;
          lamp = false;
          let sum = 0, hits = 0;
          for (const dy of [0.25, 0.75]) {
            const v = ray(X, -((r + dy) * CELL_H - cy) / S);
            if (v >= 0) { sum += v; hits++; }
          }
          if (!hits) continue;
          const i = r * cols + c, v = Math.min(1, (sum / 2) * 0.92);
          covered.push(i);
          occ[i] = 1;
          occCh[i] = v < 0.03 ? 0 : rampCh(v);
          occLevel[i] = levelOf(v);
          if (lamp) glow.push(i, 46, 0.5 + 0.1 * Math.sin(s * 1.7 + i));
        }
      }

      /** Where a point of the station lands on screen, as a cell, or -1 off screen. */
      const cellOf = (px: number, py: number, pz: number) => {
        const X = A[0] * px + A[1] * py + A[2] * pz, Y = A[3] * px + A[4] * py + A[5] * pz;
        const c = Math.floor((cx + X * S) / CELL_W), r = Math.floor((cy - Y * S) / CELL_H);
        return c < 0 || c >= cols || r < 0 || r >= rows ? -1 : r * cols + c;
      };
      // A beacon on the mast and one on the capsule, blinking out of step.
      const top = cellOf(0, 0.95, 0), low = cellOf(0, -0.98, 0);
      if (top >= 0 && (fract(s * 0.5) < 0.22 || still)) glow.push(top, 42, 0.95);
      if (low >= 0 && (fract(s * 0.5 + 0.5) < 0.22 || still)) glow.push(low, 42, 0.8);
      feed = top >= 0 ? { x: top % cols, y: Math.floor(top / cols) } : { x: -1, y: -1 };
    };

    // What Saturn shows at a point `dx, dy` pixels from its center at `s` seconds: sets `ch` and returns its
    // brightness from 0 to 1, or -1 where there is only sky.
    let ch = 0;
    const saturn = (dx: number, dy: number, s: number): number => {
      const r = planet.r;
      const u = dx * COS_R + dy * SIN_R, v = -dx * SIN_R + dy * COS_R;
      // The point of the ring plane behind this pixel is (u, b) across and toward the viewer.
      const b = v / SIN_B;
      const rho = Math.hypot(u, b) / r;
      const nu = u / r, nv = v / (0.93 * r);
      const q = nu * nu + nv * nv;
      const density = ringDensity(rho);
      // The near half of the rings crosses in front of the planet; the far half goes behind it.
      if (density > 0 && (b > 0 || q >= 1)) {
        const py = b * SIN_B, pz = b * COS_B;
        const along = u * LIGHT_U + py * LIGHT_V + pz * LIGHT[2];
        const shadowed = along < 0 && u * u + py * py + pz * pz - along * along < r * r;
        const clump = 0.5 + 0.5 * Math.sin((Math.atan2(b, u) - s * 0.3) * 24 + rho * 37);
        const val = density * (0.66 + 0.34 * clump) * (shadowed ? 0.14 : 1);
        ch = RING_RAMP.charCodeAt(Math.min(RING_RAMP.length - 1, Math.floor(val * RING_RAMP.length)));
        return val * 0.8;
      }
      if (q >= 1) return -1;
      const nz = Math.sqrt(1 - q);
      const lat = Math.asin(Math.max(-1, Math.min(1, -nv * COS_B + nz * SIN_B)));
      const lon = Math.atan2(nu, nv * SIN_B + nz * COS_B) + s * SPIN;
      // Bands by latitude, bent a little by longitude, with waves and storms that show the spin.
      let tex = 0.5 + 0.26 * Math.sin(lat * 13 + 0.6 * Math.sin(lon * 3)) + 0.12 * Math.sin(lat * 29 + 1.3)
        + 0.16 * Math.sin(lon * 5 + lat * 8) * Math.cos(lat);
      for (const st of STORMS) {
        const dl = wrap(lon - st.lon) * Math.cos(lat), dt = lat - st.lat;
        const d = (dl * dl + dt * dt) / (st.r * st.r);
        if (d < 1) tex += 0.5 * (1 - d);
      }
      tex = Math.max(0, Math.min(1, tex));
      const diffuse = Math.max(0, (nu * COS_R - nv * SIN_R) * LIGHT[0] + (nu * SIN_R + nv * COS_R) * LIGHT[1] + nz * LIGHT[2]);
      const val = (0.05 + 0.95 * diffuse) * (0.42 + 0.58 * tex);
      ch = rampCh(val);
      return val;
    };

    const buckets: number[][] = Array.from({ length: LEVELS }, () => []);
    const draw = (t: number) => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, cols * CELL_W, rows * CELL_H);
      ctx.font = FONT;
      ctx.textBaseline = 'top';
      const s = t / 1000;
      for (const b of buckets) b.length = 0;
      glow.length = 0;
      station(s);
      const reach = planet.r * 2.35;
      for (let y = 0; y < rows; y++) {
        const dy = y * CELL_H + CELL_H / 2 - planet.y;
        for (let x = 0; x < cols; x++) {
          const i = y * cols + x;
          if (occ[i]) { if (occCh[i]) buckets[occLevel[i]].push(i, occCh[i]); continue; }
          const dx = x * CELL_W + CELL_W / 2 - planet.x;
          if (Math.abs(dx) < reach && Math.abs(dy) < reach) {
            const v = saturn(dx, dy, s);
            if (v >= 0) { buckets[levelOf(v)].push(i, ch); continue; }
          }
          if (star[i]) {
            const tw = 0.5 + 0.5 * Math.sin(s * 1.3 + star[i] * 3);
            buckets[Math.floor((0.12 + tw * 0.5) * LEVELS)].push(i, tw > 0.82 ? 43 : 46);
          }
        }
      }
      // The dish's signal: short runs of dots climbing from its feed to the planet.
      const fx = feed.x * CELL_W + CELL_W / 2, fy = feed.y * CELL_H + CELL_H / 2;
      const steps = Math.floor(Math.hypot(planet.x - fx, planet.y - fy) / 15);
      if (feed.y >= 0 && feed.y < rows && feed.x >= 0 && feed.x < cols) glow.push(feed.y * cols + feed.x, 111, 0.9);
      for (let k = 2; k < steps; k++) {
        const px = fx + ((planet.x - fx) * k) / steps, py = fy + ((planet.y - fy) * k) / steps;
        if (Math.hypot(px - planet.x, py - planet.y) < planet.r * 1.1) break;
        const wave = Math.sin(k * 0.5 - s * 3.2);
        if (wave < 0.5) continue;
        const cx = Math.floor(px / CELL_W), cy = Math.floor(py / CELL_H);
        if (cx < 0 || cx >= cols || cy < 0 || cy >= rows || occ[cy * cols + cx]) continue;
        glow.push(cy * cols + cx, wave > 0.9 ? 42 : 46, (wave - 0.5) * 1.5);
      }
      for (let l = 0; l < LEVELS; l++) {
        const cells = buckets[l];
        if (!cells.length) continue;
        ctx.fillStyle = `rgb(${rgb} / ${alphaOf(l).toFixed(3)})`;
        for (let k = 0; k < cells.length; k += 2) {
          const i = cells[k];
          ctx.fillText(String.fromCharCode(cells[k + 1]), (i % cols) * CELL_W, Math.floor(i / cols) * CELL_H);
        }
      }
      for (let k = 0; k < glow.length; k += 3) {
        const i = glow[k], gx = (i % cols) * CELL_W, gy = Math.floor(i / cols) * CELL_H;
        ctx.clearRect(gx, gy, CELL_W, CELL_H);
        ctx.fillStyle = `rgb(${accent} / ${Math.min(1, glow[k + 2]).toFixed(3)})`;
        ctx.fillText(String.fromCharCode(glow[k + 1]), gx, gy);
      }
    };

    resize();
    const observer = new ResizeObserver(() => { resize(); draw(still ? 0 : performance.now()); });
    observer.observe(canvas);

    let raf = 0, last = 0;
    const loop = (now: number) => {
      raf = requestAnimationFrame(loop);
      if (now - last < FRAME_MS) return;
      last = now;
      draw(now);
    };
    if (still) draw(0);
    else raf = requestAnimationFrame(loop);

    // Theme changes recolor the letters on the next frame.
    const themeObserver = new MutationObserver(() => { readColors(); if (still) draw(0); });
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });

    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
      themeObserver.disconnect();
    };
  }, []);

  return <canvas ref={ref} className="saturn-scene" aria-hidden />;
}
