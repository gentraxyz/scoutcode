// The panel on the right: a real terminal, what changed since the last commit, and the project's files with an editor.
import { useEffect, useRef, useState } from 'react';
import { Channel } from '@tauri-apps/api/core';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal as XTerm } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { basicSetup, EditorView } from 'codemirror';
import { Compartment, EditorState } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { tags } from '@lezer/highlight';
import { invoke, type DiffLines } from './api';
import { GamesTab } from './Games';
import { ReviewTab } from './ReviewTab';
import { Resizer } from './Sidebar';
import { ui, useUi, useView, type DockTab } from './store';
import { subscribeTheme, themeColor } from './theme';
import { DiffBody, Icon, IconButton, Modal, Spinner } from './widgets';

// ---------- terminal ----------

const TERM_FONT = '"Cascadia Mono", Consolas, monospace';
const TERM_SIZE = 13;
const TERM_LINE = 1.25;

/** xterm can't read CSS variables, so it takes the theme's colors when it opens and again when the theme changes. */
function terminalTheme() {
  const glass = document.documentElement.dataset.material === 'glass';
  // xterm's own cursor stays invisible: the caret below draws it.
  return { background: glass ? 'rgba(0, 0, 0, 0)' : themeColor('sidebar'), foreground: themeColor('text'), cursor: 'rgba(0, 0, 0, 0)', cursorAccent: 'rgba(0, 0, 0, 0)', selectionBackground: '#ffffff30', black: themeColor('surface'), brightBlack: themeColor('dim'), red: '#e86464', green: '#4fbf7f', yellow: '#e5c07b', blue: '#5b9cf5', magenta: '#c586c0', cyan: '#56b6c2', white: themeColor('text'), brightWhite: themeColor('strong') };
}

/** A guess at the panel's rows and columns before it has opened, for the shell started ahead of time. */
function terminalGuess(width: number) {
  return { cols: Math.max(20, Math.floor((width - 24) / (TERM_SIZE * 0.6))), rows: Math.max(4, Math.floor((window.innerHeight - 100) / Math.ceil(TERM_SIZE * TERM_LINE))) };
}

/**
 * A thin, round caret that glides to where the shell's cursor is. Moves along a line slide; jumps to another line (or a
 * long way) land at once, so it never flies across the screen. Solid while typing, a soft blink once idle, dim when
 * the terminal isn't focused, gone when the program hides its cursor.
 */
function attachCaret(xterm: XTerm) {
  const screen = xterm.element!.querySelector('.xterm-screen') as HTMLElement;
  const caret = document.createElement('div');
  caret.className = 'term-caret';
  screen.appendChild(caret);
  let frame = 0;
  let row = -1;
  let col = -1;
  let idle = 0;
  const place = () => {
    frame = 0;
    const buf = xterm.buffer.active;
    // Not in xterm's public API; without it the caret just stays shown.
    const hidden = (xterm as unknown as { _core?: { coreService?: { isCursorHidden?: boolean } } })._core?.coreService?.isCursorHidden;
    const r = buf.baseY + buf.cursorY - buf.viewportY;
    if (hidden || r < 0 || r >= xterm.rows || !screen.clientWidth) {
      caret.classList.add('gone');
      return;
    }
    caret.classList.remove('gone');
    const cw = screen.clientWidth / xterm.cols;
    const ch = screen.clientHeight / xterm.rows;
    const c = Math.min(buf.cursorX, xterm.cols - 1);
    const h = Math.round(ch * 0.78);
    caret.style.height = `${h}px`;
    const jump = r !== row || Math.abs(c - col) > 12;
    if (r === row && c === col) return;
    if (jump) caret.style.transition = 'none';
    caret.style.transform = `translate(${c * cw - 1}px, ${r * ch + (ch - h) / 2}px)`;
    if (jump) {
      void caret.offsetWidth;
      caret.style.transition = '';
    }
    row = r;
    col = c;
    // Solid while it moves; blinks once it has rested a moment.
    caret.classList.remove('idle');
    caret.classList.add('moving');
    clearTimeout(idle);
    idle = window.setTimeout(() => { caret.classList.remove('moving'); caret.classList.add('idle'); }, 550);
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(place); };
  const subs = [xterm.onCursorMove(schedule), xterm.onRender(schedule), xterm.onScroll(schedule), xterm.onResize(() => { row = -1; schedule(); })];
  const focus = () => caret.classList.remove('blurred');
  const blur = () => caret.classList.add('blurred');
  xterm.textarea?.addEventListener('focus', focus);
  xterm.textarea?.addEventListener('blur', blur);
  if (document.activeElement !== xterm.textarea) blur();
  schedule();
  return () => {
    cancelAnimationFrame(frame);
    clearTimeout(idle);
    subs.forEach((s) => s.dispose());
    xterm.textarea?.removeEventListener('focus', focus);
    xterm.textarea?.removeEventListener('blur', blur);
    caret.remove();
  };
}

/** Everyday commands, after the shell's history and the project's own scripts. */
const COMMON = [
  'git status', 'git diff', 'git add .', 'git commit -m ""', 'git push', 'git pull', 'git checkout ', 'git switch ', 'git log --oneline', 'git stash',
  'npm install', 'npm run dev', 'npm run build', 'npm test', 'npx ', 'cd ..', 'clear', 'code .', 'python ', 'node ',
];
const CASELESS = navigator.userAgent.includes('Windows');
const PATH_SEP = CASELESS ? '\\' : '/';
/** Commands whose argument is a folder, or a file or folder: completed from disk. */
const FOLDER_CMDS = new Set(['cd', 'sl', 'set-location', 'pushd', 'chdir']);
const FILE_CMDS = new Set(['ls', 'dir', 'gci', 'get-childitem', 'cat', 'type', 'gc', 'get-content', 'rm', 'del', 'remove-item', 'rmdir', 'rd', 'cp', 'copy', 'mv', 'move', 'code', 'vim', 'nvim', 'nano', 'notepad', 'less', 'more', 'head', 'tail', 'start', 'explorer', 'ii', 'invoke-item', 'open', 'node', 'python', 'py']);
/** What the user ran in the panel, newest last; kept across restarts. */
const RAN_KEY = 'scout.terminal.ran';

function loadRan(): string[] {
  try { return JSON.parse(localStorage.getItem(RAN_KEY) ?? '[]'); } catch { return []; }
}

function saveRan(cmd: string) {
  try {
    const list = loadRan().filter((c) => c !== cmd);
    list.push(cmd);
    localStorage.setItem(RAN_KEY, JSON.stringify(list.slice(-500)));
  } catch { /* storage off: this run's history only */ }
}

/**
 * Suggestions as you type, fish-style: the rest of the most recent matching command, in dim text after the cursor.
 * → or End takes all of it, Ctrl+→ the next word, Esc hides it. Tab stays the shell's own completion.
 * The shells mark where their prompt ends (OSC 133;B, from terminal/init.*), which is how the panel knows what's been
 * typed; without a mark there are no suggestions.
 */
function attachSuggest(xterm: XTerm, cwd: string) {
  const screen = xterm.element!.querySelector('.xterm-screen') as HTMLElement;
  const ghost = document.createElement('div');
  ghost.className = 'term-ghost';
  screen.appendChild(ghost);
  let history: string[] = [];
  let project: string[] = [];
  let ran = loadRan();
  invoke<{ history: string[]; project: string[] }>('terminal_suggestions', { cwd }).then((s) => { history = s.history; project = s.project; schedule(); }).catch(() => {});

  let mark: ReturnType<XTerm['registerMarker']> | undefined;
  let markX = 0;
  const osc = xterm.parser.registerOscHandler(133, (data) => {
    if (data.startsWith('B')) {
      mark?.dispose();
      mark = xterm.registerMarker(0);
      markX = xterm.buffer.active.cursorX;
    }
    return true;
  });
  // The shell's folder, from the OSC 7 it sends with each prompt ("file:///C:/x" or "file://host/home/x").
  let here = cwd;
  const osc7 = xterm.parser.registerOscHandler(7, (data) => {
    const m = data.match(/^file:\/\/[^/]*(\/.*)$/);
    if (m) {
      let p = m[1];
      try { p = decodeURIComponent(p); } catch { /* sent unencoded */ }
      here = /^\/[A-Za-z]:/.test(p) ? p.slice(1) : p;
    }
    return true;
  });

  // Folder listings, briefly cached; null while one is being read (a redraw follows when it arrives).
  const listings = new Map<string, { at: number; entries: [string, boolean][] | null }>();
  const listing = (dir: string) => {
    const c = listings.get(dir);
    if (c && Date.now() - c.at < 3000) return c.entries;
    listings.set(dir, { at: Date.now(), entries: c?.entries ?? null });
    invoke<[string, boolean][]>('terminal_list', { dir }).then((entries) => { listings.set(dir, { at: Date.now(), entries }); schedule(); }).catch(() => {});
    return c?.entries ?? null;
  };
  const split = (word: string) => {
    const i = Math.max(word.lastIndexOf('/'), word.lastIndexOf('\\'));
    const part = word.slice(0, i + 1);
    const dir = !part ? here : /^([A-Za-z]:[\\/]|[\\/]|~)/.test(part) ? part : `${here.replace(/[\\/]+$/, '')}/${part}`;
    return { dir, name: word.slice(i + 1) };
  };
  const same = (a: string, b: string) => (CASELESS ? a.toLowerCase() === b.toLowerCase() : a === b);
  /** The rest of a path being typed, from what's really on disk: '' for none, null while the folder is read. */
  const completePath = (word: string, foldersOnly: boolean): string | null => {
    const { dir, name } = split(word);
    if (!name || name === '.' || name === '..') return '';
    const entries = listing(dir);
    if (!entries) return null;
    const n = CASELESS ? name.toLowerCase() : name;
    for (const [e, isDir] of entries) {
      if ((foldersOnly && !isDir) || (e.startsWith('.') && !name.startsWith('.')) || /\s/.test(e)) continue;
      if ((CASELESS ? e.toLowerCase() : e).startsWith(n)) return e.slice(name.length) + (isDir ? PATH_SEP : '');
    }
    return '';
  };
  /** Whether a remembered command's path is there from where the shell is now: null while finding out. */
  const pathThere = (word: string, foldersOnly: boolean): boolean | null => {
    const w = word.replace(/[\\/]+$/, '');
    if (!w || w === '.' || w === '..' || w === '~' || w === '-' || /^[A-Za-z]:$/.test(w)) return true;
    const { dir, name } = split(w);
    if (name === '..' || name === '.') return true;
    const entries = listing(dir);
    if (!entries) return null;
    return entries.some(([e, isDir]) => same(e, name) && (!foldersOnly || isDir));
  };
  /** Folder or file commands: how their arguments are completed and checked. */
  const pathKind = (cmd: string) => (FOLDER_CMDS.has(cmd.toLowerCase()) ? 'folder' : FILE_CMDS.has(cmd.toLowerCase()) ? 'file' : null);
  /** A command from history only counts when its path still makes sense here. */
  const fits = (c: string) => {
    const words = c.trim().split(/\s+/);
    const kind = pathKind(words[0]);
    if (!kind || /["'`$]/.test(c)) return true;
    const arg = words.slice(1).find((w) => !w.startsWith('-'));
    return !arg || pathThere(arg, kind === 'folder') === true;
  };

  /** What's been typed at the prompt, split at the cursor; null when not at a prompt. */
  const input = () => {
    const buf = xterm.buffer.active;
    if (buf.type !== 'normal' || !mark || mark.isDisposed || mark.line < 0) return null;
    const cur = buf.baseY + buf.cursorY;
    if (cur < mark.line || (cur === mark.line && buf.cursorX < markX)) return null;
    let before = '';
    for (let y = mark.line; y <= cur; y++) {
      const line = buf.getLine(y);
      if (!line || (y > mark.line && !line.isWrapped)) return null;
      before += line.translateToString(false, y === mark.line ? markX : 0, y === cur ? buf.cursorX : undefined);
    }
    let after = buf.getLine(cur)?.translateToString(true, buf.cursorX) ?? '';
    for (let y = cur + 1; buf.getLine(y)?.isWrapped; y++) after += buf.getLine(y)!.translateToString(true);
    return { before, after: after.trimEnd() };
  };

  let rest = '';
  let hiddenFor: string | null = null;
  let frame = 0;
  const find = (typed: string) => {
    if (!typed.trim()) return '';
    // Typing a path after cd, cat, code...: it comes from the folder, never from history.
    if (!/["'`$]/.test(typed)) {
      const words = typed.trimStart().split(/\s+/);
      const kind = pathKind(words[0]);
      if (kind && words.length >= 2) {
        const last = words[words.length - 1];
        return last.startsWith('-') ? '' : completePath(last, kind === 'folder') ?? '';
      }
    }
    const t = CASELESS ? typed.toLowerCase() : typed;
    const match = (c: string) => c.length > typed.length && (CASELESS ? c.toLowerCase() : c).startsWith(t) && fits(c);
    for (const list of [ran, history]) {
      for (let i = list.length - 1; i >= 0; i--) if (match(list[i])) return list[i].slice(typed.length);
    }
    for (const c of [...project, ...COMMON]) if (match(c)) return c.slice(typed.length);
    return '';
  };
  const place = () => {
    frame = 0;
    const got = input();
    const buf = xterm.buffer.active;
    const r = buf.baseY + buf.cursorY - buf.viewportY;
    rest = got && !got.after && got.before !== hiddenFor ? find(got.before) : '';
    if (got && got.before !== hiddenFor) hiddenFor = null;
    // Only what fits on the cursor's row.
    rest = rest.slice(0, Math.max(0, xterm.cols - buf.cursorX - 1));
    if (!rest || r < 0 || r >= xterm.rows || !screen.clientWidth) {
      rest = '';
      ghost.textContent = '';
      return;
    }
    const cw = screen.clientWidth / xterm.cols;
    const ch = screen.clientHeight / xterm.rows;
    ghost.textContent = rest;
    ghost.style.font = `${xterm.options.fontSize}px ${xterm.options.fontFamily}`;
    ghost.style.lineHeight = `${ch}px`;
    ghost.style.letterSpacing = `${cw - charWidth(ghost.style.font)}px`;
    ghost.style.transform = `translate(${buf.cursorX * cw}px, ${r * ch}px)`;
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(place); };
  const subs = [xterm.onCursorMove(schedule), xterm.onRender(schedule), xterm.onScroll(schedule), xterm.onResize(schedule)];

  /** Called first with every key: true when it was a suggestion's key and has been handled. */
  const key = (e: KeyboardEvent) => {
    if (e.type !== 'keydown') return false;
    if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.altKey) {
      const got = input();
      const cmd = got ? (got.before + got.after).trim() : '';
      if (cmd) { saveRan(cmd); ran = loadRan(); if (!ran.includes(cmd)) ran.push(cmd); }
      return false;
    }
    if (!rest) return false;
    const plain = !e.shiftKey && !e.altKey && !e.metaKey;
    let take = '';
    if (plain && !e.ctrlKey && (e.key === 'ArrowRight' || e.key === 'End')) take = rest;
    else if (plain && e.ctrlKey && e.key === 'ArrowRight') take = rest.match(/^\s*\S+/)?.[0] ?? rest;
    else if (e.key === 'Escape') { hiddenFor = input()?.before ?? null; schedule(); return true; }
    if (!take) return false;
    e.preventDefault();
    invoke('terminal_write', { data: take });
    return true;
  };

  return {
    key,
    dispose: () => {
      cancelAnimationFrame(frame);
      subs.forEach((s) => s.dispose());
      osc.dispose();
      osc7.dispose();
      mark?.dispose();
      ghost.remove();
    },
  };
}

const widths = new Map<string, number>();
/** How wide one character of the terminal's font is, to space the suggestion like xterm spaces its cells. */
function charWidth(font: string) {
  let w = widths.get(font);
  if (w === undefined) {
    const ctx = document.createElement('canvas').getContext('2d')!;
    ctx.font = font;
    w = ctx.measureText('W').width;
    widths.set(font, w);
  }
  return w;
}

/** Stays mounted while the panel is open, so the shell's screen survives switching tabs. */
function TerminalTab({ cwd, visible }: { cwd: string; visible: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<{ xterm: XTerm; fit: FitAddon } | null>(null);
  const [exited, setExited] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shell, setShell] = useState('');
  const [run, setRun] = useState(0);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;

  useEffect(() => {
    const xterm = new XTerm({
      theme: terminalTheme(), allowTransparency: true, fontFamily: TERM_FONT, fontSize: TERM_SIZE, lineHeight: TERM_LINE,
      scrollback: 10000, cursorStyle: 'bar', cursorWidth: 1, cursorBlink: false, cursorInactiveStyle: 'none',
      smoothScrollDuration: 0, macOptionIsMeta: true, allowProposedApi: true,
    });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.open(host.current!);
    term.current = { xterm, fit };
    fit.fit();
    const detachCaret = attachCaret(xterm);
    const suggest = attachSuggest(xterm, cwd);
    // Ctrl+C copies a selection (or interrupts without one) and Ctrl+V pastes.
    xterm.attachCustomKeyEventHandler((e) => {
      if (suggest.key(e)) return false;
      if (e.type !== 'keydown' || !e.ctrlKey || e.shiftKey || e.altKey) return true;
      if (e.key === 'c' && xterm.hasSelection()) {
        navigator.clipboard.writeText(xterm.getSelection());
        xterm.clearSelection();
        return false;
      }
      if (e.key === 'v') {
        e.preventDefault();
        navigator.clipboard.readText().then((t) => xterm.paste(t));
        return false;
      }
      // Ctrl+` belongs to the window.
      return e.key !== '`';
    });
    const data = xterm.onData((d) => invoke('terminal_write', { data: d }));
    const size = xterm.onResize(({ rows, cols }) => invoke('terminal_resize', { rows, cols }));
    const output = new Channel<ArrayBuffer>();
    output.onmessage = (bytes) => xterm.write(new Uint8Array(bytes));
    const done = new Channel<boolean>();
    let current = true;
    const began = Date.now();
    done.onmessage = () => {
      if (!current) return;
      // A shell that dies at once didn't start properly: say so, rather than starting it over and over.
      if (Date.now() - began < 1500) {
        setExited(true);
        return;
      }
      // `exit` closes the terminal, like closing a terminal window; a fresh shell is ready for next time.
      if (visibleRef.current) ui.set({ dockOpen: false });
      setRun((r) => r + 1);
    };
    setExited(false);
    setError(null);
    invoke<string>('terminal_start', { cwd, rows: xterm.rows, cols: xterm.cols, output, exited: done })
      .then((name) => { if (current) setShell(name); })
      .catch((e) => { if (current) setError(`The shell didn't start: ${String(e).replace(/\s*\(os error \d+\)$/, '')}`); });
    const observer = new ResizeObserver(() => { if (host.current?.offsetWidth) fit.fit(); });
    observer.observe(host.current!);
    const unsubscribe = subscribeTheme(() => { xterm.options.theme = terminalTheme(); });
    if (visible) xterm.focus();
    return () => {
      current = false;
      unsubscribe();
      observer.disconnect();
      detachCaret();
      suggest.dispose();
      data.dispose();
      size.dispose();
      invoke('terminal_stop');
      xterm.dispose();
    };
  }, [cwd, run]);

  useEffect(() => {
    if (visible) {
      term.current?.fit.fit();
      term.current?.xterm.focus();
    }
  }, [visible]);

  const restart = () => setRun(run + 1);
  // Pointing at Restart starts the next shell, so clicking it is instant without keeping a spare running all along.
  const warm = () => { const x = term.current?.xterm; if (x) invoke('terminal_prewarm', { cwd, rows: x.rows, cols: x.cols }); };
  return (
    <div className="terminal-tab" style={{ display: visible ? undefined : 'none' }}>
      <div className="terminal-tools">
        {shell && <span className="terminal-shell">{shell === 'pwsh' ? 'PowerShell 7' : shell === 'powershell' ? 'PowerShell' : shell}</span>}
        <IconButton icon="eraser" tip="Clear" size={24} onClick={() => { term.current?.xterm.clear(); term.current?.xterm.focus(); }} />
        <span onMouseEnter={warm}><IconButton icon="refresh" tip="Restart the shell" size={24} onClick={restart} /></span>
      </div>
      <div className="terminal-host" ref={host} onMouseDown={() => term.current?.xterm.focus()} />
      {(exited || error) && (
        <div className="terminal-exit">
          <span className={error ? 'red' : 'muted'}>{error ?? 'The shell has exited.'}</span>
          <button type="button" className="button compact" onMouseEnter={warm} onClick={restart}>Restart</button>
        </div>
      )}
    </div>
  );
}

/** Starts the shell in the background while the panel is still closed, so the first open shows a prompt at once. */
function usePrewarm(cwd: string, started: boolean, width: number) {
  useEffect(() => {
    if (started || !cwd) return;
    const start = () => { const g = terminalGuess(width); invoke('terminal_prewarm', { cwd, ...g }); };
    // After the window has settled: it's a process start, and the panel is rarely opened in the first second.
    const ric = (window as unknown as { requestIdleCallback?: (f: () => void, o: { timeout: number }) => number }).requestIdleCallback;
    const id = ric ? ric(start, { timeout: 1500 }) : window.setTimeout(start, 600);
    return () => { if (ric) (window as unknown as { cancelIdleCallback: (n: number) => void }).cancelIdleCallback(id); else clearTimeout(id); };
  }, [cwd, started]);
}

// ---------- changes ----------

type Change = { path: string; from: string | null; status: string; counts: [number, number] | null };

const STATUS_CLASS: Record<string, string> = { A: 'green', U: 'green', D: 'red', R: 'blue' };

function ChangeRow({ c, repo }: { c: Change; repo: string }) {
  const [diff, setDiff] = useState<DiffLines | string | null>(null);
  const [open, setOpen] = useState(false);
  const toggle = () => {
    setOpen(!open);
    if (!open) invoke<DiffLines>('changes_diff', { repo, path: c.path, from: c.from, status: c.status }).then(setDiff, (e) => setDiff(String(e)));
  };
  const name = c.path.split('/').pop();
  const dir = c.path.slice(0, c.path.length - (name?.length ?? 0));
  return (
    <div>
      <button type="button" className="change-row" title={c.from ? `${c.from} → ${c.path}` : c.path} onClick={toggle} onDoubleClick={() => {
        if (c.status !== 'D') ui.set({ openFile: `${repo}/${c.path}`.replaceAll('/', '\\'), dockTab: 'files' });
      }}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={13} />
        <span className={`change-status mono ${STATUS_CLASS[c.status] ?? 'accent'}`}>{c.status}</span>
        <span className="ellipsis grow"><span className="strong">{name}</span> <span className="tiny dim">{dir}</span></span>
        {c.counts && <span className="mono tiny nowrap"><span className="green">+{c.counts[0]}</span> <span className="red">-{c.counts[1]}</span></span>}
      </button>
      {open && <div className="change-diff">{diff === null ? <Spinner /> : typeof diff === 'string' ? <div className="small dim">{diff}</div> : <div className="diff"><DiffBody lines={diff} /></div>}</div>}
    </div>
  );
}

function ChangesTab({ cwd }: { cwd: string }) {
  const epoch = useView('filesEpoch');
  const [state, setState] = useState<{ repo: string; files: Change[] } | string | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let stale = false;
    invoke<{ repo: string; files: Change[] }>('changes_load', { cwd }).then((r) => { if (!stale) setState(r); }, (e) => { if (!stale) setState(String(e)); });
    return () => { stale = true; };
  }, [cwd, epoch, tick]);
  return (
    <div className="dock-page">
      <div className="dock-actions">
        <span className="small dim grow">{state && typeof state !== 'string' ? `${state.files.length} changed` : ''}</span>
        <IconButton icon="refresh" tip="Refresh" size={26} onClick={() => setTick(tick + 1)} />
      </div>
      <div className="dock-scroll">
        {state === null && <Spinner />}
        {typeof state === 'string' && <div className="placeholder">{state}</div>}
        {state && typeof state !== 'string' && state.files.length === 0 && <div className="placeholder">No changes since the last commit.</div>}
        {state && typeof state !== 'string' && state.files.map((c) => <ChangeRow key={c.path} c={c} repo={state.repo} />)}
      </div>
    </div>
  );
}

// ---------- files ----------

type Node = { name: string; path: string; dir: boolean };

function TreeFolder({ dir, depth, epoch, active, onOpen }: { dir: string; depth: number; epoch: number; active: string | null; onOpen: (p: string) => void }) {
  const [nodes, setNodes] = useState<Node[] | null>(null);
  const [open, setOpen] = useState<Set<string>>(new Set());
  useEffect(() => { invoke<Node[]>('dir_read', { dir }).then(setNodes); }, [dir, epoch]);
  return (
    <>
      {nodes?.map((n) => (
        <div key={n.path}>
          <button type="button" className={`tree-row ${active === n.path ? 'selected' : ''}`} style={{ paddingLeft: 8 + depth * 14 }} title={n.path} onClick={() => {
            if (!n.dir) return onOpen(n.path);
            const next = new Set(open);
            if (next.has(n.path)) next.delete(n.path); else next.add(n.path);
            setOpen(next);
          }}>
            {n.dir ? <Icon name={open.has(n.path) ? 'chevronDown' : 'chevronRight'} size={13} /> : <span className="tree-space" />}
            <Icon name={n.dir ? 'folder' : 'file'} size={14} />
            <span className="ellipsis">{n.name}</span>
          </button>
          {n.dir && open.has(n.path) && <TreeFolder dir={n.path} depth={depth + 1} epoch={epoch} active={active} onOpen={onOpen} />}
        </div>
      ))}
    </>
  );
}

const editorTheme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--sidebar)', color: 'var(--text)', fontSize: '12.5px' },
  '.cm-content': { caretColor: 'var(--text)', fontFamily: '"Cascadia Mono", Consolas, monospace' },
  '.cm-gutters': { backgroundColor: 'var(--sidebar)', color: 'var(--faint)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'rgb(255 255 255 / 3%)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: '#ffffff24 !important' },
  '.cm-cursor': { borderLeftColor: 'var(--text)' },
}, { dark: true });

const highlight = syntaxHighlighting(HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier], color: '#f5a524' },
  { tag: [tags.string, tags.special(tags.string)], color: '#4ac27d' },
  { tag: [tags.comment], color: '#767f8c', fontStyle: 'italic' },
  { tag: [tags.number, tags.bool, tags.null], color: '#f0c074' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: '#4d8fff' },
  { tag: [tags.typeName, tags.className], color: '#56b6c2' },
  { tag: [tags.heading], color: '#f3f4f6', fontWeight: 'bold' },
  { tag: [tags.link], color: '#f5a524', textDecoration: 'underline' },
]));

type OpenFile = { path: string; saved: string; crlf: boolean; mtime: number | null; blocked: string | null };

function FilesTab({ cwd }: { cwd: string }) {
  const epoch = useView('filesEpoch');
  const request = useUi((u) => u.openFile);
  const dirty = useUi((u) => u.editorDirty);
  const [file, setFile] = useState<OpenFile | null>(null);
  const [tick, setTick] = useState(0);
  const [pending, setPending] = useState<{ open: string | null } | null>(null);
  const [changedOnDisk, setChangedOnDisk] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [showTree, setShowTree] = useState(false);
  const [wide, setWide] = useState(true);
  const page = useRef<HTMLDivElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const fileRef = useRef(file);
  fileRef.current = file;

  useEffect(() => {
    const observer = new ResizeObserver(() => setWide((page.current?.offsetWidth ?? 0) >= 620));
    observer.observe(page.current!);
    return () => observer.disconnect();
  }, []);

  const save = async () => {
    const f = fileRef.current;
    if (!f || !view.current || f.blocked) return false;
    const text = view.current.state.doc.toString();
    try {
      const mtime = await invoke<number | null>('file_save', { path: f.path, text, crlf: f.crlf });
      setFile({ ...f, saved: text, mtime });
      setSaveError(null);
      setChangedOnDisk(false);
      ui.set({ editorDirty: false });
      return true;
    } catch (e) {
      setSaveError(String(e));
      return false;
    }
  };
  const saveRef = useRef(save);
  saveRef.current = save;

  const load = async (path: string) => {
    const r = await invoke<{ text: string; crlf: boolean; mtime: number | null; blocked: string | null }>('file_read', { path });
    setFile({ path, saved: r.text, crlf: r.crlf, mtime: r.mtime, blocked: r.blocked });
    setChangedOnDisk(false);
    setSaveError(null);
    ui.set({ editorDirty: false });
    if (!r.blocked) setEditor(path, r.text);
  };

  /** The line a file link asked for, shown once the file is in the editor. */
  const wantLine = useRef<number | null>(null);
  const jump = () => {
    const v = view.current;
    const n = wantLine.current;
    if (!v || !n) return;
    wantLine.current = null;
    const line = v.state.doc.line(Math.min(Math.max(1, n), v.state.doc.lines));
    v.dispatch({ selection: { anchor: line.from, head: line.to }, effects: EditorView.scrollIntoView(line.from, { y: 'center' }) });
    v.focus();
  };

  const language = useRef(new Compartment());
  const setEditor = (path: string, text: string) => {
    view.current?.destroy();
    view.current = null;
    requestAnimationFrame(async () => {
      if (!host.current) return;
      view.current?.destroy();
      view.current = new EditorView({
        parent: host.current,
        state: EditorState.create({
          doc: text,
          extensions: [
            keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => { saveRef.current(); return true; } }]),
            basicSetup,
            editorTheme,
            highlight,
            language.current.of([]),
            EditorView.updateListener.of((u) => {
              if (u.docChanged) ui.set({ editorDirty: u.state.doc.toString() !== fileRef.current?.saved });
            }),
          ],
        }),
      });
      jump();
      const desc = LanguageDescription.matchFilename(languages, path.split(/[\\/]/).pop() ?? '');
      const support = await desc?.load();
      if (support) view.current?.dispatch({ effects: language.current.reconfigure(support) });
    });
  };

  const open = (path: string) => {
    setShowTree(false);
    if (file?.path.toLowerCase() === path.toLowerCase()) return jump();
    if (dirty) setPending({ open: path });
    else load(path);
  };
  const close = () => {
    if (dirty) return setPending({ open: null });
    finish(null);
  };
  const finish = (path: string | null) => {
    setPending(null);
    if (path) return void load(path);
    view.current?.destroy();
    view.current = null;
    setFile(null);
    ui.set({ editorDirty: false });
  };

  useEffect(() => {
    if (request) {
      wantLine.current = ui.get().openLine;
      ui.set({ openFile: null, openLine: null });
      open(request);
    }
  }, [request]); // eslint-disable-line

  // Files may have changed under us: the open file reloads if it has no unsaved edits.
  useEffect(() => {
    const f = fileRef.current;
    if (!f) return;
    invoke<number | null>('file_mtime', { path: f.path }).then((m) => {
      if (m === f.mtime && !f.blocked) return;
      if (ui.get().editorDirty) {
        setChangedOnDisk(true);
        setFile({ ...f, mtime: m });
      } else load(f.path);
    });
  }, [epoch, tick]); // eslint-disable-line

  useEffect(() => () => view.current?.destroy(), []);

  const treeShown = !file || wide || showTree;
  const editorShown = !!file && (wide || !showTree);
  const tree = (
    <div className="file-tree" style={file && wide ? { width: 240 } : undefined}>
      <TreeFolder dir={cwd} depth={0} epoch={epoch + tick} active={file?.path ?? null} onOpen={open} />
    </div>
  );
  return (
    <div className="dock-page" ref={page}>
      <div className="dock-actions">
        {file && !wide && <IconButton icon="back" tip={showTree ? 'Back to the file' : 'Show the files'} size={26} onClick={() => setShowTree(!showTree)} />}
        <span className="small ellipsis grow" title={file?.path}>{file ? `${file.path.split(/[\\/]/).pop()}${dirty ? ' •' : ''}` : ''}</span>
        {file && !file.blocked && <button type="button" className="button small-button" disabled={!dirty} onClick={save}>Save</button>}
        <IconButton icon="refresh" tip="Refresh" size={26} onClick={() => setTick(tick + 1)} />
        {file && <IconButton icon="close" tip="Close the file" size={26} onClick={close} />}
      </div>
      {changedOnDisk && <div className="file-banner">This file changed on disk.<button type="button" className="text-button" onClick={() => file && load(file.path)}>Reload it</button></div>}
      {saveError && <div className="file-banner red">{saveError}</div>}
      <div className="file-split">
        {treeShown && tree}
        <div className="file-editor" style={{ display: editorShown ? undefined : 'none' }}>
          {file?.blocked ? <div className="placeholder">{file.blocked}</div> : <div className="dock-editor" ref={host} />}
        </div>
      </div>
      {pending && (
        <Modal onClose={() => setPending(null)}>
          <b className="strong big">Save your changes?</b>
          <p className="small dim">{file?.path.split(/[\\/]/).pop()} has edits that aren't saved.</p>
          <div className="row end">
            <button type="button" className="button ghost" onClick={() => setPending(null)}>Cancel</button>
            <button type="button" className="button" onClick={() => finish(pending.open)}>Don't save</button>
            <button type="button" className="button primary" onClick={async () => { if (await save()) finish(pending.open); else setPending(null); }}>Save</button>
          </div>
        </Modal>
      )}
    </div>
  );
}

// ---------- the panel ----------

const TABS: [DockTab, string, string][] = [['terminal', 'Terminal', 'terminal'], ['changes', 'Changes', 'changes'], ['files', 'Files', 'files'], ['review', 'Review', 'shield']];
/** Only while the experimental games are on and set to show in the panel. */
const GAMES_TAB: [DockTab, string, string] = ['games', 'Games', 'game'];

export function Dock() {
  const open = useUi((u) => u.dockOpen);
  const { games } = useView('cfg');
  const gamesTab = games.on && games.place === 'panel';
  const picked = useUi((u) => u.dockTab);
  const tab = picked === 'games' && !gamesTab ? 'terminal' : picked;
  const { cwd } = useView('chat');
  const [width, setWidth] = useState(560);
  const [termStarted, setTermStarted] = useState(false);
  useEffect(() => {
    if (open && tab === 'terminal') setTermStarted(true);
  }, [open, tab]);
  usePrewarm(cwd, termStarted, width);
  // Leave the chat at least a readable column.
  const max = Math.max(320, window.innerWidth - 380);
  return (
    <aside className="dock" style={{ width: Math.min(width, max), display: open ? undefined : 'none' }}>
      <Resizer side="left" width={Math.min(width, max)} min={320} max={max} onChange={setWidth} />
      <div className="dock-tabs">
        {[...TABS, ...(gamesTab ? [GAMES_TAB] : [])].map(([id, title, icon]) => (
          <button key={id} type="button" className={`dock-tab ${tab === id ? 'on' : ''}`} onClick={() => ui.set({ dockTab: id })}><Icon name={icon} size={14} />{title}</button>
        ))}
      </div>
      {termStarted && <TerminalTab cwd={cwd} visible={open && tab === 'terminal'} />}
      {open && tab === 'changes' && <ChangesTab cwd={cwd} />}
      {open && tab === 'games' && <GamesTab />}
      {open && tab === 'review' && <ReviewTab />}
      {/* Kept mounted once used: it may hold unsaved edits. */}
      <div className="dock-keep" style={{ display: tab === 'files' ? undefined : 'none' }}><FilesTab cwd={cwd} /></div>
    </aside>
  );
}
