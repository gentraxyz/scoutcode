// Chess against Scout: you play white, the computer answers with a short alpha-beta search.
import { useEffect, useRef, useState, type KeyboardEvent, type ReactElement } from 'react';

type Color = 'w' | 'b';
type Kind = 'p' | 'n' | 'b' | 'r' | 'q' | 'k';
/** A piece is its color then its kind, like 'wq'. Square 0 is a8, 63 is h1. */
type Sq = string | null;
type Move = { from: number; to: number; promo?: Kind; castle?: 'k' | 'q'; ep?: boolean };
type Pos = { board: Sq[]; turn: Color; castle: string; ep: number | null };
type Game = { pos: Pos; last: Move | null; history: Pos[]; lastHistory: (Move | null)[] };

const SAVE_KEY = 'scout.chess';
/** Plies the computer looks past its own move. */
const DEPTH = 3;
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR';
const VALUE: Record<Kind, number> = { p: 100, n: 320, b: 330, r: 500, q: 900, k: 0 };
const NAME: Record<Kind, string> = { k: 'king', q: 'queen', r: 'rook', b: 'bishop', n: 'knight', p: 'pawn' };
const KNIGHT = [[-2, -1], [-2, 1], [-1, -2], [-1, 2], [1, -2], [1, 2], [2, -1], [2, 1]];
const KING = [[-1, -1], [-1, 0], [-1, 1], [0, -1], [0, 1], [1, -1], [1, 0], [1, 1]];
const ROOK = [[-1, 0], [1, 0], [0, -1], [0, 1]];
const BISHOP = [[-1, -1], [-1, 1], [1, -1], [1, 1]];

function startPos(): Pos {
  const board: Sq[] = [];
  for (const ch of START.replace(/\//g, '')) {
    if (/\d/.test(ch)) board.push(...Array(+ch).fill(null));
    else board.push((ch === ch.toUpperCase() ? 'w' : 'b') + ch.toLowerCase());
  }
  return { board, turn: 'w', castle: 'KQkq', ep: null };
}

const at = (r: number, c: number) => (r >= 0 && r < 8 && c >= 0 && c < 8 ? r * 8 + c : -1);
const other = (c: Color): Color => (c === 'w' ? 'b' : 'w');

/** Whether `by` attacks square `sq`. */
function attacked(board: Sq[], sq: number, by: Color): boolean {
  const r = sq >> 3, c = sq & 7;
  const dir = by === 'w' ? 1 : -1; // a white pawn attacks from the row below
  for (const dc of [-1, 1]) {
    const i = at(r + dir, c + dc);
    if (i >= 0 && board[i] === by + 'p') return true;
  }
  for (const [dr, dc] of KNIGHT) {
    const i = at(r + dr, c + dc);
    if (i >= 0 && board[i] === by + 'n') return true;
  }
  for (const [dr, dc] of KING) {
    const i = at(r + dr, c + dc);
    if (i >= 0 && board[i] === by + 'k') return true;
  }
  for (const [lines, kinds] of [[ROOK, 'rq'], [BISHOP, 'bq']] as const) {
    for (const [dr, dc] of lines) {
      for (let k = 1; ; k++) {
        const i = at(r + dr * k, c + dc * k);
        if (i < 0) break;
        const p = board[i];
        if (!p) continue;
        if (p[0] === by && kinds.includes(p[1])) return true;
        break;
      }
    }
  }
  return false;
}

const kingAt = (board: Sq[], color: Color) => board.indexOf(color + 'k');
const inCheck = (pos: Pos, color: Color = pos.turn) => attacked(pos.board, kingAt(pos.board, color), other(color));

function pseudo(pos: Pos): Move[] {
  const { board, turn } = pos;
  const out: Move[] = [];
  const push = (from: number, to: number) => {
    const p = board[from]!;
    const row = to >> 3;
    if (p[1] === 'p' && (row === 0 || row === 7)) for (const promo of ['q', 'n', 'r', 'b'] as Kind[]) out.push({ from, to, promo });
    else out.push({ from, to });
  };
  for (let sq = 0; sq < 64; sq++) {
    const p = board[sq];
    if (!p || p[0] !== turn) continue;
    const r = sq >> 3, c = sq & 7, kind = p[1] as Kind;
    if (kind === 'p') {
      const dir = turn === 'w' ? -1 : 1;
      const one = at(r + dir, c);
      if (one >= 0 && !board[one]) {
        push(sq, one);
        const two = at(r + 2 * dir, c);
        if ((turn === 'w' ? r === 6 : r === 1) && !board[two]) out.push({ from: sq, to: two });
      }
      for (const dc of [-1, 1]) {
        const i = at(r + dir, c + dc);
        if (i < 0) continue;
        if (board[i] && board[i]![0] !== turn) push(sq, i);
        else if (i === pos.ep) out.push({ from: sq, to: i, ep: true });
      }
    } else if (kind === 'n' || kind === 'k') {
      for (const [dr, dc] of kind === 'n' ? KNIGHT : KING) {
        const i = at(r + dr, c + dc);
        if (i >= 0 && board[i]?.[0] !== turn) out.push({ from: sq, to: i });
      }
      if (kind === 'k') {
        const enemy = other(turn);
        const home = turn === 'w' ? 60 : 4;
        const [ks, qs] = turn === 'w' ? ['K', 'Q'] : ['k', 'q'];
        if (sq === home && !attacked(board, home, enemy)) {
          if (pos.castle.includes(ks) && !board[home + 1] && !board[home + 2] && board[home + 3] === turn + 'r' && !attacked(board, home + 1, enemy) && !attacked(board, home + 2, enemy))
            out.push({ from: home, to: home + 2, castle: 'k' });
          if (pos.castle.includes(qs) && !board[home - 1] && !board[home - 2] && !board[home - 3] && board[home - 4] === turn + 'r' && !attacked(board, home - 1, enemy) && !attacked(board, home - 2, enemy))
            out.push({ from: home, to: home - 2, castle: 'q' });
        }
      }
    } else {
      const lines = kind === 'r' ? ROOK : kind === 'b' ? BISHOP : [...ROOK, ...BISHOP];
      for (const [dr, dc] of lines) {
        for (let k = 1; ; k++) {
          const i = at(r + dr * k, c + dc * k);
          if (i < 0) break;
          if (!board[i]) { out.push({ from: sq, to: i }); continue; }
          if (board[i]![0] !== turn) out.push({ from: sq, to: i });
          break;
        }
      }
    }
  }
  return out;
}

function apply(pos: Pos, m: Move): Pos {
  const board = pos.board.slice();
  const p = board[m.from]!;
  board[m.to] = m.promo ? p[0] + m.promo : p;
  board[m.from] = null;
  if (m.ep) board[m.to + (pos.turn === 'w' ? 8 : -8)] = null;
  if (m.castle === 'k') { board[m.to - 1] = board[m.to + 1]; board[m.to + 1] = null; }
  if (m.castle === 'q') { board[m.to + 1] = board[m.to - 2]; board[m.to - 2] = null; }
  let castle = pos.castle;
  const drop = (sq: number) => {
    if (sq === 60) castle = castle.replace(/[KQ]/g, '');
    if (sq === 4) castle = castle.replace(/[kq]/g, '');
    if (sq === 63) castle = castle.replace('K', '');
    if (sq === 56) castle = castle.replace('Q', '');
    if (sq === 7) castle = castle.replace('k', '');
    if (sq === 0) castle = castle.replace('q', '');
  };
  drop(m.from);
  drop(m.to);
  const ep = p[1] === 'p' && Math.abs(m.to - m.from) === 16 ? (m.to + m.from) / 2 : null;
  return { board, turn: other(pos.turn), castle, ep };
}

function legal(pos: Pos): Move[] {
  return pseudo(pos).filter((m) => !inCheck(apply(pos, m), pos.turn));
}

/** Only kings, or a king and one minor piece against a king: nobody can mate. */
function deadDraw(board: Sq[]) {
  const rest = board.filter((p) => p && p[1] !== 'k') as string[];
  return rest.length === 0 || (rest.length === 1 && (rest[0][1] === 'n' || rest[0][1] === 'b'));
}

// ---------- the computer ----------

function evaluate(pos: Pos): number {
  let score = 0;
  pos.board.forEach((p, sq) => {
    if (!p) return;
    const r = sq >> 3, c = sq & 7;
    const kind = p[1] as Kind;
    const center = 7 - Math.abs(r - 3.5) - Math.abs(c - 3.5);
    let v = VALUE[kind];
    if (kind === 'n' || kind === 'b') v += center * 5;
    if (kind === 'q') v += center * 2;
    if (kind === 'p') v += (p[0] === 'w' ? 6 - r : r - 1) * 7 + (c >= 2 && c <= 5 ? 6 : 0);
    if (kind === 'k') v += (p[0] === 'w' ? r === 7 : r === 0) ? 15 : -10;
    score += p[0] === 'w' ? v : -v;
  });
  return pos.turn === 'w' ? score : -score;
}

function ordered(pos: Pos, moves: Move[]) {
  const gain = (m: Move) => (pos.board[m.to] ? VALUE[pos.board[m.to]![1] as Kind] * 10 - VALUE[pos.board[m.from]![1] as Kind] : 0) + (m.promo === 'q' ? 8000 : 0);
  return moves.sort((a, b) => gain(b) - gain(a));
}

function search(pos: Pos, depth: number, alpha: number, beta: number, ply: number): number {
  if (depth === 0) return evaluate(pos);
  const moves = legal(pos);
  if (!moves.length) return inCheck(pos) ? -100000 + ply : 0;
  for (const m of ordered(pos, moves)) {
    const s = -search(apply(pos, m), depth - 1, -beta, -alpha, ply + 1);
    if (s >= beta) return beta;
    if (s > alpha) alpha = s;
  }
  return alpha;
}

function bestMove(pos: Pos): Move | null {
  const moves = ordered(pos, legal(pos));
  let best: Move | null = null;
  let bestRaw = -Infinity;
  let bestScore = -Infinity;
  for (const m of moves) {
    // Moves within a few points of the best are scored exactly and a little noise picks among them, so the same
    // position doesn't always get the same answer. Worse ones are cut off below that and can't win on noise.
    const raw = -search(apply(pos, m), DEPTH, -Infinity, -(bestRaw - 8), 1);
    const s = raw + Math.random() * 8;
    bestRaw = Math.max(bestRaw, raw);
    if (s > bestScore) { bestScore = s; best = m; }
  }
  return best;
}

// ---------- the pieces ----------

const BASE = <rect x="10" y="36.5" width="25" height="4.5" rx="2" />;
/** Drawn on a 45 by 45 grid. Body shapes take the piece's fill and outline, `.pc-line` and `.pc-dot` its details. */
const SHAPES: Record<Kind, ReactElement> = {
  p: <>
    <path d="M18.5 21C18.5 27 14.5 31 13.5 37H31.5C30.5 31 26.5 27 26.5 21Z" />
    <rect x="16" y="18" width="13" height="3.2" rx="1.6" />
    <circle cx="22.5" cy="12" r="5.6" />
    {BASE}
  </>,
  r: <>
    <path d="M15.5 16H29.5L28.5 33H16.5Z" />
    <path d="M12 8H16.5V11.5H20V8H25V11.5H28.5V8H33V16.5H12Z" />
    <rect x="13" y="33" width="19" height="3.6" rx="1" />
    {BASE}
    <path className="pc-line" d="M16 20H29" />
  </>,
  n: <>
    <path d="M14 37C14 30.5 17 27.5 20.5 24.2C17.5 23.8 14.2 24.8 11.7 26.3C9.2 27.6 7.4 25.4 8.5 22.7C10 19.2 12.8 16.2 14.8 13.2C16 11.2 17.4 9.2 19 8.5L19.5 4.8L22.6 8C29.2 8.6 33.6 14.2 34 22.2C34.3 28.2 32.2 32.4 31.2 37Z" />
    {BASE}
    <circle className="pc-dot" cx="16.8" cy="14" r="1.4" />
    <path className="pc-line" d="M24.5 10.5C29 13.5 30.8 19 30.4 27.5" />
  </>,
  b: <>
    <path d="M18 29H27L29 37H16Z" />
    <rect x="16" y="26" width="13" height="3.2" rx="1.6" />
    <path d="M22.5 10C29 14 31 20 28 26H17C14 20 16 14 22.5 10Z" />
    <circle cx="22.5" cy="7" r="2.8" />
    {BASE}
    <path className="pc-line" d="M22.5 15V22M19.3 18.4H25.7" />
  </>,
  q: <>
    <path d="M14 31L9.5 14L14.6 22.4L17.5 11L20.2 20.6L22.5 9L24.8 20.6L27.5 11L30.4 22.4L35.5 14L31 31Z" />
    <circle cx="9.5" cy="13" r="2.3" /><circle cx="17.5" cy="10" r="2.3" /><circle cx="22.5" cy="7.6" r="2.3" />
    <circle cx="27.5" cy="10" r="2.3" /><circle cx="35.5" cy="13" r="2.3" />
    <rect x="13" y="31" width="19" height="3.6" rx="1.6" />
    <path d="M14 34.6H31L32 37H13Z" />
    {BASE}
  </>,
  k: <>
    <path d="M21 2.5H24V5.5H27V8.5H24V12.5H21V8.5H18V5.5H21Z" />
    <path d="M13.5 31C9.5 25 9.5 16.5 16.2 14.8C19.2 14.2 21.2 15.6 22.5 17.6C23.8 15.6 25.8 14.2 28.8 14.8C35.5 16.5 35.5 25 31.5 31Z" />
    <rect x="13" y="31" width="19" height="3.6" rx="1.6" />
    <path d="M14 34.6H31L32 37H13Z" />
    {BASE}
    <path className="pc-line" d="M22.5 18V28" />
  </>,
};

export function ChessPiece({ piece, className = '' }: { piece: string; className?: string }) {
  return <svg className={`chess-piece ${piece[0]} ${className}`} viewBox="0 0 45 45" aria-hidden>{SHAPES[piece[1] as Kind]}</svg>;
}

// ---------- saving ----------

function fresh(): Game {
  return { pos: startPos(), last: null, history: [], lastHistory: [] };
}

function load(): Game {
  try {
    const g = JSON.parse(localStorage.getItem(SAVE_KEY) ?? 'null') as Game | null;
    if (g?.pos?.board?.length === 64) return { ...fresh(), ...g };
  } catch {}
  return fresh();
}

function save(g: Game) {
  try { localStorage.setItem(SAVE_KEY, JSON.stringify(g)); } catch {}
}

let current: Game | null = null;
const FILES = 'abcdefgh';
const sqName = (sq: number) => FILES[sq & 7] + (8 - (sq >> 3));

export function Chess({ onEscape, autoFocus }: { onEscape: () => void; autoFocus: boolean }) {
  const [game, setGame] = useState<Game>(() => (current ??= load()));
  const [picked, setPicked] = useState<number | null>(null);
  const [cursor, setCursor] = useState(52);
  const host = useRef<HTMLDivElement>(null);
  const update = (g: Game) => {
    current = g;
    save(g);
    setGame(g);
  };
  useEffect(() => {
    if (autoFocus) host.current?.focus();
  }, []);

  const { pos } = game;
  const moves = legal(pos);
  const check = inCheck(pos);
  const over = !moves.length || deadDraw(pos.board);
  const status = !moves.length ? (check ? (pos.turn === 'w' ? 'Checkmate. Scout wins.' : 'Checkmate. You win!') : 'Stalemate. A draw.') : deadDraw(pos.board) ? 'Draw. Not enough pieces to mate.' : pos.turn === 'b' ? 'Scout is thinking…' : check ? 'Check! Your move.' : 'Your move.';

  // The computer answers a moment after yours, so your move shows first.
  useEffect(() => {
    if (pos.turn !== 'b' || over) return;
    const t = setTimeout(() => {
      const m = bestMove(pos);
      if (m && current?.pos === pos) update({ ...game, pos: apply(pos, m), last: m });
    }, 280);
    return () => clearTimeout(t);
  }, [pos]);

  const targets = picked == null ? [] : moves.filter((m) => m.from === picked);
  const choose = (sq: number) => {
    if (over || pos.turn !== 'w') return;
    const m = targets.find((t) => t.to === sq && (!t.promo || t.promo === 'q'));
    if (m) {
      update({ pos: apply(pos, m), last: m, history: [...game.history, pos], lastHistory: [...game.lastHistory, game.last] });
      setPicked(null);
      return;
    }
    setPicked(pos.board[sq]?.[0] === 'w' && sq !== picked ? sq : null);
  };
  const undo = () => {
    if (!game.history.length || pos.turn !== 'w') return;
    const n = game.history.length - 1;
    update({ pos: game.history[n], last: game.lastHistory[n], history: game.history.slice(0, n), lastHistory: game.lastHistory.slice(0, n) });
    setPicked(null);
    host.current?.focus();
  };
  const restart = () => {
    update(fresh());
    setPicked(null);
    host.current?.focus();
  };
  const key = (e: KeyboardEvent) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const step: Record<string, number> = { ArrowUp: -8, ArrowDown: 8, ArrowLeft: -1, ArrowRight: 1 };
    if (e.key === 'Escape') {
      e.preventDefault();
      if (picked != null) return setPicked(null);
      return onEscape();
    }
    if (e.key in step) {
      e.preventDefault();
      const d = step[e.key];
      const next = cursor + d;
      if (next < 0 || next > 63 || (Math.abs(d) === 1 && (next >> 3) !== (cursor >> 3))) return;
      setCursor(next);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      choose(cursor);
    }
  };
  const king = check ? kingAt(pos.board, pos.turn) : -1;

  return (
    <div className="game-chess" ref={host} tabIndex={0} onKeyDown={key} aria-label="Chess. You play white. Move the cursor with the arrow keys and pick with Enter.">
      <div className="row game-scores">
        <div className={`chess-status small ${over ? 'strong' : ''}`} aria-live="polite">{status}</div>
        <span className="grow" />
        <button type="button" className="button compact ghost" onClick={undo} disabled={!game.history.length || pos.turn !== 'w'}>Undo</button>
        <button type="button" className="button compact" onClick={restart}>New game</button>
      </div>
      <div className="chess-board">
        {pos.board.map((p, sq) => {
          const dark = ((sq >> 3) + (sq & 7)) % 2 === 1;
          const target = targets.some((t) => t.to === sq);
          const cls = ['chess-sq', dark ? 'dark' : 'light', sq === picked && 'picked', target && (p || (targets.find((t) => t.to === sq)?.ep) ? 'capture' : 'move'), (game.last?.from === sq || game.last?.to === sq) && 'last', sq === king && 'check', sq === cursor && 'cursor'].filter(Boolean).join(' ');
          return (
            <div key={sq} className={cls} onPointerDown={() => { setCursor(sq); choose(sq); host.current?.focus(); }} aria-label={`${sqName(sq)}${p ? `, ${p[0] === 'w' ? 'white' : 'black'} ${NAME[p[1] as Kind]}` : ''}`}>
              {p && <ChessPiece key={p} piece={p} className={game.last?.to === sq ? 'moved' : ''} />}
              {(sq & 7) === 0 && <i className="chess-rank">{8 - (sq >> 3)}</i>}
              {sq >> 3 === 7 && <i className="chess-file">{FILES[sq & 7]}</i>}
            </div>
          );
        })}
        {over && (
          <div className="game-over">
            <b className="strong big">{status}</b>
            <div className="row"><button type="button" className="button primary" onClick={restart}>New game</button></div>
          </div>
        )}
      </div>
      <div className="small dim game-hint">Click a piece, then where it goes. Pawns promote to a queen.</div>
    </div>
  );
}
