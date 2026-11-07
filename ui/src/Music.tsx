// Built-in music player for uploaded MP3s and SoundCloud, in a draggable card over the window.
// Closing the card keeps music playing in the background; there is no auto-close when Scout finishes.
// Uploaded files play in an <audio> element from the app's `music:` scheme (see music::serve in music.rs), which serves
// byte ranges, so long files start at once. SoundCloud entries play in SoundCloud's own embedded player (soundcloud.ts).
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ClipboardEvent, type FormEvent, type PointerEvent, type ReactNode } from 'react';
import { act, invoke, openExternal, type MusicTrack } from './api';
import { FRAME_HEIGHT, scActive, scPause, scPlace, scPlay, scResume, scSeek, scStep, scToggle, scVolume, type NowSound } from './soundcloud';
import { core, focusComposer, ui, useUi, useView } from './store';
import { Icon, IconButton, Spinner } from './widgets';

type Tab = 'library' | 'sc';

type PlayerState = {
  tracks: MusicTrack[];
  loaded: boolean;
  current: MusicTrack | null;
  /** The song playing inside a SoundCloud entry, which for a playlist differs from the entry itself. */
  sound: NowSound | null;
  /** What next/previous and auto-advance walk through: the library, in order. */
  queue: MusicTrack[];
  playing: boolean;
  /** Starting a track or buffering mid-track. */
  loading: boolean;
  currentTime: number;
  duration: number;
  repeat: boolean;
  muted: boolean;
  error: string | null;
  // Kept here rather than in the card so they survive closing and reopening it.
  tab: Tab;
  scInput: string;
  scBusy: boolean;
  scHint: string | null;
};

let state: PlayerState = {
  tracks: [],
  loaded: false,
  current: null,
  sound: null,
  queue: [],
  playing: false,
  loading: false,
  currentTime: 0,
  duration: 0,
  repeat: false,
  muted: false,
  error: null,
  tab: 'library',
  scInput: '',
  scBusy: false,
  scHint: null,
};

const subs = new Set<() => void>();
function setPlayer(patch: Partial<PlayerState>) {
  state = { ...state, ...patch };
  subs.forEach((f) => f());
}

function usePlayer<T>(pick: (s: PlayerState) => T): T {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => pick(state),
  );
}

export function useMusicPlaying(): boolean {
  return usePlayer((s) => s.playing);
}

// Custom schemes are served as http://<scheme>.localhost on Windows and as <scheme>://localhost elsewhere.
const MUSIC_ORIGIN = navigator.userAgent.includes('Windows') ? 'http://music.localhost' : 'music://localhost';

const isSoundCloudLink = (q: string) => /(^|\/\/|\.)soundcloud\.com\//i.test(q.trim()) || /^on\.soundcloud\.com/i.test(q.trim());
const isSc = (t: MusicTrack | null) => t?.kind === 'sc';

function savedVolume(): number {
  return Math.max(0, Math.min(1, core.get()?.view?.cfg?.music?.volume ?? 0.8));
}

// ---------- uploaded files: <audio> ----------

let audioEl: HTMLAudioElement | null = null;
let loadSeq = 0;

function getAudio(): HTMLAudioElement {
  if (audioEl) return audioEl;
  const a = new Audio();
  a.preload = 'auto';
  a.volume = savedVolume();
  const mine = () => !isSc(state.current);
  a.addEventListener('timeupdate', () => mine() && setPlayer({ currentTime: a.currentTime || 0 }));
  const syncDuration = () => {
    if (mine() && Number.isFinite(a.duration) && a.duration > 0) setPlayer({ duration: a.duration });
  };
  a.addEventListener('loadedmetadata', syncDuration);
  a.addEventListener('durationchange', syncDuration);
  a.addEventListener('play', () => mine() && setPlayer({ playing: true }));
  a.addEventListener('playing', () => mine() && setPlayer({ playing: true, loading: false }));
  a.addEventListener('waiting', () => mine() && setPlayer({ loading: true }));
  a.addEventListener('canplay', () => mine() && setPlayer({ loading: false }));
  a.addEventListener('pause', () => mine() && setPlayer({ playing: false }));
  a.addEventListener('ended', () => mine() && trackEnded());
  a.addEventListener('error', () => {
    const src = a.getAttribute('src');
    if (!src || !mine()) return; // cleared on purpose, or another player took over
    const seq = loadSeq;
    setPlayer({ playing: false, loading: false, error: 'Could not play this file.' });
    // The media element doesn't say why; ask the scheme for its error text.
    fetch(src, { headers: { Range: 'bytes=0-0' } })
      .then(async (r) => (r.ok ? null : (await r.text()).trim()))
      .then((msg) => {
        if (msg && seq === loadSeq) setPlayer({ error: msg });
      })
      .catch(() => {});
  });
  audioEl = a;
  setupMediaSession();
  return a;
}

function stopAudio() {
  const a = audioEl;
  if (!a) return;
  a.pause();
  a.removeAttribute('src');
  a.load();
}

// ---------- SoundCloud events ----------

const scHandlers = {
  play: () => isSc(state.current) && setPlayer({ playing: true, loading: false }),
  pause: () => isSc(state.current) && setPlayer({ playing: false }),
  finish: () => {
    if (!isSc(state.current)) return;
    // A playlist moves on by itself until its last song.
    void scStep(1).then((moved) => {
      if (!moved) trackEnded();
    });
  },
  progress: (s: number) => isSc(state.current) && setPlayer({ currentTime: s }),
  sound: (s: NowSound) => {
    if (!isSc(state.current)) return;
    setPlayer({ sound: s, duration: s.duration || state.duration });
    updateMediaSession(s.title || state.current!.title, s.artist || state.current!.artist, s.artwork ?? state.current!.thumbnail);
  },
  error: () => isSc(state.current) && setPlayer({ playing: false, loading: false, error: 'SoundCloud couldn’t play this. It may be private, removed or blocked in your country.' }),
};

// ---------- shared ----------

function trackEnded() {
  if (state.repeat && state.current) {
    seekTo(0);
    if (isSc(state.current)) scResume();
    else audioEl?.play().catch(() => {});
    return;
  }
  playAdjacent(1, true);
}

/** Lets the keyboard's media keys and the Windows media overlay control the player. */
function setupMediaSession() {
  const ms = navigator.mediaSession;
  if (!ms) return;
  const handlers: [MediaSessionAction, MediaSessionActionHandler][] = [
    ['play', () => togglePlayPause()],
    ['pause', () => togglePlayPause()],
    ['nexttrack', () => playAdjacent(1)],
    ['previoustrack', () => playAdjacent(-1)],
    ['seekto', (d) => d.seekTime != null && seekTo(d.seekTime)],
  ];
  for (const [action, fn] of handlers) {
    try {
      ms.setActionHandler(action, fn);
    } catch {
      // unsupported action
    }
  }
}

function updateMediaSession(title: string, artist: string, artwork: string | null) {
  const ms = navigator.mediaSession;
  if (!ms || typeof MediaMetadata === 'undefined') return;
  ms.metadata = new MediaMetadata({ title, artist, artwork: artwork ? [{ src: artwork }] : [] });
}

export async function ensureLibraryLoaded() {
  if (state.loaded) return;
  try {
    const tracks = await invoke<MusicTrack[]>('music_list');
    setPlayer({ tracks, queue: tracks, loaded: true });
  } catch (e) {
    setPlayer({ loaded: true, error: String(e) });
  }
}

/** Plays `track`, or toggles pause when it's already the current one. */
export function playTrack(track: MusicTrack) {
  if (state.current?.id === track.id && (isSc(track) ? scActive() : audioEl?.getAttribute('src'))) {
    togglePlayPause();
    return;
  }
  const seq = ++loadSeq;
  setPlayer({
    current: track,
    sound: null,
    queue: state.tracks,
    playing: false,
    loading: true,
    currentTime: 0,
    duration: track.duration ?? 0,
    error: null,
  });
  updateMediaSession(track.title, track.artist, track.thumbnail);

  if (isSc(track)) {
    stopAudio();
    if (!track.embed) {
      setPlayer({ loading: false, error: 'This SoundCloud entry has no player link. Remove it and add it again.' });
      return;
    }
    scPlay(track.embed, scHandlers).catch((e) => {
      if (seq === loadSeq) setPlayer({ loading: false, error: e instanceof Error ? e.message : String(e) });
    });
    return;
  }

  scPause();
  const a = getAudio();
  a.src = `${MUSIC_ORIGIN}/file/${encodeURIComponent(track.id)}`;
  a.play().catch((e: unknown) => {
    // A newer track replacing this one aborts its play() call; that isn't an error.
    if (seq !== loadSeq || (e instanceof DOMException && e.name === 'AbortError')) return;
    setPlayer({ loading: false, playing: false });
  });
}

export function togglePlayPause() {
  const cur = state.current;
  if (!cur) {
    if (state.tracks.length > 0) playTrack(state.tracks[0]);
    return;
  }
  if (isSc(cur)) {
    if (scActive()) scToggle();
    else playTrack(cur);
    return;
  }
  const a = getAudio();
  if (!a.getAttribute('src')) playTrack(cur);
  else if (a.paused) a.play().catch((e) => setPlayer({ error: String(e) }));
  else a.pause();
}

export function playAdjacent(delta: 1 | -1, auto = false) {
  const { current } = state;
  const queue = state.queue.length > 0 ? state.queue : state.tracks;
  // "Previous" a few seconds into a song restarts it, like most players.
  if (delta === -1 && state.currentTime > 4) {
    seekTo(0);
    return;
  }
  const move = () => {
    if (queue.length === 0) {
      setPlayer({ playing: false });
      return;
    }
    const idx = current ? queue.findIndex((t) => t.id === current.id) : -1;
    // At the end of the library, auto-advance stops instead of starting over.
    if (auto && idx === queue.length - 1) {
      setPlayer({ playing: false, currentTime: 0 });
      return;
    }
    const nextIdx = idx === -1 ? 0 : (idx + delta + queue.length) % queue.length;
    if (queue[nextIdx].id === current?.id && queue.length === 1) {
      seekTo(0);
      return;
    }
    playTrack(queue[nextIdx]);
  };
  if (isSc(current) && !auto) {
    void scStep(delta).then((moved) => !moved && move());
  } else {
    move();
  }
}

function seekTo(seconds: number) {
  if (isSc(state.current)) {
    scSeek(seconds);
  } else {
    const a = getAudio();
    if (!a.getAttribute('src')) return;
    a.currentTime = seconds;
  }
  setPlayer({ currentTime: seconds });
}

function setVolume(v: number, muted: boolean) {
  const a = getAudio();
  a.volume = v;
  a.muted = muted;
  scVolume(muted ? 0 : v);
}

function formatTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return '0:00';
  const total = Math.floor(sec);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s < 10 ? '0' : ''}${s}`;
}

export function openMusic() {
  void ensureLibraryLoaded();
  ui.set({ musicOpen: true });
}

/** Closes the floating music card. Pass `stop = true` when turning the Music feature off in Settings. */
export function closeMusic(stop = false) {
  if (stop) {
    audioEl?.pause();
    scPause();
  }
  ui.set({ musicOpen: false });
}

const CARD_W = 336;

function clamp(pos: [number, number], el: HTMLElement | null): [number, number] {
  const w = el?.offsetWidth ?? CARD_W;
  const h = el?.offsetHeight ?? 440;
  return [Math.max(4, Math.min(pos[0], window.innerWidth - w - 24)), Math.max(4, Math.min(pos[1], window.innerHeight - h - 4))];
}

function Thumb({ track, active, playing, loading }: { track: MusicTrack; active: boolean; playing: boolean; loading: boolean }) {
  const [broken, setBroken] = useState(false);
  const overlay = active ? loading ? <Spinner size={12} /> : <Icon name={playing ? 'pause' : 'play'} size={12} /> : null;
  if (track.thumbnail && !broken) {
    return (
      <span className="music-row-thumb-wrap">
        <img className="music-row-thumb" src={track.thumbnail} alt="" onError={() => setBroken(true)} />
        {overlay && <span className="music-row-thumb-overlay">{overlay}</span>}
      </span>
    );
  }
  return <span className="music-row-icon">{overlay ?? <Icon name="play" size={12} />}</span>;
}

function TrackRow({ track, action }: { track: MusicTrack; action: ReactNode }) {
  const p = usePlayer((s) => s);
  const active = p.current?.id === track.id;
  const playlist = track.embed?.includes('/playlists/');
  return (
    <div className={`music-row ${active ? 'active' : ''}`}>
      <button type="button" className="music-row-main grow min0" onClick={() => playTrack(track)}>
        <Thumb track={track} active={active} playing={p.playing} loading={p.loading} />
        <div className="stack tight grow min0">
          <span className="small strong ellipsis">{track.title}</span>
          <span className="tiny dim ellipsis">
            {track.artist}
            {track.duration ? ` · ${formatTime(track.duration)}` : ''}
          </span>
        </div>
        <span className={`music-badge ${track.kind}`}>{track.kind === 'mp3' ? 'MP3' : playlist ? 'Playlist' : 'SoundCloud'}</span>
      </button>
      {action}
    </div>
  );
}

export function FloatingMusic() {
  const { music } = useView('cfg');
  const open = useUi((u) => u.musicOpen);
  const p = usePlayer((s) => s);
  const card = useRef<HTMLDivElement>(null);
  const slot = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [drag, setDrag] = useState<{ pos: [number, number]; dx: number; dy: number } | null>(null);
  const [, setSize] = useState(0);
  const [shown, setShown] = useState(open);
  const [uploading, setUploading] = useState(false);

  useEffect(() => {
    if (open) {
      void ensureLibraryLoaded();
      return setShown(true);
    }
    const t = setTimeout(() => setShown(false), 150);
    return () => clearTimeout(t);
  }, [open]);

  useEffect(() => {
    const resized = () => setSize((n) => n + 1);
    window.addEventListener('resize', resized);
    return () => window.removeEventListener('resize', resized);
  }, []);

  useEffect(() => {
    setVolume(music.volume ?? 0.8, state.muted);
  }, [music.volume]);

  useEffect(() => {
    if (open && p.tab === 'sc') input.current?.focus();
  }, [open, p.tab]);

  // SoundCloud's player follows the card around (it lives outside it, so closing the card doesn't stop the music).
  useLayoutEffect(() => {
    scPlace(open && isSc(p.current) ? slot.current : null);
  });
  useEffect(() => () => scPlace(null), []);

  if (!music.on || !(open || shown)) return null;

  const pos = drag?.pos ?? clamp(music.pos ?? [Infinity, window.innerHeight - (card.current?.offsetHeight ?? 440) - 150], card.current);

  const start = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as Element).closest('button')) return;
    const r = card.current!.getBoundingClientRect();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ pos: [r.left, r.top], dx: e.clientX - r.left, dy: e.clientY - r.top });
  };
  const moveTo = (e: PointerEvent) => {
    if (drag) setDrag({ ...drag, pos: clamp([e.clientX - drag.dx, e.clientY - drag.dy], card.current) });
  };
  const end = () => {
    if (!drag) return;
    act('setMusic', { music: { ...music, pos: drag.pos.map(Math.round) } });
    setDrag(null);
  };
  const close = () => {
    ui.set({ musicOpen: false });
    focusComposer();
  };

  const handleUploadMp3 = async () => {
    if (uploading) return;
    setUploading(true);
    setPlayer({ error: null });
    try {
      const next = await invoke<MusicTrack[] | null>('music_upload');
      if (next) {
        const added = next.filter((t) => !state.tracks.some((x) => x.id === t.id));
        setPlayer({ tracks: next, queue: next, loaded: true });
        if (!state.current && added.length > 0) playTrack(added[0]);
      }
    } catch (e) {
      setPlayer({ error: String(e) });
    } finally {
      setUploading(false);
    }
  };

  const handleRemoveTrack = async (id: string) => {
    try {
      const next = await invoke<MusicTrack[]>('music_remove', { id });
      if (state.current?.id === id) {
        if (isSc(state.current)) scPause();
        else stopAudio();
        setPlayer({ tracks: next, queue: next, current: null, sound: null, playing: false, loading: false, currentTime: 0, duration: 0 });
      } else {
        setPlayer({ tracks: next, queue: next });
      }
    } catch (e) {
      setPlayer({ error: String(e) });
    }
  };

  const addSoundCloud = async (raw: string) => {
    const q = raw.trim();
    if (!q || state.scBusy) return;
    if (!isSoundCloudLink(q)) {
      // No key-free search exists, so search happens on soundcloud.com and the link comes back here.
      openExternal(`https://soundcloud.com/search/sounds?q=${encodeURIComponent(q)}`);
      setPlayer({ scHint: 'Search opened in your browser. Copy a song’s link (Share → Copy link) and paste it here.' });
      return;
    }
    setPlayer({ scBusy: true, error: null, scHint: null });
    try {
      const r = await invoke<{ tracks: MusicTrack[]; id: string; new: boolean }>('music_soundcloud_add', { url: q });
      setPlayer({ tracks: r.tracks, queue: r.tracks, loaded: true, scInput: '', tab: 'library' });
      const t = r.tracks.find((x) => x.id === r.id);
      if (t && state.current?.id !== t.id) playTrack(t);
      if (!r.new) setPlayer({ scHint: null });
    } catch (err) {
      setPlayer({ error: String(err) });
    } finally {
      setPlayer({ scBusy: false });
    }
  };

  const handleScSubmit = (e: FormEvent) => {
    e.preventDefault();
    void addSoundCloud(state.scInput);
  };

  // Pasting a link adds it straight away.
  const handleScPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData('text');
    if (isSoundCloudLink(text) && !state.scInput.trim()) {
      e.preventDefault();
      setPlayer({ scInput: text.trim() });
      void addSoundCloud(text);
    }
  };

  const handleVolume = (v: number) => {
    setVolume(v, false);
    setPlayer({ muted: false });
    act('setMusic', { music: { ...music, volume: v } });
  };

  const toggleMute = () => {
    const muted = !p.muted;
    setVolume(music.volume ?? 0.8, muted);
    setPlayer({ muted });
  };

  const duration = p.duration || p.current?.duration || 0;
  const setTab = (tab: Tab) => setPlayer({ tab });
  const sc = isSc(p.current);
  const title = (sc && p.sound?.title) || p.current?.title;
  const artist = (sc && p.sound?.artist) || p.current?.artist;
  const art = (sc && p.sound?.artwork) || p.current?.thumbnail;
  const page = (sc && p.sound?.url) || p.current?.url;

  return (
    <div
      className={`game-float music-float ${drag ? 'dragging' : ''} ${open ? '' : 'leaving'}`}
      ref={card}
      style={{ left: pos[0], top: pos[1] }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          close();
        }
      }}
    >
      <div className="game-bar" onPointerDown={start} onPointerMove={moveTo} onPointerUp={end} onPointerCancel={end} title="Drag to move">
        <span className="game-grip" aria-hidden />
        <Icon name="music" size={14} />
        <b className="strong">Music</b>
        <div className="music-tabs">
          <button type="button" className={`music-tab ${p.tab === 'library' ? 'on' : ''}`} onClick={() => setTab('library')}>
            Library{p.tracks.length > 0 ? ` (${p.tracks.length})` : ''}
          </button>
          <button type="button" className={`music-tab ${p.tab === 'sc' ? 'on' : ''}`} onClick={() => setTab('sc')}>
            SoundCloud
          </button>
        </div>
        <span className="grow" />
        <IconButton icon="close" tip="Close (music keeps playing)" size={24} onClick={close} />
      </div>

      <div className="music-body">
        <div className="music-now">
          <div className="row music-now-top">
            {art ? (
              <img className="music-art" src={art} alt="" />
            ) : (
              <div className="music-art placeholder-art">
                <Icon name="music" size={16} />
              </div>
            )}
            <div className="stack tight grow min0">
              <div className="strong small ellipsis">{title ?? 'Nothing playing'}</div>
              <div className="tiny dim ellipsis">{p.current ? artist : 'Upload an MP3 or add a SoundCloud link'}</div>
            </div>
            {p.current &&
              (sc && page ? (
                <button type="button" className="music-badge sc" title="Open on SoundCloud" onClick={() => openExternal(page)}>
                  SoundCloud ↗
                </button>
              ) : (
                <span className={`music-badge ${p.current.kind}`}>{p.current.kind === 'mp3' ? 'MP3' : 'SoundCloud'}</span>
              ))}
          </div>

          {sc && <div className="music-sc-slot" ref={slot} style={{ height: FRAME_HEIGHT }} />}

          <div className="music-seek-row">
            <span className="tiny dim mono">{formatTime(p.currentTime)}</span>
            <input
              type="range"
              className="music-range grow"
              min={0}
              max={duration > 0 ? duration : 100}
              step={0.5}
              disabled={!p.current || duration <= 0}
              value={Math.min(p.currentTime, duration || 100)}
              onChange={(e) => seekTo(Number(e.target.value))}
              aria-label="Seek position"
            />
            <span className="tiny dim mono">{formatTime(duration)}</span>
          </div>

          <div className="row music-controls">
            <IconButton icon="skipBack" tip="Previous" size={26} disabled={!p.current && p.tracks.length === 0} onClick={() => playAdjacent(-1)} />
            <button
              type="button"
              className="button primary music-play-btn"
              disabled={!p.current && p.tracks.length === 0}
              onClick={togglePlayPause}
              title={p.playing ? 'Pause' : 'Play'}
            >
              {p.loading ? <Spinner size={13} /> : <Icon name={p.playing ? 'pause' : 'play'} size={15} />}
            </button>
            <IconButton icon="skipForward" tip="Next" size={26} disabled={!p.current && p.tracks.length === 0} onClick={() => playAdjacent(1)} />
            <IconButton
              icon="repeat"
              tip={p.repeat ? 'Repeat: On' : 'Repeat: Off'}
              size={26}
              active={p.repeat}
              onClick={() => setPlayer({ repeat: !p.repeat })}
            />
            <span className="grow" />
            <IconButton
              icon={p.muted || (music.volume ?? 0.8) === 0 ? 'volumeOff' : 'volume'}
              tip={p.muted ? 'Unmute' : 'Mute'}
              size={24}
              onClick={toggleMute}
            />
            <input
              type="range"
              className="music-range music-vol"
              min={0}
              max={1}
              step={0.02}
              value={p.muted ? 0 : (music.volume ?? 0.8)}
              onChange={(e) => handleVolume(Number(e.target.value))}
              aria-label="Volume"
            />
          </div>
        </div>

        {p.error && (
          <div className="music-error tiny red row">
            <span className="grow">{p.error}</span>
            <IconButton icon="close" tip="Dismiss" size={18} onClick={() => setPlayer({ error: null })} />
          </div>
        )}

        {p.tab === 'library' ? (
          <div className="music-section stack">
            <div className="row music-actions">
              <button type="button" className="button compact primary" disabled={uploading} onClick={handleUploadMp3}>
                {uploading ? <Spinner size={12} /> : <Icon name="upload" size={13} />}
                Upload MP3
              </button>
              <button type="button" className="button compact" onClick={() => setTab('sc')}>
                <Icon name="plus" size={13} />
                Add from SoundCloud
              </button>
            </div>

            <div className="music-list">
              {p.tracks.length === 0 ? (
                <div className="music-empty small dim">Your library is empty. Upload an MP3, or add songs and playlists from SoundCloud.</div>
              ) : (
                p.tracks.map((t) => (
                  <TrackRow
                    key={t.id}
                    track={t}
                    action={<IconButton icon="close" tip="Remove from library" size={22} onClick={() => void handleRemoveTrack(t.id)} />}
                  />
                ))
              )}
            </div>
          </div>
        ) : (
          <div className="music-section stack">
            <form className="row music-search-form" onSubmit={handleScSubmit}>
              <input
                ref={input}
                className="input grow"
                placeholder="Paste a SoundCloud link, or type to search"
                value={p.scInput}
                onChange={(e) => setPlayer({ scInput: e.target.value, scHint: null })}
                onPaste={handleScPaste}
              />
              <button type="submit" className="button primary compact" disabled={p.scBusy || !p.scInput.trim()}>
                {p.scBusy ? <Spinner size={12} /> : isSoundCloudLink(p.scInput) ? 'Add' : 'Search'}
              </button>
            </form>
            <div className="music-empty small dim">
              {p.scHint ?? (
                <>
                  Paste a link to a SoundCloud song or playlist to add it to your library. Typing a name searches on soundcloud.com in your
                  browser, where you can copy a song’s link.
                </>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
