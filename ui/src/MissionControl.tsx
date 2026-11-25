// IDE (formerly Mission Control): the project in a full code editor, opened from the Code page's "Switch to IDE". An activity bar (files, search, source
// control), editor tabs that keep their edits while you move between them, a terminal panel, quick open (Ctrl+P), a
// command palette (Ctrl+Shift+P) and a status bar. One CodeMirror view shows whichever tab is in front; each tab keeps
// its own editor state, so undo history, selection and scroll survive switching.
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { basicSetup, EditorView } from 'codemirror';
import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { LanguageDescription } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { act, invoke } from './api';
import { Composer } from './Composer';
import { Transcript } from './Transcript';
import { ChangesTab, editorTheme, highlight, TerminalSession } from './Dock';
import { Resizer } from './Sidebar';
import { createStore, focusComposer, ui, useUi, useView } from './store';
import { Icon, IconButton, Modal, Spinner } from './widgets';
import './mission.css';

const WIN = navigator.userAgent.includes('Windows');
/** The same file, however its path was spelt: Windows paths ignore case and take either slash. */
const keyOf = (p: string) => (WIN ? p.replaceAll('/', '\\').toLowerCase() : p);
const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;
const sepOf = (root: string) => (root.includes('\\') ? '\\' : '/');
const join = (root: string, rel: string) => `${root.replace(/[\\/]+$/, '')}${sepOf(root)}${rel.replaceAll('/', sepOf(root))}`;
/** `path` relative to `root`, with `/`, or the whole path when it's somewhere else. */
const relTo = (root: string, path: string) => {
  const r = keyOf(root.replace(/[\\/]+$/, '')) + (WIN ? '\\' : '/');
  return keyOf(path).startsWith(r) ? path.slice(r.length).replaceAll('\\', '/') : path;
};

type Side = 'explorer' | 'search' | 'scm';
type Tab = { path: string; saved: string; crlf: boolean; mtime: number | null; blocked: string | null; dirty: boolean; lang: string; stale: boolean };
type Node = { name: string; path: string; dir: boolean };

const SIDE_KEY = 'scout:missionLayout';
function savedLayout(): { side: Side | null; sideWidth: number; panel: boolean; panelHeight: number; agent: boolean; agentWidth: number } {
  try {
    const v = JSON.parse(localStorage.getItem(SIDE_KEY) ?? '{}');
    return {
      side: ['explorer', 'search', 'scm', null].includes(v.side) ? v.side : 'explorer',
      sideWidth: Number(v.sideWidth) >= 180 && Number(v.sideWidth) <= 520 ? Number(v.sideWidth) : 260,
      panel: !!v.panel,
      panelHeight: Number(v.panelHeight) >= 100 && Number(v.panelHeight) <= 800 ? Number(v.panelHeight) : 240,
      agent: v.agent !== false,
      agentWidth: Number(v.agentWidth) >= 300 && Number(v.agentWidth) <= 900 ? Number(v.agentWidth) : 400,
    };
  } catch {
    return { side: 'explorer', sideWidth: 260, panel: false, panelHeight: 240, agent: true, agentWidth: 400 };
  }
}

/** Everything Mission Control shows, kept while the page is closed so tabs and layout come back as they were. */
export const mc = createStore({
  tabs: [] as Tab[],
  active: null as string | null,
  ...savedLayout(),
  expanded: [] as string[],
  palette: null as null | 'files' | 'commands',
  /** A tab with edits the user asked to close: the dialog asks whether to save them. */
  confirmClose: null as string | null,
  terms: [] as number[],
  term: null as number | null,
  error: null as string | null,
  /** Bumped to make the explorer read its folders again. */
  treeEpoch: 0,
  newFile: false,
  searchFocus: 0,
});
mc.subscribe(() => {
  const s = mc.get();
  try { localStorage.setItem(SIDE_KEY, JSON.stringify({ side: s.side, sideWidth: s.sideWidth, panel: s.panel, panelHeight: s.panelHeight, agent: s.agent, agentWidth: s.agentWidth })); } catch {}
});
const useMc = <T,>(pick: (s: ReturnType<typeof mc.get>) => T): T => useSyncExternalStore(mc.subscribe, () => pick(mc.get()));

/** Where the caret is, in its own store: it changes on every key, and only the status bar cares. */
const caret = createStore({ line: 1, col: 1, selected: 0, lines: 1 });

// ---------- editor states ----------

/** Each tab's editor state while it isn't in front. The tab in front lives in `view`. */
const states = new Map<string, EditorState>();
let view: EditorView | null = null;
const language = new Compartment();
const EMPTY = EditorState.create({ doc: '', extensions: [EditorState.readOnly.of(true)] });

const tabOf = (path: string | null) => (path ? mc.get().tabs.find((t) => keyOf(t.path) === keyOf(path)) : undefined);
const isActive = (path: string) => { const a = mc.get().active; return !!a && keyOf(a) === keyOf(path); };
/** The newest state of a tab: the view's when it's in front. */
const stateOf = (path: string) => (view && isActive(path) ? view.state : states.get(keyOf(path)));

function patchTab(path: string, patch: Partial<Tab>) {
  mc.set({ tabs: mc.get().tabs.map((t) => (keyOf(t.path) === keyOf(path) ? { ...t, ...patch } : t)) });
}

function reportCaret(s: EditorState) {
  const sel = s.selection.main;
  const line = s.doc.lineAt(sel.head);
  const next = { line: line.number, col: sel.head - line.from + 1, selected: Math.abs(sel.to - sel.from), lines: s.doc.lines };
  const c = caret.get();
  if (c.line !== next.line || c.col !== next.col || c.selected !== next.selected || c.lines !== next.lines) caret.set(next);
}

async function languageFor(path: string): Promise<{ ext: Extension; name: string }> {
  const desc = LanguageDescription.matchFilename(languages, baseName(path));
  if (!desc) return { ext: [], name: 'Plain Text' };
  try {
    return { ext: await desc.load(), name: desc.name };
  } catch {
    return { ext: [], name: desc.name };
  }
}

function makeState(path: string, text: string, lang: Extension): EditorState {
  return EditorState.create({
    doc: text,
    extensions: [
      keymap.of([{ key: 'Mod-s', preventDefault: true, run: () => { void saveTab(path); return true; } }]),
      basicSetup,
      editorTheme,
      highlight,
      language.of(lang),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          const t = tabOf(path);
          // Compare lengths first: most edits change it, and it's far cheaper than the whole text.
          const dirty = !!t && (u.state.doc.length !== t.saved.length || u.state.doc.toString() !== t.saved);
          if (t && t.dirty !== dirty) patchTab(path, { dirty });
        }
        if (u.docChanged || u.selectionSet) reportCaret(u.state);
      }),
    ],
  });
}

/** Puts `path`'s tab in front, keeping the one that was there. */
function activate(path: string | null) {
  const prev = mc.get().active;
  if (prev && view) states.set(keyOf(prev), view.state);
  mc.set({ active: path });
  if (!view) return;
  const next = path ? states.get(keyOf(path)) : undefined;
  view.setState(next ?? EMPTY);
  reportCaret(view.state);
  if (next && !tabOf(path)?.blocked) requestAnimationFrame(() => view?.focus());
}

function jumpTo(line: number) {
  if (!view) return;
  const doc = view.state.doc;
  const l = doc.line(Math.min(Math.max(1, line), doc.lines));
  view.dispatch({ selection: { anchor: l.from, head: l.to }, effects: EditorView.scrollIntoView(l.from, { y: 'center' }) });
  view.focus();
}

/** Opens a file in its own tab (or goes to its tab), at `line` when given. */
export async function openFile(path: string, line?: number) {
  if (tabOf(path)) {
    activate(tabOf(path)!.path);
    if (line) jumpTo(line);
    return;
  }
  try {
    const [r, lang] = await Promise.all([
      invoke<{ text: string; crlf: boolean; mtime: number | null; blocked: string | null }>('file_read', { path }),
      languageFor(path),
    ]);
    // Opened twice in a row before the first read finished.
    if (tabOf(path)) return activate(tabOf(path)!.path);
    states.set(keyOf(path), makeState(path, r.text, lang.ext));
    const tab: Tab = { path, saved: r.text, crlf: r.crlf, mtime: r.mtime, blocked: r.blocked, dirty: false, lang: lang.name, stale: false };
    // A new tab goes just after the one in front, like an editor's.
    const tabs = mc.get().tabs.slice();
    const at = tabs.findIndex((t) => isActive(t.path));
    tabs.splice(at < 0 ? tabs.length : at + 1, 0, tab);
    mc.set({ tabs, error: null });
    activate(path);
    if (line) jumpTo(line);
  } catch (e) {
    mc.set({ error: `Couldn't open ${baseName(path)}: ${String(e)}` });
  }
}

async function saveTab(path: string): Promise<boolean> {
  const t = tabOf(path);
  const s = stateOf(path);
  if (!t || !s || t.blocked) return false;
  const text = s.doc.toString();
  try {
    const mtime = await invoke<number | null>('file_save', { path: t.path, text, crlf: t.crlf });
    // Typing may have gone on while it saved: it's only clean if the text is still what was written.
    const now = stateOf(path)?.doc.toString() ?? text;
    patchTab(path, { saved: text, mtime, dirty: now !== text, stale: false });
    mc.set({ error: null });
    return true;
  } catch (e) {
    mc.set({ error: String(e) });
    return false;
  }
}

async function saveAll() {
  for (const t of mc.get().tabs) if (t.dirty) await saveTab(t.path);
}

/** Closes a tab, asking first when it has edits (unless `force`). */
function closeTab(path: string, force = false) {
  const t = tabOf(path);
  if (!t) return;
  if (t.dirty && !force) return mc.set({ confirmClose: t.path });
  const tabs = mc.get().tabs;
  const i = tabs.findIndex((x) => keyOf(x.path) === keyOf(path));
  const rest = tabs.filter((_, j) => j !== i);
  const wasActive = isActive(path);
  if (wasActive && view) states.delete(keyOf(path));
  states.delete(keyOf(path));
  if (wasActive) {
    // Take the tab out of the store before showing the next, so activate doesn't keep the closed one's state.
    mc.set({ tabs: rest, active: null, confirmClose: null });
    activate(rest[Math.min(i, rest.length - 1)]?.path ?? null);
  } else {
    mc.set({ tabs: rest, confirmClose: null });
  }
}

function cycleTab(step: number) {
  const { tabs, active } = mc.get();
  if (tabs.length < 2) return;
  const i = tabs.findIndex((t) => active && keyOf(t.path) === keyOf(active));
  activate(tabs[(i + step + tabs.length) % tabs.length].path);
}

/** Reads open files that changed on disk again; a tab with edits is marked instead of losing them. */
async function checkDisk() {
  for (const t of mc.get().tabs) {
    const m = await invoke<number | null>('file_mtime', { path: t.path }).catch(() => t.mtime);
    if (m === t.mtime) continue;
    const now = tabOf(t.path);
    if (!now) continue;
    if (now.dirty) { patchTab(t.path, { stale: true, mtime: m }); continue; }
    await reloadTab(t.path);
  }
}

async function reloadTab(path: string) {
  const t = tabOf(path);
  if (!t) return;
  const r = await invoke<{ text: string; crlf: boolean; mtime: number | null; blocked: string | null }>('file_read', { path }).catch(() => null);
  if (!r) return;
  const lang = await languageFor(path);
  const s = makeState(path, r.text, lang.ext);
  if (view && isActive(path)) {
    view.setState(s);
    reportCaret(s);
  } else {
    states.set(keyOf(path), s);
  }
  patchTab(path, { saved: r.text, crlf: r.crlf, mtime: r.mtime, blocked: r.blocked, dirty: false, stale: false });
}

function toggleTerminal() {
  const { panel, terms } = mc.get();
  if (panel) return mc.set({ panel: false });
  if (!terms.length) return newTerminal();
  mc.set({ panel: true });
}

/** Mission Control's shells are numbered from 9001, apart from the panel's. */
let nextTerm = 9001;
function newTerminal() {
  const id = nextTerm++;
  mc.set({ terms: [...mc.get().terms, id], term: id, panel: true });
}
function closeTerminal(id: number) {
  const terms = mc.get().terms.filter((t) => t !== id);
  mc.set({ terms, term: mc.get().term === id ? terms[terms.length - 1] ?? null : mc.get().term, panel: terms.length > 0 && mc.get().panel });
}

/** Hands the selection (or the file) to Scout: quoted in the message box of the Scout panel. */
function askScout(root: string) {
  const t = tabOf(mc.get().active);
  if (!t || !view) return;
  const s = view.state;
  const sel = s.selection.main;
  const rel = relTo(root, t.path);
  let text: string;
  if (sel.empty) {
    text = `In \`${rel}\`: `;
  } else {
    const a = s.doc.lineAt(sel.from).number;
    const b = s.doc.lineAt(sel.to).number;
    const lang = rel.split('.').pop() ?? '';
    text = `In \`${rel}\` (lines ${a}${b !== a ? `–${b}` : ''}):\n\`\`\`${lang}\n${s.sliceDoc(sel.from, sel.to)}\n\`\`\`\n`;
  }
  const had = ui.get().input;
  // In Mission Control it goes to the Scout panel on the right; from anywhere else, to the chat.
  if (ui.get().page === 'mission') mc.set({ agent: true });
  else ui.set({ page: 'chat' });
  ui.set({ input: had ? `${had}\n${text}` : text });
  requestAnimationFrame(focusComposer);
}

// ---------- explorer ----------

const dirCache = new Map<string, Node[]>();

function Folder({ dir, depth, root }: { dir: string; depth: number; root: string }) {
  const epoch = useMc((s) => s.treeEpoch);
  const filesEpoch = useView('filesEpoch');
  const expanded = useMc((s) => s.expanded);
  const active = useMc((s) => s.active);
  const dirty = useMc((s) => s.tabs);
  const [nodes, setNodes] = useState<Node[] | null>(() => dirCache.get(dir) ?? null);
  useEffect(() => {
    let stale = false;
    invoke<Node[]>('dir_read', { dir }).then((n) => { if (stale) return; dirCache.set(dir, n); setNodes(n); }, () => { if (!stale) setNodes([]); });
    return () => { stale = true; };
  }, [dir, epoch, filesEpoch]);
  const open = useMemo(() => new Set(expanded.map(keyOf)), [expanded]);
  const unsaved = useMemo(() => new Set(dirty.filter((t) => t.dirty).map((t) => keyOf(t.path))), [dirty]);
  if (nodes === null) return depth === 0 ? <div className="mc-pad"><Spinner size={14} /></div> : null;
  if (depth === 0 && nodes.length === 0) return <div className="mc-empty small dim">This folder is empty.</div>;
  return (
    <>
      {nodes.map((n) => {
        const k = keyOf(n.path);
        const isOpen = n.dir && open.has(k);
        return (
          <div key={n.path}>
            <button
              type="button"
              className={`mc-tree-row ${!n.dir && active && keyOf(active) === k ? 'selected' : ''}`}
              style={{ paddingLeft: 8 + depth * 12 }}
              title={relTo(root, n.path)}
              onClick={() => {
                if (!n.dir) return void openFile(n.path);
                const e = mc.get().expanded;
                mc.set({ expanded: isOpen ? e.filter((x) => keyOf(x) !== k) : [...e, n.path] });
              }}
            >
              {n.dir ? <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} /> : <span className="mc-tree-space" />}
              <FileIcon name={n.name} dir={n.dir} open={isOpen} />
              <span className="ellipsis grow">{n.name}</span>
              {unsaved.has(k) && <span className="mc-dot" />}
            </button>
            {isOpen && <Folder dir={n.path} depth={depth + 1} root={root} />}
          </div>
        );
      })}
    </>
  );
}

/** A small colored badge for a file's kind, like an editor's file icons. */
const KINDS: [RegExp, string, string][] = [
  [/\.(tsx?|mts|cts)$/i, 'TS', '#4d8fff'], [/\.(jsx?|mjs|cjs)$/i, 'JS', '#e5c07b'], [/\.rs$/i, 'RS', '#e3794b'],
  [/\.py$/i, 'PY', '#4ac27d'], [/\.(css|scss|less)$/i, '#', '#c678dd'], [/\.html?$/i, '<>', '#e86464'], [/\.json$/i, '{}', '#e5c07b'],
  [/\.(md|mdx)$/i, 'M', '#56b6c2'], [/\.(toml|ya?ml|ini)$/i, '⚙', '#a3a3a3'], [/\.go$/i, 'GO', '#56b6c2'], [/\.(c|h|cpp|hpp|cc)$/i, 'C', '#4d8fff'],
  [/\.(sh|ps1|bat|cmd)$/i, '$', '#4fbf7f'], [/\.astro$/i, 'A', '#f5a524'], [/\.(png|jpe?g|gif|svg|webp|ico)$/i, '▣', '#c678dd'],
];
function FileIcon({ name, dir, open }: { name: string; dir: boolean; open?: boolean }) {
  if (dir) return <span className="mc-ficon dir"><Icon name={open ? 'folderOpen' : 'folder'} size={13} /></span>;
  const kind = KINDS.find(([re]) => re.test(name));
  return kind ? <span className="mc-ficon" style={{ color: kind[2] }}>{kind[1]}</span> : <span className="mc-ficon"><Icon name="file" size={13} /></span>;
}

function NewFileRow({ root }: { root: string }) {
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const done = () => mc.set({ newFile: false });
  const create = async () => {
    const rel = name.trim().replace(/^[\\/]+/, '');
    if (!rel) return done();
    const path = join(root, rel);
    try {
      if ((await invoke<number | null>('file_mtime', { path })) !== null) return setError('A file with that name is already there.');
      await invoke('file_save', { path, text: '', crlf: WIN });
      done();
      mc.set({ treeEpoch: mc.get().treeEpoch + 1 });
      void openFile(path);
    } catch (e) {
      setError(String(e).includes('os error 3') || String(e).includes('No such file') ? 'That folder doesn’t exist yet.' : String(e));
    }
  };
  return (
    <div className="mc-newfile">
      <input
        autoFocus
        className="input"
        placeholder="name.ts or src/name.ts"
        value={name}
        onChange={(e) => { setName(e.target.value); setError(''); }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void create();
          if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); done(); }
        }}
        onBlur={() => { if (!name.trim()) done(); }}
      />
      {error && <div className="tiny red">{error}</div>}
    </div>
  );
}

function Explorer({ root, project }: { root: string; project: string }) {
  const newFile = useMc((s) => s.newFile);
  return (
    <div className="mc-side-body">
      <div className="mc-side-head">
        <span className="grow ellipsis">{project.toUpperCase()}</span>
        <IconButton icon="plus" tip="New file (Ctrl+N)" size={22} onClick={() => mc.set({ newFile: true })} />
        <IconButton icon="refresh" tip="Refresh" size={22} onClick={() => { dirCache.clear(); mc.set({ treeEpoch: mc.get().treeEpoch + 1 }); }} />
        <IconButton icon="chevronUp" tip="Collapse folders" size={22} onClick={() => mc.set({ expanded: [] })} />
      </div>
      {newFile && <NewFileRow root={root} />}
      <div className="mc-scroll">{root ? <Folder dir={root} depth={0} root={root} /> : <div className="mc-empty small dim">Open a project to see its files.</div>}</div>
    </div>
  );
}

// ---------- search ----------

type Hit = { path: string; line: number; text: string };

function Highlighted({ text, query, caseSensitive, regex }: { text: string; query: string; caseSensitive: boolean; regex: boolean }) {
  let re: RegExp | null = null;
  try {
    re = new RegExp(regex ? query : query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), caseSensitive ? '' : 'i');
  } catch { /* shown plain */ }
  const m = re?.exec(text);
  if (!m || !m[0]) return <>{text}</>;
  return <>{text.slice(0, m.index)}<mark>{m[0]}</mark>{text.slice(m.index + m[0].length)}</>;
}

function Search({ root }: { root: string }) {
  const focus = useMc((s) => s.searchFocus);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const [caseSensitive, setCase] = useState(false);
  const [regex, setRegex] = useState(false);
  const [state, setState] = useState<{ hits: Hit[]; busy: boolean; error: string | null }>({ hits: [], busy: false, error: null });
  const [closed, setClosed] = useState<Set<string>>(new Set());
  useEffect(() => { input.current?.focus(); input.current?.select(); }, [focus]);
  useEffect(() => {
    if (!query.trim() || !root) return setState({ hits: [], busy: false, error: null });
    let stale = false;
    setState((s) => ({ ...s, busy: true }));
    // Waits for a pause in typing: each search reads the whole project.
    const t = setTimeout(() => {
      invoke<Hit[]>('files_search', { root, query, case: caseSensitive, regex }).then(
        (hits) => { if (!stale) { setState({ hits, busy: false, error: null }); setClosed(new Set()); } },
        (e) => { if (!stale) setState({ hits: [], busy: false, error: String(e) }); },
      );
    }, 280);
    return () => { stale = true; clearTimeout(t); };
  }, [query, caseSensitive, regex, root]);
  const groups = useMemo(() => {
    const m = new Map<string, Hit[]>();
    for (const h of state.hits) (m.get(h.path) ?? m.set(h.path, []).get(h.path)!).push(h);
    return [...m.entries()];
  }, [state.hits]);
  return (
    <div className="mc-side-body">
      <div className="mc-side-head"><span className="grow">SEARCH</span></div>
      <div className="mc-search-box">
        <input ref={input} className="input" placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
        <button type="button" className={`mc-toggle ${caseSensitive ? 'on' : ''}`} title="Match case" onClick={() => setCase(!caseSensitive)}>Aa</button>
        <button type="button" className={`mc-toggle ${regex ? 'on' : ''}`} title="Use regular expression" onClick={() => setRegex(!regex)}>.*</button>
      </div>
      <div className="mc-search-meta tiny dim">
        {state.busy ? 'Searching…' : state.error ? <span className="red">{state.error}</span> : query.trim() ? `${state.hits.length}${state.hits.length >= 2000 ? '+' : ''} results in ${groups.length} files` : ''}
      </div>
      <div className="mc-scroll">
        {groups.map(([path, hits]) => {
          const shut = closed.has(path);
          return (
            <div key={path}>
              <button type="button" className="mc-tree-row mc-hit-file" onClick={() => { const n = new Set(closed); if (shut) n.delete(path); else n.add(path); setClosed(n); }}>
                <Icon name={shut ? 'chevronRight' : 'chevronDown'} size={12} />
                <FileIcon name={baseName(path)} dir={false} />
                <span className="ellipsis grow"><span className="strong">{baseName(path)}</span> <span className="tiny dim">{path.slice(0, -baseName(path).length)}</span></span>
                <span className="mc-count">{hits.length}</span>
              </button>
              {!shut && hits.map((h) => (
                <button key={h.line} type="button" className="mc-hit" title={`${path}:${h.line}`} onClick={() => void openFile(join(root, path), h.line)}>
                  <span className="ellipsis"><Highlighted text={h.text} query={query} caseSensitive={caseSensitive} regex={regex} /></span>
                  <span className="tiny dim">{h.line}</span>
                </button>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ---------- quick open and commands ----------

let fileList: { root: string; files: string[]; at: number } | null = null;

/** How well `q` matches `s` in order, or -1 when it doesn't: runs, word starts and the file name score higher. */
function fuzzy(q: string, s: string): number {
  const name = s.slice(s.lastIndexOf('/') + 1);
  let qi = 0, score = 0, last = -2;
  for (let i = 0; i < s.length && qi < q.length; i++) {
    if (s[i] !== q[qi]) continue;
    score += i === last + 1 ? 6 : 1;
    if (i === 0 || '/._- '.includes(s[i - 1])) score += 4;
    last = i;
    qi++;
  }
  if (qi < q.length) return -1;
  if (name.startsWith(q)) score += 25; else if (name.includes(q)) score += 12;
  return score - s.length * 0.02;
}

type Command = { id: string; label: string; keys?: string; run: () => void };

function commands(root: string): Command[] {
  const active = mc.get().active;
  return [
    { id: 'files', label: 'Go to File…', keys: 'Ctrl+P', run: () => mc.set({ palette: 'files' }) },
    { id: 'save', label: 'File: Save', keys: 'Ctrl+S', run: () => { if (active) void saveTab(active); } },
    { id: 'saveAll', label: 'File: Save All', run: () => void saveAll() },
    { id: 'new', label: 'File: New File', keys: 'Ctrl+N', run: () => mc.set({ side: 'explorer', newFile: true }) },
    { id: 'close', label: 'View: Close Editor', keys: 'Ctrl+W', run: () => { if (active) closeTab(active); } },
    { id: 'closeAll', label: 'View: Close All Editors', run: () => { for (const t of mc.get().tabs) if (!t.dirty) closeTab(t.path); } },
    { id: 'revert', label: 'File: Revert File', run: () => { if (active) void reloadTab(active); } },
    { id: 'explorer', label: 'View: Show Explorer', keys: 'Ctrl+Shift+E', run: () => mc.set({ side: 'explorer' }) },
    { id: 'search', label: 'View: Search in Files', keys: 'Ctrl+Shift+F', run: () => mc.set({ side: 'search', searchFocus: mc.get().searchFocus + 1 }) },
    { id: 'scm', label: 'View: Show Source Control', keys: 'Ctrl+Shift+G', run: () => mc.set({ side: 'scm' }) },
    { id: 'sidebar', label: 'View: Toggle Side Bar', keys: 'Ctrl+B', run: () => mc.set({ side: mc.get().side ? null : 'explorer' }) },
    { id: 'terminal', label: 'View: Toggle Terminal', keys: 'Ctrl+`', run: toggleTerminal },
    { id: 'newTerminal', label: 'Terminal: New Terminal', keys: 'Ctrl+Shift+`', run: newTerminal },
    { id: 'line', label: 'Go to Line…', keys: 'Ctrl+G', run: () => mc.set({ palette: 'files' }) },
    { id: 'ask', label: 'Scout: Ask About the Selection', keys: 'Ctrl+L', run: () => askScout(root) },
    { id: 'agent', label: 'Scout: Toggle the Scout Panel', keys: 'Ctrl+Alt+B', run: () => mc.set({ agent: !mc.get().agent }) },
    { id: 'newChat', label: 'Scout: New Chat', run: () => { mc.set({ agent: true }); act('newChat'); requestAnimationFrame(focusComposer); } },
    { id: 'agents', label: 'IDE: Back to Agents', run: () => ui.set({ page: 'agent' }) },
  ];
}

function Palette({ root }: { root: string }) {
  const mode = useMc((s) => s.palette);
  const [q, setQ] = useState(mode === 'commands' ? '>' : '');
  const [sel, setSel] = useState(0);
  const [files, setFiles] = useState<string[] | null>(fileList && fileList.root === root ? fileList.files : null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    // The list is read again when it's older than a few seconds: files come and go while agents work.
    if (fileList && fileList.root === root && Date.now() - fileList.at < 8000) return;
    let stale = false;
    invoke<string[]>('files_list', { root }).then((f) => { fileList = { root, files: f, at: Date.now() }; if (!stale) setFiles(f); }, () => { if (!stale) setFiles([]); });
    return () => { stale = true; };
  }, [root]);
  const close = () => { mc.set({ palette: null }); requestAnimationFrame(() => view?.focus()); };
  const isCommand = q.startsWith('>');
  const lineOnly = /^:\d+$/.test(q.trim());
  const [fileQ, lineQ] = (() => { const m = /^(.*?):(\d+)$/.exec(q.trim()); return m ? [m[1], Number(m[2])] : [q.trim(), 0]; })();
  const rows = useMemo((): { label: ReactNode; detail?: string; keys?: string; run: () => void }[] => {
    if (isCommand) {
      const cq = q.slice(1).trim().toLowerCase();
      return commands(root).filter((c) => !cq || fuzzy(cq, c.label.toLowerCase()) >= 0).map((c) => ({ label: c.label, keys: c.keys, run: c.run }));
    }
    if (lineOnly) return [{ label: `Go to line ${q.trim().slice(1)}`, run: () => jumpTo(Number(q.trim().slice(1))) }];
    const fq = fileQ.toLowerCase().replaceAll('\\', '/');
    const all = files ?? [];
    let picked: string[];
    if (!fq) {
      // Nothing typed: the open tabs first, then the project from the top.
      const open = mc.get().tabs.map((t) => relTo(root, t.path));
      picked = [...open, ...all.filter((f) => !open.includes(f))].slice(0, 60);
    } else {
      const scored: [number, string][] = [];
      for (const f of all) {
        const s = fuzzy(fq, f.toLowerCase());
        if (s >= 0) scored.push([s, f]);
      }
      scored.sort((a, b) => b[0] - a[0]);
      picked = scored.slice(0, 60).map(([, f]) => f);
    }
    return picked.map((f) => ({
      label: <><FileIcon name={baseName(f)} dir={false} /><span className="strong">{baseName(f)}</span></>,
      detail: f.slice(0, -baseName(f).length).replace(/\/$/, ''),
      run: () => void openFile(/^([a-z]:[\\/]|\/)/i.test(f) ? f : join(root, f), lineQ || undefined),
    }));
  }, [q, files, root]); // eslint-disable-line
  useEffect(() => setSel(0), [q]);
  useEffect(() => { list.current?.querySelector('.mc-pal-row.on')?.scrollIntoView({ block: 'nearest' }); }, [sel]);
  const pick = (i: number) => { const r = rows[i]; if (!r) return; close(); r.run(); };
  return (
    <div className="mc-palette-back" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="mc-palette">
        <input
          autoFocus
          className="input"
          value={q}
          placeholder={isCommand ? 'Type a command' : 'Search files by name (add :line to go to a line, > for commands)'}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
            else if (e.key === 'ArrowDown') { e.preventDefault(); setSel(Math.min(rows.length - 1, sel + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setSel(Math.max(0, sel - 1)); }
            else if (e.key === 'Enter') { e.preventDefault(); pick(sel); }
          }}
        />
        <div className="mc-pal-list" ref={list}>
          {!isCommand && !lineOnly && files === null && <div className="mc-pad"><Spinner size={14} /></div>}
          {rows.map((r, i) => (
            <button key={i} type="button" className={`mc-pal-row ${i === sel ? 'on' : ''}`} onMouseMove={() => i !== sel && setSel(i)} onClick={() => pick(i)}>
              <span className="mc-pal-label">{r.label}</span>
              {r.detail && <span className="tiny dim ellipsis">{r.detail}</span>}
              <span className="grow" />
              {r.keys && <kbd>{r.keys}</kbd>}
            </button>
          ))}
          {rows.length === 0 && files !== null && <div className="mc-empty small dim">No matches.</div>}
        </div>
      </div>
    </div>
  );
}

// ---------- the editor ----------

function TabStrip({ root }: { root: string }) {
  const tabs = useMc((s) => s.tabs);
  const active = useMc((s) => s.active);
  const strip = useRef<HTMLDivElement>(null);
  useEffect(() => { strip.current?.querySelector('.mc-tab.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' }); }, [active]);
  // Two tabs with the same name say which folder each is in.
  const names = new Map<string, number>();
  for (const t of tabs) names.set(baseName(t.path), (names.get(baseName(t.path)) ?? 0) + 1);
  return (
    <div className="mc-tabs" ref={strip} onWheel={(e) => { if (strip.current && e.deltaY) strip.current.scrollLeft += e.deltaY; }}>
      {tabs.map((t) => {
        const on = !!active && keyOf(active) === keyOf(t.path);
        const rel = relTo(root, t.path);
        const dir = rel.split('/').slice(-2, -1)[0];
        return (
          <div
            key={t.path}
            className={`mc-tab ${on ? 'on' : ''} ${t.dirty ? 'dirty' : ''}`}
            title={rel}
            onMouseDown={(e) => { if (e.button === 1) { e.preventDefault(); closeTab(t.path); } else if (e.button === 0) activate(t.path); }}
          >
            <FileIcon name={baseName(t.path)} dir={false} />
            <span className="ellipsis">{baseName(t.path)}</span>
            {(names.get(baseName(t.path)) ?? 0) > 1 && dir && <span className="tiny dim">{dir}</span>}
            <button type="button" className="mc-tab-x" title={t.dirty ? 'Unsaved: close' : 'Close (Ctrl+W)'} onMouseDown={(e) => e.stopPropagation()} onClick={() => closeTab(t.path)}>
              <span className="mc-dot" />
              <Icon name="close" size={12} />
            </button>
          </div>
        );
      })}
    </div>
  );
}

function Breadcrumbs({ root }: { root: string }) {
  const active = useMc((s) => s.active);
  const tab = useMc((s) => tabOf(s.active));
  if (!active) return null;
  const parts = relTo(root, active).split('/');
  return (
    <div className="mc-crumbs">
      {parts.map((p, i) => (
        <span key={i} className="row">
          {i > 0 && <Icon name="chevronRight" size={11} />}
          <span className={i === parts.length - 1 ? 'strong' : ''}>{p}</span>
        </span>
      ))}
      <span className="grow" />
      {tab && !tab.blocked && (
        <>
          <button type="button" className="mc-crumb-btn" title="Ask Scout about the selection or this file (Ctrl+L)" onClick={() => askScout(root)}><Icon name="sparkle" size={13} />Ask Scout</button>
          {tab.dirty && <button type="button" className="mc-crumb-btn" title="Save (Ctrl+S)" onClick={() => void saveTab(tab.path)}>Save</button>}
        </>
      )}
    </div>
  );
}

function Welcome() {
  const keys: [string, string][] = [
    ['Go to file', 'Ctrl+P'], ['Command palette', 'Ctrl+Shift+P'], ['Search in files', 'Ctrl+Shift+F'], ['New file', 'Ctrl+N'],
    ['Toggle terminal', 'Ctrl+`'], ['Toggle side bar', 'Ctrl+B'], ['Ask Scout about code', 'Ctrl+L'], ['Toggle the Scout panel', 'Ctrl+Alt+B'], ['Back to Code', 'Esc twice'],
  ];
  return (
    <div className="mc-welcome">
      <div className="mc-welcome-mark"><Icon name="mission" size={44} /></div>
      <b className="strong big">Mission Control</b>
      <span className="small dim">Your project in a full editor, with Scout working beside it on the right.</span>
      <div className="mc-keys">
        {keys.map(([label, k]) => <div key={label} className="mc-key-row"><span className="small">{label}</span><kbd>{k}</kbd></div>)}
      </div>
    </div>
  );
}

function Editor({ root }: { root: string }) {
  const host = useRef<HTMLDivElement>(null);
  const tab = useMc((s) => tabOf(s.active));
  const hasTabs = useMc((s) => s.tabs.length > 0);
  useEffect(() => {
    const a = mc.get().active;
    view = new EditorView({ parent: host.current!, state: (a && states.get(keyOf(a))) || EMPTY });
    reportCaret(view.state);
    return () => {
      const now = mc.get().active;
      if (now && view) states.set(keyOf(now), view.state);
      view?.destroy();
      view = null;
    };
  }, []);
  return (
    <div className="mc-editor">
      <TabStrip root={root} />
      {tab && <Breadcrumbs root={root} />}
      {tab?.stale && (
        <div className="file-banner">
          {baseName(tab.path)} changed on disk while you had edits.
          <button type="button" className="text-button" onClick={() => void reloadTab(tab.path)}>Load theirs</button>
          <button type="button" className="text-button" onClick={() => patchTab(tab.path, { stale: false })}>Keep mine</button>
        </div>
      )}
      <div className="mc-code" style={{ display: tab && !tab.blocked ? undefined : 'none' }} ref={host} />
      {tab?.blocked && <div className="placeholder">{tab.blocked}</div>}
      {!hasTabs && <Welcome />}
    </div>
  );
}

// ---------- terminal panel ----------

function Panel({ cwd }: { cwd: string }) {
  const panel = useMc((s) => s.panel);
  const terms = useMc((s) => s.terms);
  const term = useMc((s) => s.term);
  const panelHeight = useMc((s) => s.panelHeight);
  const page = useUi((u) => u.page);
  if (!terms.length) return null;
  return (
    <div className="mc-panel" style={{ height: panelHeight, display: panel ? undefined : 'none' }}>
      <div
        className="mc-panel-resize"
        onMouseDown={(e) => {
          e.preventDefault();
          const start = e.clientY;
          const h = mc.get().panelHeight;
          const move = (m: MouseEvent) => mc.set({ panelHeight: Math.max(100, Math.min(window.innerHeight - 200, h + start - m.clientY)) });
          const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
          window.addEventListener('mousemove', move);
          window.addEventListener('mouseup', up);
        }}
      />
      <div className="mc-panel-head">
        <span className="mc-panel-title">TERMINAL</span>
        <div className="mc-term-tabs">
          {terms.map((id, i) => (
            <button key={id} type="button" className={`mc-term-tab ${id === term ? 'on' : ''}`} onClick={() => mc.set({ term: id })}>
              <Icon name="terminal" size={12} />{WIN ? 'powershell' : 'shell'} {i + 1}
              <span className="mc-term-x" role="button" title="Kill terminal" onClick={(e) => { e.stopPropagation(); closeTerminal(id); }}><Icon name="close" size={11} /></span>
            </button>
          ))}
        </div>
        <span className="grow" />
        <IconButton icon="plus" tip="New terminal (Ctrl+Shift+`)" size={24} onClick={newTerminal} />
        <IconButton icon="chevronDown" tip="Hide the panel (Ctrl+`)" size={24} onClick={() => mc.set({ panel: false })} />
      </div>
      <div className="mc-panel-body">
        {terms.map((id) => (
          <div key={id} className="terminal-tab" style={{ display: id === term ? undefined : 'none' }}>
            <TerminalSession id={id} cwd={cwd} visible={panel && id === term && page === 'mission'} onExit={() => closeTerminal(id)} onFolder={() => {}} />
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------- status bar ----------

function StatusBar({ project }: { project: string }) {
  const tab = useMc((s) => tabOf(s.active));
  const unsaved = useMc((s) => s.tabs.filter((t) => t.dirty).length);
  const error = useMc((s) => s.error);
  const c = useSyncExternalStore(caret.subscribe, caret.get);
  return (
    <footer className="mc-status">
      <button type="button" className="mc-status-item" title="Back to Agents" onClick={() => ui.set({ page: 'agent' })}><Icon name="code" size={12} />Agents</button>
      <span className="mc-status-item"><Icon name="folder" size={12} />{project}</span>
      {unsaved > 0 && <button type="button" className="mc-status-item" title="Save all" onClick={() => void saveAll()}>{unsaved} unsaved</button>}
      {error && <span className="mc-status-item red ellipsis" title={error}>{error}</span>}
      <span className="grow" />
      {tab && !tab.blocked && (
        <>
          <span className="mc-status-item">Ln {c.line}, Col {c.col}{c.selected ? ` (${c.selected} selected)` : ''}</span>
          <span className="mc-status-item">{c.lines} lines</span>
          <span className="mc-status-item">UTF-8</span>
          <span className="mc-status-item">{tab.crlf ? 'CRLF' : 'LF'}</span>
          <span className="mc-status-item">{tab.lang}</span>
        </>
      )}
      <button type="button" className="mc-status-item" title="Toggle terminal (Ctrl+`)" onClick={toggleTerminal}><Icon name="terminal" size={12} /></button>
    </footer>
  );
}

// ---------- Scout, on the right ----------

/** The chat with Scout beside the editor, like an editor's chat panel: the same chat and message box as the agents
 *  page, so a turn started here carries on there and the other way round. */
function AgentPanel() {
  const width = useMc((s) => s.agentWidth);
  const title = useView('chat').title;
  return (
    <aside className="mc-agent" style={{ width }}>
      <Resizer side="left" width={width} min={300} max={Math.max(320, Math.min(900, window.innerWidth - 520))} onChange={(w) => mc.set({ agentWidth: w })} />
      <div className="mc-side-head mc-agent-head">
        <span className="mc-agent-title ellipsis grow" title={title}>SCOUT{title ? <span className="mc-agent-chat"> · {title}</span> : null}</span>
        <IconButton icon="plus" tip="New chat" size={22} onClick={() => { act('newChat'); requestAnimationFrame(focusComposer); }} />
        <IconButton icon="maximize" tip="Open in Code" size={22} onClick={() => ui.set({ page: 'chat' })} />
        <IconButton icon="close" tip="Hide (Ctrl+Alt+B)" size={22} onClick={() => mc.set({ agent: false })} />
      </div>
      <div className="mc-agent-chat-body chat">
        <Transcript />
        <Composer />
      </div>
    </aside>
  );
}

// ---------- the page ----------

function useMissionKeys(root: string) {
  useEffect(() => {
    let lastEsc = 0;
    const key = (e: KeyboardEvent) => {
      const u = ui.get();
      if (u.page !== 'mission' || u.settings || e.defaultPrevented || mc.get().palette || mc.get().confirmClose) return;
      const k = e.key.toLowerCase();
      const ctrl = e.ctrlKey || e.metaKey;
      const run = (f: () => void) => { e.preventDefault(); f(); };
      if (ctrl && e.shiftKey && k === 'p') return run(() => mc.set({ palette: 'commands' }));
      if (ctrl && !e.shiftKey && (k === 'p' || k === 'g')) return run(() => mc.set({ palette: 'files' }));
      if (ctrl && e.shiftKey && k === 'f') return run(() => mc.set({ side: 'search', searchFocus: mc.get().searchFocus + 1 }));
      if (ctrl && e.shiftKey && k === 'e') return run(() => mc.set({ side: 'explorer' }));
      if (ctrl && e.shiftKey && k === 'g') return run(() => mc.set({ side: 'scm' }));
      if (ctrl && !e.shiftKey && k === 'b') return run(() => mc.set({ side: mc.get().side ? null : 'explorer' }));
      if (ctrl && e.shiftKey && (e.key === '`' || e.key === '~')) return run(newTerminal);
      if (ctrl && e.key === '`') return run(toggleTerminal);
      if (ctrl && !e.shiftKey && k === 'n') return run(() => mc.set({ side: 'explorer', newFile: true }));
      if (ctrl && !e.shiftKey && k === 'w') return run(() => { const a = mc.get().active; if (a) closeTab(a); });
      if (ctrl && !e.shiftKey && k === 's') return run(() => { const a = mc.get().active; if (a) void saveTab(a); });
      if (ctrl && k === 'tab') return run(() => cycleTab(e.shiftKey ? -1 : 1));
      if (ctrl && !e.shiftKey && k === 'l') return run(() => askScout(root));
      if (ctrl && e.altKey && k === 'b') return run(() => mc.set({ agent: !mc.get().agent }));
      if (ctrl && e.shiftKey && k === 'i') return run(() => { mc.set({ agent: true }); requestAnimationFrame(focusComposer); });
      // In the Scout panel's message box, as in the chat: Shift+Tab cycles the permission mode.
      if (e.shiftKey && e.key === 'Tab' && document.activeElement?.matches('.composer textarea.prompt')) return run(() => act('cycleMode'));
      if (ctrl && e.altKey && k === 's') return run(() => void saveAll());
      // Esc twice, outside the editor and the terminal, goes back to the agents.
      if (e.key === 'Escape' && !document.activeElement?.closest('.cm-editor, .xterm, input, textarea, .mc-agent')) {
        const now = Date.now();
        if (now - lastEsc < 500) ui.set({ page: 'chat' });
        lastEsc = now;
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [root]);
}

export function MissionControl() {
  const { cwd, project } = useView('chat');
  const filesEpoch = useView('filesEpoch');
  const page = useUi((u) => u.page);
  const side = useMc((s) => s.side);
  const sideWidth = useMc((s) => s.sideWidth);
  const palette = useMc((s) => s.palette);
  const confirm = useMc((s) => s.confirmClose);
  const agent = useMc((s) => s.agent);
  const request = useUi((u) => u.openFile);
  const root = cwd;
  useMissionKeys(root);

  // Files the agents changed, and changes made outside the app when the window comes back.
  useEffect(() => { void checkDisk(); }, [filesEpoch]);
  useEffect(() => {
    const f = () => { if (ui.get().page === 'mission') void checkDisk(); };
    window.addEventListener('focus', f);
    return () => window.removeEventListener('focus', f);
  }, []);
  // A file opened from elsewhere (a double click in Source Control) opens here while Mission Control is in front.
  useEffect(() => {
    if (!request || page !== 'mission') return;
    const line = ui.get().openLine ?? undefined;
    ui.set({ openFile: null, openLine: null });
    void openFile(request, line);
  }, [request, page]);
  // Coming back to the page puts the caret back in the editor.
  useEffect(() => { if (page === 'mission') requestAnimationFrame(() => view?.focus()); }, [page]);

  const items: [Side, string, string][] = [['explorer', 'files', 'Explorer (Ctrl+Shift+E)'], ['search', 'search', 'Search (Ctrl+Shift+F)'], ['scm', 'changes', 'Source Control (Ctrl+Shift+G)']];
  const confirmTab = tabOf(confirm);
  return (
    <div className="mission">
      <div className="mc-body">
        <nav className="mc-activity" aria-label="Mission Control">
          {items.map(([id, icon, tip]) => (
            <button key={id} type="button" className={`mc-act ${side === id ? 'on' : ''}`} title={tip} aria-label={tip} onClick={() => mc.set({ side: side === id ? null : id, ...(id === 'search' ? { searchFocus: mc.get().searchFocus + 1 } : {}) })}>
              <Icon name={icon} size={20} />
            </button>
          ))}
          <button type="button" className="mc-act" title="Command palette (Ctrl+Shift+P)" aria-label="Command palette" onClick={() => mc.set({ palette: 'commands' })}><Icon name="bolt" size={19} /></button>
          <span className="grow" />
          <button type="button" className="mc-act" title="Terminal (Ctrl+`)" aria-label="Terminal" onClick={toggleTerminal}><Icon name="terminal" size={19} /></button>
          <button type="button" className={`mc-act ${agent ? 'on-quiet' : ''}`} title="Scout panel (Ctrl+Alt+B)" aria-label="Scout panel" onClick={() => mc.set({ agent: !agent })}><Icon name="chat" size={19} /></button>
        </nav>
        {side && (
          <aside className="mc-side" style={{ width: sideWidth }}>
            {side === 'explorer' && <Explorer root={root} project={project} />}
            {side === 'search' && <Search root={root} />}
            {side === 'scm' && (
              <div className="mc-side-body">
                <div className="mc-side-head"><span className="grow">SOURCE CONTROL</span></div>
                <div className="mc-scm"><ChangesTab cwd={root} /></div>
                <div className="tiny dim mc-pad">Double-click a file to open it.</div>
              </div>
            )}
            <Resizer side="right" width={sideWidth} min={180} max={520} onChange={(w) => mc.set({ sideWidth: w })} />
          </aside>
        )}
        <div className="mc-main">
          <Editor root={root} />
          <Panel cwd={root} />
        </div>
        {agent && page === 'mission' && <AgentPanel />}
      </div>
      <StatusBar project={project} />
      {palette && <Palette key={palette} root={root} />}
      {confirmTab && (
        <Modal onClose={() => mc.set({ confirmClose: null })}>
          <b className="strong big">Save changes to {baseName(confirmTab.path)}?</b>
          <p className="small dim">Your changes will be lost if you don't save them.</p>
          <div className="row end">
            <button type="button" className="button ghost" onClick={() => mc.set({ confirmClose: null })}>Cancel</button>
            <button type="button" className="button" onClick={() => closeTab(confirmTab.path, true)}>Don't save</button>
            <button type="button" className="button primary" onClick={async () => { if (await saveTab(confirmTab.path)) closeTab(confirmTab.path, true); else mc.set({ confirmClose: null }); }}>Save</button>
          </div>
        </Modal>
      )}
    </div>
  );
}

/** How many open files have edits that aren't saved, for the rail's badge. */
export const useMissionUnsaved = () => useMc((s) => s.tabs.reduce((n, t) => n + (t.dirty ? 1 : 0), 0));
