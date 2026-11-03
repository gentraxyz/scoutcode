// Experimental: a game to play while Scout works. 2048 for now, in a card the user drags anywhere or a tab in the panel.
import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from 'react';
import { act, type Games } from './api';
import { core, focusComposer, ui, useUi, useView } from './store';
import { IconButton } from './widgets';

// ---------- 2048 ----------

type Tile = { id: number; value: number; r: number; c: number; fresh?: boolean; merged?: boolean; gone?: boolean };
/** `gain` is what the last move scored and `moves` counts them, so each gain floats up once. */
type Board = { tiles: Tile[]; score: number; best: number; won: boolean; keepGoing: boolean; over: boolean; gain?: number; moves?: number };
type Dir = 'up' | 'down' | 'left' | 'right';

const SIZE = 4;
const SAVE_KEY = 'scout.2048';
/** As long as a tile's slide in the styles: joined tiles go, and their partners show the sum, once it has played. */
const SLIDE_MS = 120;
const NUDGE: Record<Dir, [number, number]> = { up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] };
const KEYS: Record<string, Dir> = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', w: 'up', s: 'down', a: 'left', d: 'right' };
let nextId = 1;

function addRandom(tiles: Tile[]): Tile[] {
  const taken = new Set(tiles.map((t) => t.r * SIZE + t.c));
  const free = [...Array(SIZE * SIZE).keys()].filter((i) => !taken.has(i));
  if (!free.length) return tiles;
  const at = free[Math.floor(Math.random() * free.length)];
  return [...tiles, { id: nextId++, value: Math.random() < 0.9 ? 2 : 4, r: Math.floor(at / SIZE), c: at % SIZE, fresh: true }];
}

function newBoard(best: number): Board {
  return { tiles: addRandom(addRandom([])), score: 0, best, won: false, keepGoing: false, over: false };
}

function canMove(tiles: Tile[]) {
  if (tiles.length < SIZE * SIZE) return true;
  const at = new Map(tiles.map((t) => [t.r * SIZE + t.c, t.value]));
  for (let r = 0; r < SIZE; r++) {
    for (let c = 0; c < SIZE; c++) {
      const v = at.get(r * SIZE + c);
      if ((c + 1 < SIZE && v === at.get(r * SIZE + c + 1)) || (r + 1 < SIZE && v === at.get((r + 1) * SIZE + c))) return true;
    }
  }
  return false;
}

/** Slides every tile as far as it goes toward `dir`, joining each equal pair once. Null when nothing would move. */
function slide(board: Board, dir: Dir): Board | null {
  const grid: (Tile | undefined)[][] = [...Array(SIZE)].map(() => Array(SIZE));
  for (const t of board.tiles) if (!t.gone) grid[t.r][t.c] = { ...t, fresh: false, merged: false };
  const out: Tile[] = [];
  let moved = false;
  let gained = 0;
  for (let line = 0; line < SIZE; line++) {
    // The line's cells, from the edge the tiles slide to.
    const cells = [...Array(SIZE).keys()].map((i) => {
      const k = dir === 'left' || dir === 'up' ? i : SIZE - 1 - i;
      return dir === 'left' || dir === 'right' ? [line, k] : [k, line];
    });
    let last: Tile | null = null;
    let place = 0;
    for (const [r, c] of cells) {
      const t = grid[r][c];
      if (!t) continue;
      if (last && !last.merged && last.value === t.value) {
        // It slides under the one ahead, which doubles, and goes once the slide has played.
        out.push({ ...t, r: last.r, c: last.c, gone: true });
        last.value *= 2;
        last.merged = true;
        gained += last.value;
        moved = true;
        continue;
      }
      const [tr, tc] = cells[place++];
      if (tr !== t.r || tc !== t.c) moved = true;
      last = { ...t, r: tr, c: tc };
      out.push(last);
    }
  }
  if (!moved) return null;
  const tiles = addRandom(out);
  const live = tiles.filter((t) => !t.gone);
  const score = board.score + gained;
  return { ...board, tiles, score, best: Math.max(board.best, score), won: board.won || live.some((t) => t.value >= 2048), over: !canMove(live), gain: gained, moves: (board.moves ?? 0) + 1 };
}

function load(): Board {
  try {
    const saved = JSON.parse(localStorage.getItem(SAVE_KEY) ?? 'null') as Board | null;
    if (saved?.tiles?.length) {
      const tiles = saved.tiles.filter((t) => !t.gone).map((t) => ({ id: nextId++, value: t.value, r: t.r, c: t.c }));
      return { ...saved, tiles, over: !canMove(tiles) };
    }
    return newBoard(saved?.best ?? 0);
  } catch {
    return newBoard(0);
  }
}

function save(b: Board) {
  try {
    localStorage.setItem(SAVE_KEY, JSON.stringify({ ...b, tiles: b.tiles.filter((t) => !t.gone).map(({ value, r, c }) => ({ value, r, c })) }));
  } catch {}
}

// The game outlives the card and the panel tab, so closing it never loses a board.
let current: Board | null = null;
/** The next game to mount takes the keyboard: true when the user opened it, false when Scout's work did. */
let focusOnOpen = false;

function Game2048({ onEscape }: { onEscape: () => void }) {
  const [board, setBoard] = useState<Board>(() => (current ??= load()));
  const host = useRef<HTMLDivElement>(null);
  const grid = useRef<HTMLDivElement>(null);
  const swipe = useRef<{ x: number; y: number } | null>(null);
  const update = (b: Board) => {
    current = b;
    save(b);
    setBoard(b);
  };
  useEffect(() => {
    if (focusOnOpen) host.current?.focus();
    focusOnOpen = false;
  }, []);
  useEffect(() => {
    if (!board.tiles.some((t) => t.gone)) return;
    const t = setTimeout(() => {
      if (current === board) update({ ...board, tiles: board.tiles.filter((x) => !x.gone) });
    }, SLIDE_MS);
    return () => clearTimeout(t);
  }, [board]);

  const ended = board.over || (board.won && !board.keepGoing);
  const move = (dir: Dir) => {
    if (ended) return;
    const next = slide(current ?? board, dir);
    if (next) return update(next);
    // Nothing moves that way: the board leans toward it and back.
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const [x, y] = NUDGE[dir];
    grid.current?.animate([{ transform: 'none' }, { transform: `translate(${x * 5}px, ${y * 5}px)` }, { transform: 'none' }], { duration: 180, easing: 'cubic-bezier(0.3, 0, 0.2, 1)' });
  };
  // While tiles slide, a joined pair still shows its old number; the sum pops in when they meet.
  const sliding = board.tiles.some((t) => t.gone);
  const key = (e: KeyboardEvent) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      return onEscape();
    }
    const dir = KEYS[e.key.length === 1 ? e.key.toLowerCase() : e.key];
    if (!dir) return;
    e.preventDefault();
    move(dir);
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
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 24) return;
    move(Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? 'right' : 'left') : dy > 0 ? 'down' : 'up');
  };
  const restart = () => {
    update(newBoard(board.best));
    host.current?.focus();
  };

  return (
    <div className="game-2048" ref={host} tabIndex={0} onKeyDown={key} aria-label="2048. Slide the tiles with the arrow keys.">
      <div className="row game-scores">
        <div className="game-score">
          <span>Score</span><b>{board.score}</b>
          {!!board.gain && <i key={board.moves} className="game-gain" aria-hidden>+{board.gain}</i>}
        </div>
        <div className="game-score"><span>Best</span><b key={board.best} className={board.gain && board.best === board.score ? 'game-best' : ''}>{board.best}</b></div>
        <span className="grow" />
        <button type="button" className="button compact" onClick={restart}>New game</button>
      </div>
      <div className="game-board" ref={grid} onPointerDown={down} onPointerUp={up} onPointerCancel={() => { swipe.current = null; }}>
        {[...Array(SIZE * SIZE)].map((_, i) => <div key={i} className="game-cell" style={{ '--r': Math.floor(i / SIZE), '--c': i % SIZE } as CSSProperties} />)}
        {board.tiles.map((t) => {
          const shown = t.merged && sliding ? t.value / 2 : t.value;
          return (
            <div key={t.id} className={`game-tile ${t.gone ? 'gone' : ''}`} data-v={shown > 2048 ? 'big' : shown} style={{ '--r': t.r, '--c': t.c } as CSSProperties}>
              {/* Keyed by the number, so the pop plays again on every join. */}
              <span key={shown} className={t.fresh ? 'fresh' : t.merged && !sliding ? 'merged' : ''} data-digits={String(shown).length}>{shown}</span>
            </div>
          );
        })}
        {ended && (
          <div className={`game-over ${board.over ? '' : 'won'}`}>
            <b className="strong big">{board.over ? 'No moves left' : 'You made 2048!'}</b>
            <div className="row">
              {!board.over && <button type="button" className="button" onClick={() => { update({ ...board, keepGoing: true }); host.current?.focus(); }}>Keep going</button>}
              <button type="button" className="button primary" onClick={restart}>{board.over ? 'Try again' : 'New game'}</button>
            </div>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Arrow keys, WASD or swipe. Join equal tiles to reach 2048.</div>
    </div>
  );
}

// ---------- where the game shows ----------

/** Opens the game where the settings put it. `byUser` gives it the keyboard; Scout's own work leaves the composer be. */
export function openGame(games: Games, byUser: boolean) {
  focusOnOpen = byUser;
  if (games.place === 'panel') ui.set({ dockOpen: true, dockTab: 'games' });
  else ui.set({ gameOpen: true });
}

export function closeGame() {
  const { dockOpen, dockTab } = ui.get();
  ui.set({ gameOpen: false, ...(dockOpen && dockTab === 'games' ? { dockOpen: false } : {}) });
}

export function gameShowing(games: Games) {
  const u = ui.get();
  return games.place === 'panel' ? u.dockOpen && u.dockTab === 'games' : u.gameOpen;
}

/** Opens the game when Scout starts working and closes it when every chat has finished, as the settings say. */
export function useGameAutomation() {
  const { anyRunning } = useView('run');
  const was = useRef(anyRunning);
  useEffect(() => {
    if (was.current === anyRunning) return;
    was.current = anyRunning;
    const { games } = core.get().view.cfg;
    if (!games.on) return;
    const { dockOpen, dockTab } = ui.get();
    // In the panel, the game never takes the place of a tab the user is looking at.
    const panelFree = games.place !== 'panel' || !dockOpen || dockTab === 'games';
    if (anyRunning && games.openOnRun && panelFree) openGame(games, false);
    if (!anyRunning && games.autoClose) closeGame();
  }, [anyRunning]);
}

/** The game in the panel's Games tab. */
export function GamesTab() {
  return <div className="dock-page game-page"><Game2048 onEscape={focusComposer} /></div>;
}

const CARD_W = 304;

function clamp(pos: [number, number], el: HTMLElement | null): [number, number] {
  const w = el?.offsetWidth ?? CARD_W;
  const h = el?.offsetHeight ?? 432;
  return [Math.max(4, Math.min(pos[0], window.innerWidth - w - 24)), Math.max(4, Math.min(pos[1], window.innerHeight - h - 4))];
}

/** The game as a card over the window, dragged by its bar. Where it's left is saved. */
export function FloatingGame() {
  const { games } = useView('cfg');
  const open = useUi((u) => u.gameOpen);
  const card = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ pos: [number, number]; dx: number; dy: number } | null>(null);
  const [, setSize] = useState(0);
  // Stays up for its closing animation after it's closed.
  const [shown, setShown] = useState(open);
  useEffect(() => {
    if (open) return setShown(true);
    const t = setTimeout(() => setShown(false), 150);
    return () => clearTimeout(t);
  }, [open]);
  useEffect(() => {
    const resized = () => setSize((n) => n + 1);
    window.addEventListener('resize', resized);
    return () => window.removeEventListener('resize', resized);
  }, []);
  if (!games.on || games.place !== 'float' || !(open || shown)) return null;

  // Until it's dragged, it sits above the message box at the right.
  const pos = drag?.pos ?? clamp(games.pos ?? [Infinity, window.innerHeight - (card.current?.offsetHeight ?? 432) - 150], card.current);
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
    act('setGames', { games: { ...games, pos: drag.pos.map(Math.round) } });
    setDrag(null);
  };
  const close = () => {
    ui.set({ gameOpen: false });
    focusComposer();
  };
  return (
    <div className={`game-float ${drag ? 'dragging' : ''} ${open ? '' : 'leaving'}`} ref={card} style={{ left: pos[0], top: pos[1] }}>
      <div className="game-bar" onPointerDown={start} onPointerMove={moveTo} onPointerUp={end} onPointerCancel={end} title="Drag to move">
        <span className="game-grip" aria-hidden />
        <b className="strong">2048</b>
        <span className="grow" />
        <IconButton icon="close" tip="Close (Esc)" size={24} onClick={close} />
      </div>
      <Game2048 onEscape={close} />
    </div>
  );
}
