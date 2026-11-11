import { useEffect, useState } from 'react';
import { act, invoke, usd, type View } from './api';
import { ui, useUi, useView } from './store';
import { Icon, IconButton, MenuItem, Popover, SectionLabel, Spinner } from './widgets';
import { Hideable, useHidden } from './Hideable';

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();
const sameDir = (a: string, b: string) => norm(a) === norm(b);

/** Opens Scout's folder picker; choosing a folder there adds it (see FolderPicker). */
export function addProject() {
  ui.set({ pickingFolder: true });
}

type SessionInfo = View['sessions'][number];

/** The project a chat's folder is listed as, or the folder's own name. */
export function useProjectName() {
  const projects = useView('projects');
  return (cwd: string) => projects.find((p) => sameDir(p.path, cwd))?.name ?? (cwd.replaceAll('\\', '/').replace(/\/+$/, '').split('/').pop() || cwd);
}

/** How a chat is doing, for one that isn't on screen as well as the one that is. */
function SessionState({ s }: { s: SessionInfo }) {
  if (s.waiting) return <span className="session-status waiting" title="Scout needs your answer"><span className="session-dot waiting" />Needs input</span>;
  if (s.running) return <span className="session-status working" title="Scout is working"><Spinner size={11} />Working</span>;
  if (s.unread) return <span className="session-status finished" title="Finished"><span className="session-dot" />New</span>;
  return null;
}

export function SessionRow({ s, current, project }: { s: SessionInfo; current: boolean; project: string }) {
  const busy = s.running || s.waiting;
  return (
    <div className={`session-row two-line ${current ? 'selected' : ''}`} title={`${s.title}\n${s.cwd}`}>
      <button type="button" className="session-open" onClick={() => { ui.set({ page: 'chat' }); act('openChat', { id: s.id }); }}>
        <span className="session-line">
          <span className="session-title ellipsis grow">{s.title}</span>
          <SessionState s={s} />
        </span>
        <span className="session-line sub">
          <span className="ellipsis grow">{project}</span>
          <span className="tiny when">{s.ago}</span>
        </span>
      </button>
      <span className="row-actions">
        {s.done
          ? <IconButton icon="back" tip="Move back to Recent" size={24} onClick={() => act('setDone', { id: s.id, done: false })} />
          : <IconButton icon="check" tip={busy ? 'Scout is still working on this chat' : 'Mark as done'} size={24} disabled={busy} onClick={() => act('setDone', { id: s.id, done: true })} />}
        <Popover width={160} trigger={(_, toggle) => <IconButton icon="more" tip="More" size={24} onClick={toggle} />}>
          {(close) => (
            <>
              {s.done
                ? <MenuItem onClick={() => { close(); act('setDone', { id: s.id, done: false }); }}>Move back to Recent</MenuItem>
                : <MenuItem disabled={busy} onClick={() => { close(); act('setDone', { id: s.id, done: true }); }}>Mark as done</MenuItem>}
              {/* The chat Scout is working in stays until it's done. */}
              <MenuItem disabled={s.running} onClick={() => { close(); ui.set({ deleting: { id: s.id, title: s.title } }); }}>Delete…</MenuItem>
            </>
          )}
        </Popover>
      </span>
    </div>
  );
}

/** Every chat across every project, newest first: Recent, or the ones marked Done. */
function ChatList({ done }: { done: boolean }) {
  const sessions = useView('sessions');
  const chat = useView('chat');
  const projectName = useProjectName();
  const rows = sessions.filter((s) => !!s.done === done);
  return (
    <div className="side-scroll">
      {rows.map((s) => <SessionRow key={s.id} s={s} current={s.id === chat.id} project={projectName(s.cwd)} />)}
      {rows.length === 0 && <div className="small dim no-chats">{done ? 'Nothing marked done yet.' : 'No chats yet. Start one above.'}</div>}
    </div>
  );
}

/** The project folders: start a chat in one, rename it, or add another. */
function ProjectsMenu() {
  const projects = useView('projects');
  const chat = useView('chat');
  const renaming = useUi((u) => u.renaming);
  const [draft, setDraft] = useState('');
  useEffect(() => {
    const p = projects.find((x) => renaming && sameDir(x.path, renaming));
    if (p) setDraft(p.name);
  }, [renaming, projects]);
  const commit = (path: string) => {
    act('renameProject', { path, name: draft });
    ui.set({ renaming: null });
  };
  return (
    <Popover width={240} align="end" trigger={(open, toggle) => <IconButton icon="folder" tip="Projects: new chat in a folder" size={26} active={open} onClick={toggle} />}>
      {(close) => (
        <>
          <div className="tiny dim projects-menu-label">New chat in…</div>
          <div className="projects-menu">
            {projects.map((p) => renaming && sameDir(renaming, p.path) ? (
              <input key={p.path} className="input rename" autoFocus value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => commit(p.path)} onKeyDown={(e) => {
                if (e.key === 'Enter') e.currentTarget.blur();
                if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); ui.set({ renaming: null }); }
              }} />
            ) : (
              <div key={p.path} className={`project-row ${sameDir(p.path, chat.cwd) ? 'here' : ''}`} title={p.path}>
                <button type="button" className="project-open" onClick={() => { close(); ui.set({ page: 'chat' }); act('changeProject', { dir: p.path }); }}>
                  <Icon name="folder" size={15} />
                  <span className="ellipsis">{p.name}</span>
                </button>
                <IconButton icon="pencil" tip="Rename" size={24} className="row-more" onClick={() => ui.set({ renaming: p.path })} />
                {/* The open project stays, so the chat on screen always has one. */}
                {!sameDir(p.path, chat.cwd) && <IconButton icon="close" tip="Remove from list" size={24} className="row-more" onClick={() => act('removeProject', { path: p.path })} />}
              </div>
            ))}
          </div>
          <hr />
          <MenuItem icon="folderPlus" onClick={() => { close(); addProject(); }}>Add a project folder…</MenuItem>
        </>
      )}
    </Popover>
  );
}

/** Chats matching the sidebar search, across every project: titles at once, then text once the search ends. */
function SearchResults({ query }: { query: string }) {
  const sessions = useView('sessions');
  const chat = useView('chat');
  const projectName = useProjectName();
  const [hits, setHits] = useState<{ q: string; ids: Set<string> } | null>(null);
  const q = query.trim();
  useEffect(() => {
    let stale = false;
    const t = setTimeout(() => invoke<string[]>('search_chats', { q }).then((ids) => { if (!stale) setHits({ q, ids: new Set(ids) }); }), 120);
    return () => { stale = true; clearTimeout(t); };
  }, [q]);
  const found = hits?.q === q ? hits.ids : null;
  const rows = sessions.filter((s) => (found ? found.has(s.id) : s.title.toLowerCase().includes(q.toLowerCase())));
  return (
    <>
      <div className="row side-label"><SectionLabel>Results</SectionLabel>{!found && <Spinner size={11} />}</div>
      <div className="side-scroll">
        {rows.map((s) => <SessionRow key={s.id} s={s} current={s.id === chat.id} project={projectName(s.cwd)} />)}
        {rows.length === 0 && found && <div className="small dim no-chats">No chats match.</div>}
      </div>
    </>
  );
}

/** Shown in the sidebar when today's daily login reward can be claimed. */
function DailyRewardBanner() {
  const { hasKey } = useView('cfg');
  const { info, busy } = useView('usage');
  const dr = info?.dailyReward;
  if (!hasKey || !dr?.canClaim) return null;
  return (
    <div className="daily-reward-bar">
      <button type="button" className="daily-reward-info grow min0" onClick={() => ui.set({ settings: 'usage' })} title="Open Usage & rewards">
        <div className="strong small ellipsis">Daily reward · Day {dr.nextDay}</div>
        <div className="tiny dim ellipsis">+{usd(dr.amount)} to your balance</div>
      </button>
      <button
        type="button"
        className="button primary compact"
        disabled={busy}
        onClick={() => act('usageChange', { body: { action: 'claim-daily-reward' }, said: `Claimed ${usd(dr.amount)} daily login reward (day ${dr.nextDay}).` })}
      >
        Claim {usd(dr.amount)}
      </button>
    </div>
  );
}

const WIDTH_KEY = 'scout:sidebarWidth';
/** The search, kept while the sidebar is hidden (hiding it unmounts it). */
let lastSearch = '';
/** Whether the Done list is showing, kept the same way. */
let lastShowDone = false;

export function Sidebar() {
  const [search, setSearchState] = useState(lastSearch);
  const setSearch = (s: string) => { lastSearch = s; setSearchState(s); };
  // The width it was dragged to, across hiding it and restarts.
  const [width, setWidthState] = useState(() => {
    try {
      const w = Number(localStorage.getItem(WIDTH_KEY));
      return w >= 200 && w <= 420 ? w : 264;
    } catch { return 264; }
  });
  const setWidth = (w: number) => {
    setWidthState(w);
    try { localStorage.setItem(WIDTH_KEY, String(w)); } catch {}
  };
  const searching = !!search.trim() && !useHidden().includes('side.search');
  const [showDone, setShowDoneState] = useState(lastShowDone);
  const setShowDone = (v: boolean) => { lastShowDone = v; setShowDoneState(v); };
  const doneCount = useView('sessions').filter((s) => s.done).length;
  return (
    <aside className="sidebar" style={{ width }}>
      <div className="side-head" data-tauri-drag-region>
        <b className="brand" data-tauri-drag-region>Chats</b>
        <span className="grow" data-tauri-drag-region />
        <IconButton icon="sidebar" tip="Hide sidebar" onClick={() => ui.set({ sidebar: false })} />
      </div>
      <Hideable id="side.newChat"><button type="button" className="new-chat" onClick={() => { ui.set({ page: 'chat' }); act('newChat'); }}>
        <Icon name="plus" /><span className="grow">New chat</span><span className="tiny dim">Ctrl+N</span>
      </button></Hideable>
      <Hideable id="side.search">
      <div className="side-search">
        <Icon name="search" />
        <input placeholder="Search chats" value={search} onChange={(e) => setSearch(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape' && search) { e.preventDefault(); e.stopPropagation(); setSearch(''); } }} />
        {search && <IconButton icon="close" tip="Clear" size={24} onClick={() => setSearch('')} />}
      </div>
      </Hideable>
      {searching ? <SearchResults query={search} /> : (
        <>
          <div className="row side-label">
            {showDone
              ? <button type="button" className="side-back" onClick={() => setShowDone(false)}><Icon name="back" size={13} /><SectionLabel>Done</SectionLabel></button>
              : <SectionLabel>Recent</SectionLabel>}
            <span className="grow" />
            <ProjectsMenu />
            <IconButton icon="folderPlus" tip="Add a project folder" size={26} onClick={addProject} />
          </div>
          <ChatList done={showDone} />
          {!showDone && (
            <button type="button" className="done-footer" onClick={() => setShowDone(true)} title="Chats you marked as done">
              <span className="grow">Done</span>
              <span className="dim">{doneCount}</span>
              <Icon name="chevronRight" size={13} />
            </button>
          )}
        </>
      )}
      <DailyRewardBanner />
      <Resizer side="right" width={width} min={200} max={420} onChange={setWidth} />
    </aside>
  );
}

/** Drags a panel's edge. */
export function Resizer({ side, width, min, max, onChange }: { side: 'left' | 'right'; width: number; min: number; max: number; onChange: (w: number) => void }) {
  return (
    <div className={`resizer ${side}`} onMouseDown={(e) => {
      e.preventDefault();
      const start = e.clientX;
      const move = (m: MouseEvent) => onChange(Math.max(min, Math.min(max, width + (side === 'right' ? m.clientX - start : start - m.clientX))));
      const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
    }} />
  );
}
