import { useEffect, useState } from 'react';
import { act, invoke, openExternal, type View } from './api';
import { ui, useCore, useUi, useView } from './store';
import { Icon, IconButton, MenuItem, Popover, SectionLabel, Spinner } from './widgets';
import { ScoutLogo } from './logo';

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();
const sameDir = (a: string, b: string) => norm(a) === norm(b);

export async function addProject() {
  const dir = await invoke<string | null>('add_project');
  if (dir) ui.set({ renaming: dir });
}

/** How a chat is doing, for one that isn't on screen as well as the one that is. */
function SessionState({ s }: { s: View['sessions'][number] }) {
  if (s.waiting) return <span className="session-dot waiting" title="Scout needs your answer" />;
  if (s.running) return <span className="session-state" title="Scout is working"><Spinner size={11} /></span>;
  if (s.unread) return <span className="session-dot" title="Finished" />;
  return null;
}

function SessionRow({ s, current, tip }: { s: View['sessions'][number]; current: boolean; tip?: string }) {
  return (
    <div className={`session-row ${current ? 'selected' : ''}`} title={tip}>
      <button type="button" className="session-open" onClick={() => act('openChat', { id: s.id })}>
        <SessionState s={s} />
        <span className="ellipsis grow">{s.title}</span>
        <span className="tiny dim when">{s.ago}</span>
      </button>
      <Popover width={150} trigger={(_, toggle) => <IconButton icon="more" tip="More" size={24} className="row-more" onClick={toggle} />}>
        {(close) => (
          // The chat Scout is working in stays until it's done.
          <MenuItem disabled={s.running} onClick={() => { close(); ui.set({ deleting: { id: s.id, title: s.title } }); }}>Delete…</MenuItem>
        )}
      </Popover>
    </div>
  );
}

function Projects() {
  const projects = useView('projects');
  const sessions = useView('sessions');
  const chat = useView('chat');
  const renaming = useUi((u) => u.renaming);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [draft, setDraft] = useState('');
  // A project is shown open when Scout enters it.
  useEffect(() => {
    setExpanded((e) => (e.has(norm(chat.cwd)) ? e : new Set(e).add(norm(chat.cwd))));
  }, [chat.cwd]);
  useEffect(() => {
    const p = projects.find((x) => renaming && sameDir(x.path, renaming));
    if (p) setDraft(p.name);
  }, [renaming, projects]);
  const commit = (path: string) => {
    act('renameProject', { path, name: draft });
    ui.set({ renaming: null });
  };
  return (
    <div className="side-scroll">
      {projects.map((p) => {
        const here = sameDir(p.path, chat.cwd);
        const open = expanded.has(norm(p.path));
        const list = sessions.filter((s) => sameDir(s.cwd, p.path));
        return (
          <div key={p.path}>
            {renaming && sameDir(renaming, p.path) ? (
              <input className="input rename" autoFocus value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => commit(p.path)} onKeyDown={(e) => {
                if (e.key === 'Enter') e.currentTarget.blur();
                if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); ui.set({ renaming: null }); }
              }} />
            ) : (
              <div className={`project-row ${here ? 'here' : ''}`} title={p.path}>
                <button type="button" className="project-open" onClick={() => {
                  const next = new Set(expanded);
                  if (open) next.delete(norm(p.path)); else next.add(norm(p.path));
                  setExpanded(next);
                }}>
                  <Icon name={open ? 'chevronDown' : 'chevronRight'} size={13} />
                  <Icon name="folder" size={15} />
                  <span className="ellipsis">{p.name}</span>
                </button>
                <IconButton icon="plus" tip="New chat here" size={24} className="row-more" onClick={() => act('changeProject', { dir: p.path })} />
                <Popover width={170} trigger={(_, toggle) => <IconButton icon="more" tip="More" size={24} className="row-more" onClick={toggle} />}>
                  {(close) => (
                    <>
                      <MenuItem onClick={() => { close(); act('changeProject', { dir: p.path }); }}>New chat</MenuItem>
                      <MenuItem onClick={() => { close(); ui.set({ renaming: p.path }); }}>Rename</MenuItem>
                      {/* The open project stays, so the chat on screen always has one. */}
                      {!here && <MenuItem onClick={() => { close(); act('removeProject', { path: p.path }); }}>Remove from list</MenuItem>}
                    </>
                  )}
                </Popover>
              </div>
            )}
            {open && (
              <div className="project-chats">
                {list.map((s) => <SessionRow key={s.id} s={s} current={s.id === chat.id} />)}
                {list.length === 0 && <div className="small dim no-chats">No chats yet</div>}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** Chats matching the sidebar search, across every project: titles at once, then text once the search ends. */
function SearchResults({ query }: { query: string }) {
  const sessions = useView('sessions');
  const projects = useView('projects');
  const chat = useView('chat');
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
        {rows.map((s) => <SessionRow key={s.id} s={s} current={s.id === chat.id} tip={projects.find((p) => sameDir(p.path, s.cwd))?.name} />)}
        {rows.length === 0 && found && <div className="small dim no-chats">No chats match.</div>}
      </div>
    </>
  );
}

function Profile() {
  const cfg = useView('cfg');
  const { additionalProviders } = useCore((s) => s.statics);
  const linked = additionalProviders.find((a) => a.id === cfg.additional[0]?.id)?.name;
  const name = cfg.username ?? (cfg.browserSignIn ? 'Signed in' : cfg.hasKey ? 'API key' : cfg.openrouterKeys.length ? 'OpenRouter key' : cfg.nvidiaKey ? 'NVIDIA key' : cfg.claude ? 'Claude Code' : cfg.codex ? 'Codex' : cfg.opencode ? 'OpenCode' : cfg.grokSignedIn || cfg.grokKey ? 'Grok Build' : cfg.hermesSignedIn || cfg.hermesKey ? 'Hermes' : linked ?? (cfg.guest ? 'No account' : 'Kilo Code'));
  const { version } = useView('updater');
  return (
    <div className="profile">
      <Popover side="top" className="profile-menu" width="anchor" trigger={(open, toggle) => (
        <button type="button" className={`profile-row ${open ? 'open' : ''}`} onClick={toggle}>
          <span className="avatar">{name.charAt(0).toUpperCase()}</span>
          <span className="ellipsis grow">{name}</span>
          <Icon name={open ? 'chevronDown' : 'chevronUp'} size={14} />
        </button>
      )}>
        {(close) => (
          <>
            <div className="profile-head">
              <span className="avatar big">{name.charAt(0).toUpperCase()}</span>
              <div className="min0">
                <div className="strong ellipsis">{name}</div>
                <div className="tiny dim">Scout {version}</div>
              </div>
            </div>
            <hr />
            <MenuItem icon="gear" onClick={() => { close(); ui.set({ settings: 'notifications' }); }}>Settings</MenuItem>
            <MenuItem icon="chart" onClick={() => { close(); ui.set({ settings: 'usage' }); }}>Usage</MenuItem>
            <hr />
            <MenuItem icon="download" onClick={() => { close(); ui.set({ settings: 'updates' }); act('checkUpdates'); }}>Check for updates</MenuItem>
            <MenuItem icon="user" onClick={() => { close(); openExternal(`${cfg.baseUrl.replace(/\/v1$/, '')}/account`); }}>Account page</MenuItem>
          </>
        )}
      </Popover>
    </div>
  );
}

export function Sidebar() {
  const [search, setSearch] = useState('');
  const [width, setWidth] = useState(264);
  return (
    <aside className="sidebar" style={{ width }}>
      <div className="side-head" data-tauri-drag-region>
        <ScoutLogo size={24} drag />
        <b className="brand" data-tauri-drag-region>Scout</b>
        <span className="grow" data-tauri-drag-region />
        <IconButton icon="sidebar" tip="Hide sidebar" onClick={() => ui.set({ sidebar: false })} />
      </div>
      <button type="button" className="new-chat" onClick={() => act('newChat')}>
        <Icon name="plus" /><span className="grow">New chat</span><span className="tiny dim">Ctrl+N</span>
      </button>
      <div className="side-search">
        <Icon name="search" />
        <input placeholder="Search chats" value={search} onChange={(e) => setSearch(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape' && search) { e.preventDefault(); e.stopPropagation(); setSearch(''); } }} />
        {search && <IconButton icon="close" tip="Clear" size={24} onClick={() => setSearch('')} />}
      </div>
      {search.trim() ? <SearchResults query={search} /> : (
        <>
          <div className="row side-label">
            <SectionLabel>Projects</SectionLabel>
            <span className="grow" />
            <IconButton icon="plus" tip="Add a project folder" size={24} onClick={addProject} />
          </div>
          <Projects />
        </>
      )}
      <Profile />
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
