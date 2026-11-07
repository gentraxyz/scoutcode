// SoundCloud's embedded player, driven through its Widget API (https://developers.soundcloud.com/docs/api/html5-widget).
// One iframe lives for the whole session so music keeps playing while the music card is closed; the card places it in
// its player slot while open (see FloatingMusic), since SoundCloud's player is what credits the uploader and SoundCloud.

type Sound = { title?: string; permalink_url?: string; artwork_url?: string | null; duration?: number; user?: { username?: string } };

type Widget = {
  bind(event: string, fn: (e?: { currentPosition?: number }) => void): void;
  load(url: string, opts: Record<string, unknown>): void;
  play(): void;
  pause(): void;
  toggle(): void;
  seekTo(ms: number): void;
  setVolume(v: number): void;
  next(): void;
  prev(): void;
  getDuration(cb: (ms: number) => void): void;
  getCurrentSound(cb: (s: Sound | null) => void): void;
  getCurrentSoundIndex(cb: (i: number) => void): void;
  getSounds(cb: (s: Sound[]) => void): void;
  isPaused(cb: (p: boolean) => void): void;
};

type WidgetApi = ((el: HTMLIFrameElement) => Widget) & { Events: Record<string, string> };

declare global {
  interface Window {
    SC?: { Widget: WidgetApi };
  }
}

export type NowSound = { title: string; artist: string; artwork: string | null; url: string | null; duration: number };

export type Handlers = {
  play(): void;
  pause(): void;
  finish(): void;
  progress(seconds: number): void;
  sound(s: NowSound): void;
  error(): void;
};

const OPTIONS = {
  auto_play: true,
  visual: false,
  show_artwork: false,
  show_comments: false,
  show_user: true,
  show_reposts: false,
  show_teaser: false,
  hide_related: true,
  buying: false,
  sharing: false,
  download: false,
  color: '#ff5500',
};

/** Height of SoundCloud's single-line mini player. */
export const FRAME_HEIGHT = 20;

let apiLoad: Promise<WidgetApi> | null = null;
let frame: HTMLIFrameElement | null = null;
let widget: Widget | null = null;
let volume = 80;

function loadApi(): Promise<WidgetApi> {
  apiLoad ??= new Promise<WidgetApi>((resolve, reject) => {
    if (window.SC?.Widget) return resolve(window.SC.Widget);
    const s = document.createElement('script');
    s.src = 'https://w.soundcloud.com/player/api.js';
    s.async = true;
    s.onload = () => (window.SC?.Widget ? resolve(window.SC.Widget) : reject(new Error('SoundCloud player failed to load.')));
    s.onerror = () => reject(new Error('Could not reach SoundCloud. Check your connection.'));
    document.head.appendChild(s);
  }).catch((e) => {
    apiLoad = null;
    throw e;
  });
  return apiLoad;
}

function playerSrc(embed: string): string {
  const q = new URLSearchParams({ url: embed });
  for (const [k, v] of Object.entries(OPTIONS)) q.set(k, String(v));
  return `https://w.soundcloud.com/player/?${q}`;
}

function reportSound(w: Widget, h: Handlers) {
  w.getCurrentSound((s) => {
    if (!s) return;
    h.sound({
      title: s.title ?? '',
      artist: s.user?.username ?? '',
      artwork: s.artwork_url ?? null,
      url: s.permalink_url ?? null,
      duration: (s.duration ?? 0) / 1000,
    });
  });
}

/** Starts an entry (a song or playlist source from music.rs) in the player, creating it the first time. */
export async function scPlay(embed: string, h: Handlers): Promise<void> {
  const api = await loadApi();
  if (widget) {
    widget.load(embed, { ...OPTIONS, callback: () => widget?.setVolume(volume) });
    return;
  }
  const f = document.createElement('iframe');
  f.className = 'sc-frame away';
  f.title = 'SoundCloud player';
  f.allow = 'autoplay; encrypted-media';
  f.src = playerSrc(embed);
  f.height = String(FRAME_HEIGHT);
  document.body.appendChild(f);
  frame = f;
  const w = api(f);
  const E = api.Events;
  w.bind(E.READY, () => w.setVolume(volume));
  w.bind(E.PLAY, () => {
    h.play();
    reportSound(w, h);
  });
  w.bind(E.PAUSE, () => h.pause());
  w.bind(E.FINISH, () => h.finish());
  w.bind(E.PLAY_PROGRESS, (e) => h.progress((e?.currentPosition ?? 0) / 1000));
  w.bind(E.ERROR, () => h.error());
  widget = w;
}

export const scToggle = () => widget?.toggle();
export const scPause = () => widget?.pause();
export const scResume = () => widget?.play();
export const scSeek = (seconds: number) => widget?.seekTo(Math.max(0, seconds) * 1000);

export function scVolume(v: number) {
  volume = Math.round(Math.max(0, Math.min(1, v)) * 100);
  widget?.setVolume(volume);
}

/** Within a playlist: moves by `delta` and returns true, or returns false at its ends (or for a single song). */
export function scStep(delta: 1 | -1): Promise<boolean> {
  const w = widget;
  if (!w) return Promise.resolve(false);
  return new Promise((resolve) => {
    w.getSounds((sounds) => {
      w.getCurrentSoundIndex((i) => {
        const to = i + delta;
        if (sounds.length <= 1 || to < 0 || to >= sounds.length) return resolve(false);
        if (delta === 1) w.next();
        else w.prev();
        resolve(true);
      });
    });
  });
}

/** Puts the player over `slot` (the card's player area), or out of sight when the card is closed. */
export function scPlace(slot: HTMLElement | null) {
  const f = frame;
  if (!f) return;
  const r = slot?.getBoundingClientRect();
  if (r && r.width > 0) {
    f.style.left = `${r.left}px`;
    f.style.top = `${r.top}px`;
    f.style.width = `${r.width}px`;
    f.classList.remove('away');
  } else {
    f.classList.add('away');
  }
}

export const scActive = () => frame !== null;
