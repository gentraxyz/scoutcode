// The panel's tabs, like a browser's: a strip of open tabs with a + that opens a New Tab page, which lists what the
// panel can show (what used to be buttons beside the window controls) and the agent's background work.
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke, type AgentStep, type Item } from './api';
import { openGame } from './Games';
import { openMusic } from './Music';
import { Markdown } from './Transcript';
import { core, ui, useCore, useUi, useView, type DockTab } from './store';
import { Icon, IconButton, Spinner } from './widgets';

type ToolItem = Extract<Item, { kind: 'tool' }>;
export type Shell = { id: string; command: string; running: boolean; exit: number | null; output: string };

/** What a fixed tab shows: its icon, name and a line for the New Tab page. */
const KINDS: { id: DockTab; icon: string; name: string; about: string; shortcut?: string }[] = [
  { id: 'terminal', icon: 'terminal', name: 'Terminal', about: 'A new shell in the project folder', shortcut: 'Ctrl+`' },
  { id: 'browser', icon: 'globe', name: 'Browser', about: 'A web page beside the chat' },
  { id: 'files', icon: 'files', name: 'Files', about: "The project's files, with an editor" },
  { id: 'changes', icon: 'changes', name: 'Changes', about: 'What changed since the last commit' },
  { id: 'review', icon: 'shield', name: 'Code Review', about: 'A second opinion from another model' },
  { id: 'games', icon: 'game', name: 'Games', about: 'Something to play while Scout works' },
];

export const isTermTab = (t: string): t is 'terminal' | `terminal:${number}` => t === 'terminal' || t.startsWith('terminal:');
let nextTerm = 2;
/** Another terminal: the first is 'terminal', and each one after gets a number. */
const freshTerminal = (): DockTab => (ui.get().dockTabs.includes('terminal') ? `terminal:${nextTerm++}` : 'terminal');
export function setTermFolder(tab: DockTab, dir: string) {
  const f = ui.get().termFolders;
  if (f[tab] !== dir) ui.set({ termFolders: { ...f, [tab]: dir } });
}
const folderName = (d: string) => d.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || 'Terminal';

export const isNewTab = (t: string): t is 'new' | `new:${number}` => t === 'new' || t.startsWith('new:');
let nextNew = 1;

export const isTaskTab = (t: string): t is `task:${number}` => t.startsWith('task:');
export const isShellTab = (t: string): t is `shell:${string}` => t.startsWith('shell:');

/** The sub-agents (the task tool) in the open chat, newest last. */
export function useTasks(): ToolItem[] {
  const items = useCore((s) => s.items);
  return useMemo(() => items.filter((i): i is ToolItem => i.kind === 'tool' && i.name === 'task'), [items]);
}

/** The agent's background shells, polled while something shows them. `lines` of output each, 0 for the list alone. */
export function useShells(lines: number, every = 1500): Shell[] {
  const [shells, setShells] = useState<Shell[]>([]);
  useEffect(() => {
    let live = true;
    const load = () => invoke<Shell[]>('background_shells', { lines }).then((s) => { if (live) setShells(s); }).catch(() => {});
    load();
    const t = window.setInterval(load, every);
    return () => { live = false; clearInterval(t); };
  }, [lines, every]);
  return shells;
}

/** Shows `tab` in the panel, adding it to the strip if it isn't there. */
export function openDockTab(tab: DockTab) {
  const { dockTabs } = ui.get();
  ui.set({ dockOpen: true, dockTab: tab, dockTabs: dockTabs.includes(tab) ? dockTabs : [...dockTabs, tab] });
}

/** Opens another New Tab page, as Ctrl+T does in a browser, even from a New Tab page. */
export function newDockTab() {
  openDockTab(`new:${nextNew++}`);
  focusDock();
}

/** Puts the keyboard in the tab in front, so the next Ctrl+W or Ctrl+Tab acts on the panel. A terminal takes it
 *  itself when it shows; the New Tab page gives it to its first choice. */
function focusDock() {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const dock = document.querySelector<HTMLElement>('.dock');
    if (!dock || dock.contains(document.activeElement)) return;
    (dock.querySelector<HTMLElement>('.newtab-tile') ?? dock).focus({ preventScroll: true });
  }));
}

/** Recently closed tabs, newest last, for Ctrl+Shift+T. */
const closed: DockTab[] = [];

/** Closes a tab; the one beside it takes its place, and the panel closes with its last tab. */
export function closeDockTab(tab: DockTab) {
  const { dockTabs, dockTab } = ui.get();
  const at = dockTabs.indexOf(tab);
  if (at < 0) return;
  if (!isNewTab(tab)) closed.push(tab);
  if (closed.length > 20) closed.shift();
  const rest = dockTabs.filter((t) => t !== tab);
  if (!rest.length) return ui.set({ dockTabs: [], dockOpen: false, dockTab: 'new' });
  ui.set({ dockTabs: rest, ...(dockTab === tab ? { dockTab: rest[Math.min(at, rest.length - 1)] } : {}) });
  focusDock();
}

/** Brings back the last tab closed. A terminal comes back as a fresh shell, and a finished task as its report. */
export function reopenDockTab() {
  while (closed.length) {
    let t = closed.pop()!;
    if (isTaskTab(t) && !core.get().items.some((i) => i.id === Number(t.slice(5)))) continue;
    if (isTermTab(t) && ui.get().dockTabs.includes(t)) t = freshTerminal();
    openDockTab(t);
    return focusDock();
  }
}

/** Moves to the tab `by` places along, wrapping around. */
function cycleDockTab(by: number) {
  const { dockTabs, dockTab } = ui.get();
  if (!dockTabs.length) return;
  const i = dockTabs.indexOf(dockTab);
  ui.set({ dockOpen: true, dockTab: dockTabs[(i + by + dockTabs.length) % dockTabs.length] });
  focusDock();
}

/** Whether the keyboard is in the panel, or nowhere in particular (just after a click on empty space). */
const inDock = () => !!document.activeElement?.closest('.dock');

/**
 * The panel's keys, as in a browser: Ctrl+T new tab, Ctrl+W close the tab in front, Ctrl+Shift+T reopen the last one
 * closed, Ctrl+Tab / Ctrl+Shift+Tab (and Ctrl+PageDown / PageUp) move between tabs, Ctrl+1–9 pick one (9 is the last).
 * All but Ctrl+T need the keyboard in the panel, so they never fight the chat box. True when it handled the key.
 */
export function handleDockKeys(e: KeyboardEvent): boolean {
  if (!e.ctrlKey || e.altKey || e.metaKey) return false;
  const k = e.key.toLowerCase();
  const done = () => { e.preventDefault(); e.stopPropagation(); return true; };
  if (k === 't' && !e.shiftKey) { newDockTab(); return done(); }
  if (k === 't' && e.shiftKey) { reopenDockTab(); return done(); }
  const { dockOpen, dockTab, dockTabs } = ui.get();
  if (!dockOpen || !inDock()) return false;
  if (k === 'w' && !e.shiftKey) { closeDockTab(dockTab); return done(); }
  if (k === 'tab' || k === 'pagedown' || k === 'pageup') { cycleDockTab(k === 'pageup' || (k === 'tab' && e.shiftKey) ? -1 : 1); return done(); }
  if (!e.shiftKey && /^[1-9]$/.test(e.key)) {
    const n = Number(e.key);
    const t = n === 9 ? dockTabs[dockTabs.length - 1] : dockTabs[n - 1];
    if (t) { ui.set({ dockTab: t }); focusDock(); }
    return done();
  }
  return false;
}

/** The keys handleDockKeys takes, which a terminal lets through instead of sending to its shell. */
export function isDockKey(e: KeyboardEvent): boolean {
  if (!e.ctrlKey || e.altKey || e.metaKey) return false;
  const k = e.key.toLowerCase();
  return k === 't' || (k === 'w' && !e.shiftKey) || k === 'tab' || k === 'pagedown' || k === 'pageup' || (!e.shiftKey && /^[1-9]$/.test(e.key));
}

/** On the New Tab page, choosing something turns that tab into it, as a browser's new tab does. One of a kind already
 *  open is shown instead, except terminals, which open as many as you like. */
function navigate(from: DockTab, to: DockTab) {
  if (to === 'terminal') to = freshTerminal();
  const { dockTabs } = ui.get();
  if (dockTabs.includes(to)) ui.set({ dockTabs: dockTabs.filter((t) => t !== from), dockTab: to });
  else ui.set({ dockTabs: dockTabs.map((t) => (t === from ? to : t)), dockTab: to });
  focusDock();
}

function TabTitle({ tab }: { tab: DockTab }) {
  const tasks = useTasks();
  const folder = useUi((u) => u.termFolders[tab]);
  const cwd = useView('chat').cwd;
  if (isTermTab(tab)) return <><Icon name="terminal" size={13} /><span className="ellipsis">{folderName(folder || cwd)}</span></>;
  if (isNewTab(tab)) return <><Icon name="plus" size={13} /><span className="ellipsis">New tab</span></>;
  if (isTaskTab(tab)) {
    const t = tasks.find((x) => x.id === Number(tab.slice(5)));
    return <>{t && !t.result ? <Spinner size={11} /> : <Icon name="sparkle" size={13} />}<span className="ellipsis">{t?.label || 'Task'}</span></>;
  }
  if (isShellTab(tab)) return <><Icon name="terminal" size={13} /><span className="ellipsis">{tab.slice(6)}</span></>;
  const k = KINDS.find((x) => x.id === tab);
  return <><Icon name={k?.icon ?? 'panel'} size={13} /><span className="ellipsis">{k?.name ?? tab}</span></>;
}

/** The strip across the top of the panel. Middle-click closes a tab, and dragging one moves it. */
export function DockTabStrip() {
  const tabs = useUi((u) => u.dockTabs);
  const active = useUi((u) => u.dockTab);
  const drag = useRef<DockTab | null>(null);
  const move = (over: DockTab) => {
    const from = drag.current;
    if (!from || from === over) return;
    const list = ui.get().dockTabs.filter((t) => t !== from);
    list.splice(list.indexOf(over), 0, from);
    ui.set({ dockTabs: list });
  };
  return (
    <div className="dtabs" role="tablist">
      {tabs.map((t) => (
        <div
          key={t}
          role="tab"
          aria-selected={t === active}
          className={`dtab ${t === active ? 'on' : ''}`}
          draggable
          onDragStart={() => { drag.current = t; }}
          onDragEnter={() => move(t)}
          onDragOver={(e) => e.preventDefault()}
          onDragEnd={() => { drag.current = null; }}
          onMouseDown={(e) => { if (e.button === 0) ui.set({ dockTab: t }); }}
          onAuxClick={(e) => { if (e.button === 1) { e.preventDefault(); closeDockTab(t); } }}
          title={isNewTab(t) ? 'New tab' : undefined}
        >
          <TabTitle tab={t} />
          <button type="button" className="dtab-close" aria-label="Close tab" onMouseDown={(e) => e.stopPropagation()} onClick={() => closeDockTab(t)}><Icon name="close" size={10} /></button>
        </div>
      ))}
      <IconButton icon="plus" tip="New tab" size={26} className="dtab-new" onClick={newDockTab} />
    </div>
  );
}

function Tile({ icon, name, about, hint, onClick }: { icon: string; name: string; about: string; hint?: string; onClick: () => void }) {
  return (
    <button type="button" className="newtab-tile" onClick={onClick}>
      <span className="newtab-icon"><Icon name={icon} size={18} /></span>
      <span className="stack" style={{ gap: 1, minWidth: 0 }}>
        <b className="strong small">{name}</b>
        <span className="tiny dim ellipsis">{about}</span>
      </span>
      {hint && <kbd className="newtab-kbd tiny">{hint}</kbd>}
    </button>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="newtab-section">
      <div className="section-label">{title}</div>
      {children}
    </div>
  );
}

/** What a new tab shows: everything the panel can open, then the agent's background work. */
export function NewTabPage({ tab }: { tab: DockTab }) {
  const { games, music } = useView('cfg');
  // ↑ and ↓ move between the choices; Enter opens one.
  const arrows = (e: React.KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const all = [...e.currentTarget.querySelectorAll<HTMLElement>('.newtab-tile, .newtab-row')];
    const i = all.indexOf(document.activeElement as HTMLElement);
    all[(i + (e.key === 'ArrowDown' ? 1 : -1) + all.length) % all.length]?.focus();
    e.preventDefault();
  };
  const tasks = useTasks();
  const shells = useShells(0, 2000);
  const running = tasks.filter((t) => !t.result);
  const runningShells = shells.filter((s) => s.running);
  const kinds = KINDS.filter((k) => k.id !== 'games' || (games.on && games.place === 'panel'));
  return (
    <div className="dock-page newtab" onKeyDown={arrows}>
      <div className="newtab-inner">
        <b className="strong big newtab-head">What do you want to open?</b>
        <div className="newtab-list">
          {kinds.map((k) => <Tile key={k.id} icon={k.icon} name={k.name} about={k.about} hint={k.shortcut} onClick={() => navigate(tab, k.id)} />)}
          {games.on && games.place !== 'panel' && <Tile icon="game" name="Games" about="Opens the floating game card" onClick={() => openGame(games, true)} />}
          {music.on && <Tile icon="music" name="Music" about="Opens the music player" onClick={() => openMusic()} />}
        </div>
        {(running.length > 0 || runningShells.length > 0) && <Section title="Background tasks">
          <div className="newtab-list">
            {[...running].reverse().map((t) => (
              <button key={t.id} type="button" className="newtab-row" onClick={() => navigate(tab, `task:${t.id}`)}>
                {!t.result ? <Spinner size={12} /> : <Icon name={t.result.isError ? 'close' : 'check'} size={13} />}
                <span className="small strong ellipsis grow">{t.label || 'Task'}</span>
                <span className="tiny dim nowrap">Sub-agent · {(t.steps ?? []).filter((s) => s.tool).length} steps</span>
              </button>
            ))}
            {[...runningShells].reverse().map((s) => (
              <button key={s.id} type="button" className="newtab-row" onClick={() => navigate(tab, `shell:${s.id}`)}>
                {s.running ? <Spinner size={12} /> : <Icon name={s.exit === 0 ? 'check' : 'close'} size={13} />}
                <code className="small ellipsis grow">{s.command}</code>
                <span className="tiny dim nowrap">{s.running ? 'Running' : `Exited ${s.exit ?? ''}`}</span>
              </button>
            ))}
          </div>
        </Section>}
      </div>
    </div>
  );
}

function StepRow({ s }: { s: AgentStep }) {
  if (!s.tool) return s.text.trim() ? <div className="task-said small muted"><Markdown text={s.text} /></div> : null;
  return (
    <div className="task-step small">
      {!s.done ? <Spinner size={10} /> : <Icon name={s.isError ? 'close' : 'check'} size={11} />}
      <span className="ellipsis grow">{s.label || s.tool}</span>
      {s.summary && <span className="tiny dim nowrap">{s.summary}</span>}
    </div>
  );
}

/** A sub-agent's work as it happens, and its report once it's done. */
export function TaskTab({ id }: { id: number }) {
  const task = useTasks().find((t) => t.id === id);
  const end = useRef<HTMLDivElement>(null);
  const steps = task?.steps ?? (task?.result?.display.type === 'agent' ? task.result.display.steps : []);
  useEffect(() => { end.current?.scrollIntoView({ block: 'end' }); }, [steps.length]);
  if (!task) return <div className="dock-page newtab-gone small dim">This task belongs to another chat, or the chat was cleared.</div>;
  const answer = task.result?.display.type === 'agent' ? task.result.display.answer : '';
  return (
    <div className="dock-page task-tab">
      <div className="dock-actions">
        {!task.result ? <Spinner size={11} /> : <Icon name={task.result.isError ? 'close' : 'check'} size={12} />}
        <b className="ellipsis grow">{task.label || 'Task'}</b>
        <span className="tiny dim nowrap">{!task.result ? 'Working…' : task.result.isError ? 'Failed' : 'Done'}</span>
      </div>
      <div className="dock-scroll task-body">
        {steps.map((s, i) => <StepRow key={i} s={s} />)}
        {!task.result && task.progress && <div className="tiny dim task-progress">{task.progress}</div>}
        {answer && <div className="task-answer"><div className="section-label">Report</div><Markdown text={answer} /></div>}
        <div ref={end} />
      </div>
    </div>
  );
}

/** A background shell's output, live, with a Stop button while it runs. */
export function ShellTab({ id }: { id: string }) {
  const shell = useShells(400, 1000).find((s) => s.id === id);
  const box = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useEffect(() => { const el = box.current; if (el && stick.current) el.scrollTop = el.scrollHeight; }, [shell?.output]);
  if (!shell) return <div className="dock-page newtab-gone small dim">Loading…</div>;
  return (
    <div className="dock-page shell-tab">
      <div className="dock-actions">
        {shell.running ? <Spinner size={11} /> : <Icon name={shell.exit === 0 ? 'check' : 'close'} size={12} />}
        <code className="ellipsis grow small">{shell.command}</code>
        {shell.running
          ? <button type="button" className="button ghost compact" onClick={() => invoke('stop_background_shell', { id })}>Stop</button>
          : <span className="tiny dim nowrap">Exited {shell.exit ?? ''}</span>}
      </div>
      <pre ref={box} className="shell-out" onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40; }}>{shell.output || 'No output yet.'}</pre>
    </div>
  );
}

/** Running sub-agents get a tab of their own as they start, without opening the panel. */
export function useAutoTaskTabs() {
  const tasks = useTasks();
  const seen = useRef(new Set<number>());
  const chat = useView('chat').id;
  useEffect(() => {
    seen.current = new Set(core.get().items.filter((i) => i.kind === 'tool' && i.name === 'task').map((i) => i.id));
    // A sub-agent's tab belongs to its chat: switching chats closes them.
    const { dockTabs, dockTab } = ui.get();
    const rest = dockTabs.filter((t) => !isTaskTab(t));
    if (rest.length !== dockTabs.length) ui.set({ dockTabs: rest, ...(isTaskTab(dockTab) ? (rest.length ? { dockTab: rest[rest.length - 1] } : { dockOpen: false, dockTab: 'new' }) : {}) });
  }, [chat]);
  useEffect(() => {
    const fresh = tasks.filter((t) => !t.result && !seen.current.has(t.id));
    if (!fresh.length) return;
    fresh.forEach((t) => seen.current.add(t.id));
    const { dockTabs } = ui.get();
    const add = fresh.map((t) => `task:${t.id}` as DockTab).filter((t) => !dockTabs.includes(t));
    if (add.length) ui.set({ dockTabs: [...dockTabs, ...add] });
  }, [tasks]);
}
