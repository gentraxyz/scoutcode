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
import { Resizer } from './Sidebar';
import { ui, useUi, useView, type DockTab } from './store';
import { subscribeTheme, themeColor } from './theme';
import { DiffBody, Icon, IconButton, Modal, Spinner } from './widgets';

// ---------- terminal ----------

/** xterm can't read CSS variables, so it takes the theme's colors when it opens and again when the theme changes. */
function terminalTheme() {
  return { background: themeColor('sidebar'), foreground: themeColor('text'), cursor: themeColor('text'), selectionBackground: '#ffffff30', black: themeColor('surface'), brightBlack: themeColor('dim'), red: '#e86464', green: '#4fbf7f', yellow: '#e5c07b', blue: '#5b9cf5', magenta: '#c586c0', cyan: '#56b6c2', white: themeColor('text'), brightWhite: themeColor('strong') };
}

/** Stays mounted while the panel is open, so the shell's screen survives switching tabs. */
function TerminalTab({ cwd, visible }: { cwd: string; visible: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<{ xterm: XTerm; fit: FitAddon } | null>(null);
  const [exited, setExited] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [run, setRun] = useState(0);

  useEffect(() => {
    const xterm = new XTerm({ theme: terminalTheme(), fontFamily: '"Cascadia Mono", Consolas, monospace', fontSize: 13, scrollback: 5000, cursorBlink: true, allowProposedApi: true });
    const fit = new FitAddon();
    xterm.loadAddon(fit);
    xterm.open(host.current!);
    term.current = { xterm, fit };
    fit.fit();
    // Ctrl+C copies a selection (or interrupts without one) and Ctrl+V pastes.
    xterm.attachCustomKeyEventHandler((e) => {
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
    done.onmessage = () => { if (current) setExited(true); };
    setExited(false);
    setError(null);
    invoke('terminal_start', { cwd, rows: xterm.rows, cols: xterm.cols, output, exited: done }).catch((e) => setError(String(e)));
    const observer = new ResizeObserver(() => { if (host.current?.offsetWidth) fit.fit(); });
    observer.observe(host.current!);
    const unsubscribe = subscribeTheme(() => { xterm.options.theme = terminalTheme(); });
    return () => {
      current = false;
      unsubscribe();
      observer.disconnect();
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

  return (
    <div className="terminal-tab" style={{ display: visible ? undefined : 'none' }}>
      <div className="terminal-host" ref={host} />
      {(exited || error) && (
        <div className="terminal-exit">
          <span className="small muted">{error ?? 'The shell has exited.'}</span>
          <button type="button" className="button" onClick={() => setRun(run + 1)}>Restart</button>
        </div>
      )}
    </div>
  );
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
      const desc = LanguageDescription.matchFilename(languages, path.split(/[\\/]/).pop() ?? '');
      const support = await desc?.load();
      if (support) view.current?.dispatch({ effects: language.current.reconfigure(support) });
    });
  };

  const open = (path: string) => {
    setShowTree(false);
    if (file?.path === path) return;
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
      ui.set({ openFile: null });
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

const TABS: [DockTab, string, string][] = [['terminal', 'Terminal', 'terminal'], ['changes', 'Changes', 'changes'], ['files', 'Files', 'files']];
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
      {/* Kept mounted once used: it may hold unsaved edits. */}
      <div className="dock-keep" style={{ display: tab === 'files' ? undefined : 'none' }}><FilesTab cwd={cwd} /></div>
    </aside>
  );
}
