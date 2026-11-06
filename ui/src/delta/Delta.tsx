/**
 * ScoutCode Delta: an issue tracker for teams, in the desktop app and on the website. Teams have issues with a workflow,
 * priorities, labels and assignees, shown as a list or a board; projects group issues across teams and sit on a roadmap;
 * cycles are a team's sprints. Any issue can be handed to Scout, which works on it in one of the user's projects.
 *
 * This file is the frame (workspaces, the navigation, the issue list and board, new issues, the command palette);
 * IssueDetail.tsx is one issue, and views.tsx the projects, roadmap, cycles and settings pages. The host app passes a
 * Host (sync.ts) that says where workspaces can be kept and how to reach Scout.
 */
import { createContext, Fragment, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore, type DragEvent, type ReactNode } from 'react';
import { cycleName, cycleStatus, findByKey, isDone, issueKey, newId, PRIORITIES, PROJECT_STATUSES, SCOUT, STATE_TYPES, teamStates, today, type Issue, type StateType } from './model';
import { createWorkspace, listWorkspaces, message, Session, type Host, type Storage, type WorkspaceInfo } from './sync';
import { issueMenu } from './menus';
import { Avatar, ContextMenuHost, LightboxHost, Icon, LabelDot, openContextMenu, labelOptions, Menu, MenuItem, Modal, peopleOptions, personName, Picker, PriorityIcon, priorityOptions, shortDay, StateIcon, stateOptions, useLinger } from './ui';
import { IssueDetail } from './IssueDetail';
import { CycleView, CyclesView, ProjectsView, ProjectView, RoadmapView, SettingsView } from './views';
import { DocTree, DocsView, DocView, newPage } from './Docs';
import { imageHandlers } from './images';
import { pageText, pageTitle } from './model';
import './delta.css';

export type Route =
  | { view: 'my' }
  | { view: 'team'; teamId: string; tab?: 'all' | 'active' | 'backlog' }
  | { view: 'cycles'; teamId: string }
  | { view: 'cycle'; id: string }
  | { view: 'projects' }
  | { view: 'project'; id: string }
  | { view: 'roadmap' }
  | { view: 'docs' }
  | { view: 'doc'; id: string }
  | { view: 'settings' };

export interface DeltaCtx {
  s: Session;
  host: Host;
  /** Bumped on every change, for memos. */
  v: number;
  route: Route;
  go: (r: Route) => void;
  open: (id: string | null) => void;
  create: (defaults?: Partial<Issue>) => void;
}

const Ctx = createContext<DeltaCtx | null>(null);
export const useDelta = () => useContext(Ctx)!;

const LAST = 'delta.workspace';
const remember = (k: string, v: string | null) => {
  try {
    if (v == null) localStorage.removeItem(k);
    else localStorage.setItem(k, v);
  } catch {}
};
const recall = (k: string) => {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
};

const isTyping = (t: EventTarget | null) => {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable);
};

// ---------- the app ----------

export function Delta({ host, onExit }: { host: Host; onExit?: () => void }) {
  const [list, setList] = useState<WorkspaceInfo[] | null>(null);
  const [cloudError, setCloudError] = useState<string | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [making, setMaking] = useState(false);

  const refresh = useCallback(async () => {
    const r = await listWorkspaces(host);
    setList(r.list);
    setCloudError(r.cloudError);
    return r.list;
  }, [host]);

  const openWs = useCallback(async (info: WorkspaceInfo) => {
    setOpening(info.id);
    setError(null);
    try {
      const s = await Session.open(host, info);
      setSession((old) => {
        old?.close();
        return s;
      });
      remember(LAST, `${info.storage}:${info.id}`);
    } catch (e) {
      setError(message(e));
    } finally {
      setOpening(null);
    }
  }, [host]);

  useEffect(() => {
    refresh().then((l) => {
      const last = recall(LAST);
      const pick = l.find((w) => `${w.storage}:${w.id}` === last) ?? l[0];
      if (pick) openWs(pick);
    });
  }, [refresh, openWs]);
  useEffect(() => () => session?.close(), [session]);

  if (list === null || (opening && !session)) return <div className="delta d-center"><DeltaMark size={48} loading /></div>;
  if (!session || making || !list.length) {
    return (
      <div className="delta d-center d-scroll">
        <Welcome
          host={host}
          first={!list.length}
          cloudError={cloudError}
          error={error}
          onCancel={session ? () => setMaking(false) : undefined}
          onExit={onExit}
          existing={list}
          onOpen={(w) => { setMaking(false); openWs(w); }}
          onMade={async (w) => {
            setMaking(false);
            await refresh();
            openWs(w);
          }}
        />
      </div>
    );
  }
  return (
    <Workspace
      key={`${session.info.storage}:${session.info.id}`}
      s={session}
      host={host}
      list={list}
      cloudError={cloudError}
      onSwitch={openWs}
      onNew={() => setMaking(true)}
      onExit={onExit}
      onGone={async () => {
        remember(LAST, null);
        session.close();
        setSession(null);
        const l = await refresh();
        if (l[0]) openWs(l[0]);
      }}
      onMade={async (w) => {
        await refresh();
        openWs(w);
      }}
    />
  );
}

function Welcome({ host, first, cloudError, error, existing, onCancel, onExit, onOpen, onMade }: {
  host: Host;
  first: boolean;
  cloudError: string | null;
  error: string | null;
  existing: WorkspaceInfo[];
  onCancel?: () => void;
  onExit?: () => void;
  onOpen: (w: WorkspaceInfo) => void;
  onMade: (w: WorkspaceInfo) => void;
}) {
  const [name, setName] = useState('');
  const [team, setTeam] = useState('');
  const [key, setKey] = useState('');
  const cloudOk = !!host.me;
  // Picked by the user, unless only one place is open to them.
  const [storage, setStorage] = useState<Storage | null>(!host.local ? 'cloud' : !cloudOk ? 'local' : null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(error);
  const suggested = team.replace(/[^a-z0-9]/gi, '').slice(0, 3).toUpperCase();
  const ready = !!name.trim() && !!team.trim() && !!storage && !busy;
  const make = async () => {
    if (!ready || !storage) return;
    setBusy(true);
    setErr(null);
    try {
      onMade(await createWorkspace(host, name.trim(), storage, team.trim(), key || suggested || 'T'));
    } catch (e) {
      setErr(message(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="d-welcome">
      <BoardAnimation />
      <div className="d-welcome-head">
        <h1>{first ? 'Welcome to Delta' : 'New workspace'}</h1>
        <p className="d-muted">{first ? 'Plan and track your team’s work: issues, projects, cycles and a roadmap, with Scout on hand to pick up issues.' : 'A separate set of teams, issues and projects.'}</p>
      </div>
      <div className="d-form d-welcome-form">
        <label className="d-field">
          <span>Workspace name</span>
          <input className="d-input" autoFocus placeholder="e.g. Acme" value={name} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && make()} />
        </label>
        <p className="d-hint">Usually your company or project. It holds all your teams, issues and projects.</p>
        <div className="d-field-row">
          <label className="d-field d-grow">
            <span>First team</span>
            <input className="d-input" placeholder="e.g. Engineering" value={team} onChange={(e) => setTeam(e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && make()} />
          </label>
          <label className="d-field d-key-field">
            <span>Issue key</span>
            <input className="d-input" placeholder={suggested || 'ENG'} value={key} maxLength={8} onChange={(e) => setKey(e.currentTarget.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))} onKeyDown={(e) => e.key === 'Enter' && make()} />
          </label>
        </div>
        <p className="d-hint">Every issue belongs to a team, like Engineering or Design. Start with one and add more later. The issue key starts each of the team’s issue IDs, like <code>{key || suggested || 'ENG'}-1</code>, and is filled in for you if you leave it blank.</p>
        <div className="d-field">
          <span>Where it’s kept</span>
          <div className="d-choices">
            <button type="button" className={`d-choice ${storage === 'local' ? 'on' : ''}`} disabled={!host.local} onClick={() => setStorage('local')}>
              <span className="d-row"><Icon name="laptop" /><b>On this computer</b></span>
              <span className="d-small d-dim">{host.local ? 'A file on this PC. Works offline; only you see it.' : 'Only in the Scout desktop app.'}</span>
            </button>
            <button type="button" className={`d-choice ${storage === 'cloud' ? 'on' : ''}`} disabled={!cloudOk} onClick={() => setStorage('cloud')}>
              <span className="d-row"><Icon name="cloud" /><b>ScoutCode Cloud</b></span>
              <span className="d-small d-dim">{cloudOk ? 'On our servers. Invite teammates and use it from any device.' : cloudError ?? 'Sign in to use it.'}</span>
            </button>
          </div>
        </div>
        <p className="d-hint">This applies to the whole workspace. You can copy a workspace from this computer to the cloud later, in Settings.</p>
        {err && <div className="d-error">{err}</div>}
        <div className="d-row d-end d-welcome-actions">
          {onCancel && <button type="button" className="d-btn" onClick={onCancel}>Cancel</button>}
          {!onCancel && onExit && <button type="button" className="d-btn" onClick={onExit}>Back</button>}
          <button type="button" className="d-btn primary" disabled={!ready} onClick={make}>{busy ? 'Creating…' : 'Create workspace'}</button>
        </div>
      </div>
      {!first && existing.length > 0 && !onCancel && (
        <div className="d-stack">
          <div className="d-section-label">Your workspaces</div>
          {existing.map((w) => <button type="button" key={w.storage + w.id} className="d-menu-item" onClick={() => onOpen(w)}><Icon name={w.storage === 'local' ? 'laptop' : 'cloud'} />{w.name}</button>)}
        </div>
      )}
    </div>
  );
}

/** Cards moving across a small board from Todo to Done, over and over: what Delta is for, at a glance. */
function BoardAnimation() {
  const cards = [
    ['ENG-1', 'Sign-in page'],
    ['ENG-2', 'Fix the sync bug'],
    ['ENG-3', 'Dark mode'],
  ];
  return (
    <div className="d-anim" aria-hidden="true">
      {['Todo', 'In Progress', 'Done'].map((c, n) => (
        <div key={c} className="d-anim-col" style={{ left: `calc(${n} * (100% - 28px) / 3 + ${n * 6 + 8}px)` }}>
          <span className={`d-anim-dot c${n}`} />{c}
        </div>
      ))}
      {cards.map(([id, title], n) => (
        <div key={id} className="d-anim-card" style={{ animationDelay: `${n * -3.33}s` }}>
          <span className="d-anim-state" style={{ animationDelay: `${n * -3.33}s` }} />
          <span className="d-anim-text"><span className="d-anim-id">{id}</span>{title}</span>
        </div>
      ))}
    </div>
  );
}

export function DeltaMark({ size = 18, loading = false, className = '' }: { size?: number; loading?: boolean; className?: string }) {
  if (loading) {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" className={`d-mark d-mark-loading ${className}`.trim()}>
        <defs>
          <mask id="d-mark-mask">
            <path d="M1.9 19.9L11.1 3C11.5 2.2 12.5 2.2 12.9 3L15.1 7.18L7.4 21.3H2.7C1.8 21.3 1.4 20.7 1.9 19.9ZM16.5 9.75L19.1 14.52L15.4 21.3H10.2ZM20.5 17.08L22.1 19.9C22.6 20.7 22.2 21.3 21.3 21.3H18.2Z" fill="#fff" />
          </mask>
          <linearGradient id="d-mark-shimmer" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#fff" stopOpacity="0" />
            <stop offset="50%" stopColor="#fff" stopOpacity="0.9" />
            <stop offset="100%" stopColor="#fff" stopOpacity="0" />
          </linearGradient>
        </defs>
        <path d="M1.9 19.9L11.1 3C11.5 2.2 12.5 2.2 12.9 3L15.1 7.18L7.4 21.3H2.7C1.8 21.3 1.4 20.7 1.9 19.9ZM16.5 9.75L19.1 14.52L15.4 21.3H10.2ZM20.5 17.08L22.1 19.9C22.6 20.7 22.2 21.3 21.3 21.3H18.2Z" fill="currentColor" opacity="0.28" />
        <g mask="url(#d-mark-mask)">
          <rect x="-30" y="-12" width="18" height="48" fill="url(#d-mark-shimmer)" transform="rotate(25 12 12)">
            <animate
              attributeName="x"
              values="-30; 36; 36"
              keyTimes="0; 0.35; 1"
              dur="3.2s"
              repeatCount="indefinite"
            />
          </rect>
        </g>
      </svg>
    );
  }
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" className={`d-mark ${className}`.trim()}>
      <path d="M1.9 19.9L11.1 3C11.5 2.2 12.5 2.2 12.9 3L15.1 7.18L7.4 21.3H2.7C1.8 21.3 1.4 20.7 1.9 19.9ZM16.5 9.75L19.1 14.52L15.4 21.3H10.2ZM20.5 17.08L22.1 19.9C22.6 20.7 22.2 21.3 21.3 21.3H18.2Z" />
    </svg>
  );
}

// ---------- an open workspace ----------

function Workspace({ s, host, list, cloudError, onSwitch, onNew, onExit, onGone, onMade }: {
  s: Session;
  host: Host;
  list: WorkspaceInfo[];
  cloudError: string | null;
  onSwitch: (w: WorkspaceInfo) => void;
  onNew: () => void;
  onExit?: () => void;
  onGone: () => void;
  onMade: (w: WorkspaceInfo) => void;
}) {
  const v = useSyncExternalStore(s.subscribe, s.getVersion);
  const firstTeam = Object.values(s.doc.team).sort((a, b) => a.createdAt - b.createdAt)[0];
  const [route, setRoute] = useState<Route>(() => (firstTeam ? { view: 'team', teamId: firstTeam.id, tab: 'all' } : { view: 'my' }));
  const [openId, setOpenId] = useState<string | null>(null);
  const [creating, setCreating] = useState<Partial<Issue> | null>(null);
  const [palette, setPalette] = useState(false);
  const [navOpen, setNavOpen] = useState(false);

  // A team, project or cycle that was deleted takes its page with it.
  useEffect(() => {
    const gone =
      ((route.view === 'team' || route.view === 'cycles') && !s.doc.team[route.teamId]) ||
      (route.view === 'project' && !s.doc.project[route.id]) ||
      (route.view === 'cycle' && !s.doc.cycle[route.id]) ||
      (route.view === 'doc' && !s.doc.page[route.id]);
    if (gone) setRoute(route.view === 'doc' ? { view: 'docs' } : firstTeam ? { view: 'team', teamId: firstTeam.id, tab: 'all' } : { view: 'my' });
    if (openId && !s.doc.issue[openId]) setOpenId(null);
  }, [v]);

  const ctx: DeltaCtx = {
    s,
    host,
    v,
    route,
    go: (r) => {
      setRoute(r);
      setNavOpen(false);
    },
    open: setOpenId,
    create: (d) => setCreating(d ?? {}),
  };

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setPalette((p) => !p);
        return;
      }
      if (isTyping(e.target) || e.ctrlKey || e.metaKey || e.altKey || document.querySelector('.d-modal-back:not(.closing), .d-menu')) return;
      if (e.key === 'c') {
        e.preventDefault();
        setCreating(defaultsFor(ctx));
      }
      if (e.key === '/') {
        e.preventDefault();
        setPalette(true);
      }
      if (e.key === 'Escape' && openId) {
        e.preventDefault();
        setOpenId(null);
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

  // Panels stay a moment after they close so they can animate out; pages remount (and fade in) when the route changes.
  const [shownId, idLeaving] = useLinger(openId && s.doc.issue[openId] ? openId : null);
  const [shownNew, newLeaving] = useLinger(creating);
  const [shownPalette, paletteLeaving] = useLinger(palette);
  const routeKey = `${route.view}:${'teamId' in route ? route.teamId : ''}:${'id' in route ? route.id : ''}`;

  return (
    <Ctx.Provider value={ctx}>
      <div className={`delta d-app ${navOpen ? 'nav-open' : ''}`}>
        <Nav list={list} cloudError={cloudError} onSwitch={onSwitch} onNew={onNew} onExit={onExit} onSearch={() => setPalette(true)} />
        <div className="d-scrim" onClick={() => setNavOpen(false)} />
        <main className="d-main">
          <button type="button" className="d-icon-btn d-nav-toggle" onClick={() => setNavOpen(true)} aria-label="Show navigation"><Icon name="issues" /></button>
          <Fragment key={routeKey}>
            {route.view === 'my' && <IssuesPage title="My issues" source={(i) => i.assignee === s.me} storageKey="my" defaults={{ assignee: s.me }} />}
            {route.view === 'team' && s.doc.team[route.teamId] && <TeamPage teamId={route.teamId} tab={route.tab ?? 'all'} />}
            {route.view === 'cycles' && s.doc.team[route.teamId] && <CyclesView teamId={route.teamId} />}
            {route.view === 'cycle' && s.doc.cycle[route.id] && <CycleView id={route.id} />}
            {route.view === 'projects' && <ProjectsView />}
            {route.view === 'project' && s.doc.project[route.id] && <ProjectView id={route.id} />}
            {route.view === 'roadmap' && <RoadmapView />}
            {route.view === 'docs' && <DocsView />}
            {route.view === 'doc' && s.doc.page[route.id] && <DocView id={route.id} />}
            {route.view === 'settings' && <SettingsView onGone={onGone} onMade={onMade} />}
          </Fragment>
        </main>
        {shownId && <IssueDetail id={shownId} closing={idLeaving} onClose={() => setOpenId(null)} />}
        {shownNew && <NewIssue defaults={shownNew} closing={newLeaving} onClose={() => setCreating(null)} />}
        {shownPalette && <Palette closing={paletteLeaving} onClose={() => setPalette(false)} />}
        <ContextMenuHost />
        <LightboxHost />
        {s.error && (
          <div className="d-toast" role="alert">
            <span className="d-grow">{s.error}</span>
            <button type="button" className="d-icon-btn" onClick={() => s.clearError()} aria-label="Dismiss"><Icon name="close" size={14} /></button>
          </div>
        )}
      </div>
    </Ctx.Provider>
  );
}

/** What a new issue starts with, from the page it's made on. */
function defaultsFor(c: DeltaCtx): Partial<Issue> {
  const r = c.route;
  if (r.view === 'team' || r.view === 'cycles') return { teamId: r.teamId };
  if (r.view === 'cycle') return { teamId: c.s.doc.cycle[r.id]?.teamId, cycleId: r.id };
  if (r.view === 'project') return { projectId: r.id, teamId: c.s.doc.project[r.id]?.teamIds[0] };
  if (r.view === 'my') return { assignee: c.s.me };
  return {};
}

function Nav({ list, cloudError, onSwitch, onNew, onExit, onSearch }: {
  list: WorkspaceInfo[];
  cloudError: string | null;
  onSwitch: (w: WorkspaceInfo) => void;
  onNew: () => void;
  onExit?: () => void;
  onSearch: () => void;
}) {
  const c = useDelta();
  const { s, route, go } = c;
  const teams = Object.values(s.doc.team).sort((a, b) => a.createdAt - b.createdAt);
  const [shut, setShut] = useState<Set<string>>(new Set());
  const myCount = Object.values(s.doc.issue).filter((i) => i.assignee === s.me && !isDone(s.doc.state[i.stateId])).length;
  const here = (r: Route) => JSON.stringify(r) === JSON.stringify(route) || (r.view === route.view && r.view !== 'team' && r.view !== 'cycles' && !('id' in r));
  const local = list.filter((w) => w.storage === 'local');
  const cloud = list.filter((w) => w.storage === 'cloud');
  return (
    <nav className="d-nav">
      <div className="d-nav-head">
        <Menu width={260} trigger={(open, toggle) => (
          <button type="button" className={`d-ws ${open ? 'open' : ''}`} onClick={toggle}>
            <span className="d-ws-icon">{s.doc.workspace.name.charAt(0).toUpperCase()}</span>
            <span className="d-ellipsis d-grow">{s.doc.workspace.name}</span>
            <Icon name="chevronDown" size={13} />
          </button>
        )}>
          {(close) => (
            <>
              {local.length > 0 && <div className="d-menu-label">On this computer</div>}
              {local.map((w) => <MenuItem key={w.id} selected={w.id === s.info.id} onClick={() => { close(); if (w.id !== s.info.id) onSwitch(w); }}><Icon name="laptop" size={14} /><span className="d-grow d-ellipsis">{w.name}</span></MenuItem>)}
              {(cloud.length > 0 || cloudError) && <div className="d-menu-label">ScoutCode Cloud</div>}
              {cloud.map((w) => <MenuItem key={w.id} selected={w.id === s.info.id} onClick={() => { close(); if (w.id !== s.info.id) onSwitch(w); }}><Icon name="cloud" size={14} /><span className="d-grow d-ellipsis">{w.name}</span><span className="d-tiny d-dim">{w.members > 1 ? `${w.members} people` : ''}</span></MenuItem>)}
              {cloudError && !cloud.length && <div className="d-menu-empty">{cloudError}</div>}
              <hr />
              <MenuItem onClick={() => { close(); onNew(); }}><Icon name="plus" size={14} />New workspace…</MenuItem>
              <MenuItem onClick={() => { close(); go({ view: 'settings' }); }}><Icon name="gear" size={14} />Workspace settings</MenuItem>
              {onExit && <MenuItem onClick={() => { close(); onExit(); }}><Icon name="back" size={14} />Leave Delta</MenuItem>}
            </>
          )}
        </Menu>
      </div>
      <div className="d-nav-actions">
        <button type="button" className="d-new" onClick={() => c.create(defaultsFor(c))}><Icon name="plus" size={14} /><span className="d-grow">New issue</span><kbd>C</kbd></button>
        <button type="button" className="d-icon-btn" onClick={onSearch} title="Search and commands (Ctrl+K)"><Icon name="search" size={15} /></button>
      </div>
      <div className="d-nav-scroll">
        <NavItem icon="inbox" label="My issues" count={myCount} on={here({ view: 'my' })} onClick={() => go({ view: 'my' })} />
        <NavItem icon="project" label="Projects" on={route.view === 'projects' || route.view === 'project'} onClick={() => go({ view: 'projects' })} />
        <NavItem icon="roadmap" label="Roadmap" on={here({ view: 'roadmap' })} onClick={() => go({ view: 'roadmap' })} />
        <NavItem icon="doc" label="Docs" on={route.view === 'docs'} onClick={() => go({ view: 'docs' })} />
        <div className="d-section-label d-nav-label">Your teams</div>
        {teams.map((t) => {
          const open = !shut.has(t.id);
          const cycle = Object.values(s.doc.cycle).find((x) => x.teamId === t.id && cycleStatus(x) === 'current');
          return (
            <div key={t.id}>
              <button type="button" className="d-nav-team" onClick={() => { const n = new Set(shut); if (open) n.add(t.id); else n.delete(t.id); setShut(n); }}>
                <span className="d-team-key">{t.key}</span>
                <span className="d-ellipsis d-grow">{t.name}</span>
                <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
              </button>
              {open && (
                <div className="d-nav-sub">
                  <NavItem icon="issues" label="Issues" on={route.view === 'team' && route.teamId === t.id} onClick={() => go({ view: 'team', teamId: t.id, tab: 'all' })} />
                  <NavItem icon="cycle" label="Cycles" on={(route.view === 'cycles' && route.teamId === t.id) || (route.view === 'cycle' && s.doc.cycle[route.id]?.teamId === t.id)} onClick={() => go(cycle ? { view: 'cycle', id: cycle.id } : { view: 'cycles', teamId: t.id })} hint={cycle ? cycleName(cycle) : undefined} />
                </div>
              )}
            </div>
          );
        })}
        <DocTree />
      </div>
      <div className="d-nav-foot">
        <button type="button" className={`d-nav-item ${route.view === 'settings' ? 'on' : ''}`} onClick={() => go({ view: 'settings' })}>
          <Icon name="gear" size={15} /><span className="d-grow">Settings</span>
        </button>
        <div className="d-storage" title={s.info.storage === 'local' ? 'Kept in a file on this computer' : 'Kept on ScoutCode Cloud'}>
          <Icon name={s.info.storage === 'local' ? 'laptop' : 'cloud'} size={13} />
          <span className="d-grow">{s.info.storage === 'local' ? 'On this computer' : `Cloud · ${s.members.length} ${s.members.length === 1 ? 'member' : 'members'}`}</span>
          {s.saving && <span className="d-spinner small" />}
        </div>
      </div>
    </nav>
  );
}

function NavItem({ icon, label, on, onClick, count, hint }: { icon: string; label: string; on: boolean; onClick: () => void; count?: number; hint?: string }) {
  return (
    <button type="button" className={`d-nav-item ${on ? 'on' : ''}`} onClick={onClick}>
      <Icon name={icon} size={15} />
      <span className="d-grow d-ellipsis">{label}</span>
      {hint && <span className="d-tiny d-dim d-ellipsis">{hint}</span>}
      {!!count && <span className="d-count">{count}</span>}
    </button>
  );
}

// ---------- issue lists ----------

function TeamPage({ teamId, tab }: { teamId: string; tab: 'all' | 'active' | 'backlog' }) {
  const { s, go } = useDelta();
  const team = s.doc.team[teamId];
  const source = (i: Issue) => {
    if (i.teamId !== teamId) return false;
    const t = s.doc.state[i.stateId]?.type;
    if (tab === 'active') return t === 'unstarted' || t === 'started' || t === 'review';
    if (tab === 'backlog') return t === 'backlog';
    return true;
  };
  return (
    <IssuesPage
      title={team.name}
      crumb={team.key}
      source={source}
      storageKey={`team:${teamId}`}
      defaults={{ teamId, ...(tab === 'backlog' ? { stateId: teamStates(s.doc, teamId).find((x) => x.type === 'backlog')?.id } : {}) }}
      tabs={
        <div className="d-tabs">
          {(['all', 'active', 'backlog'] as const).map((t) => (
            <button type="button" key={t} className={`d-tab ${tab === t ? 'on' : ''}`} onClick={() => go({ view: 'team', teamId, tab: t })}>
              {t === 'all' ? 'All issues' : t === 'active' ? 'Active' : 'Backlog'}
            </button>
          ))}
        </div>
      }
    />
  );
}

export type Grouping = 'status' | 'assignee' | 'priority' | 'project' | 'none';
export interface Filters { q: string; states: StateType[]; assignees: (string | null)[]; priorities: number[]; labels: string[]; projects: (string | null)[] }
const NO_FILTERS: Filters = { q: '', states: [], assignees: [], priorities: [], labels: [], projects: [] };

interface Group { key: string; label: string; icon: ReactNode; issues: Issue[]; patch: (i: Issue) => Partial<Issue> | null }

const TYPE_NAMES: Record<StateType, string> = { unstarted: 'Todo', backlog: 'Backlog', started: 'In Progress', review: 'In Review', completed: 'Done', canceled: 'Canceled' };
const TYPE_COLORS: Record<StateType, string> = { unstarted: '#b0b4ba', backlog: '#8b8d98', started: '#f5a524', review: '#3e9b4f', completed: '#5e6ad2', canceled: '#6f6e77' };

export const byOrder = (a: Issue, b: Issue) => a.sortOrder - b.sortOrder || b.createdAt - a.createdAt;

function groupIssues(c: DeltaCtx, issues: Issue[], by: Grouping, teamId: string | null): Group[] {
  const { doc } = c.s;
  const sorted = [...issues].sort(byOrder);
  if (by === 'none') return [{ key: 'all', label: 'All issues', icon: null, issues: sorted, patch: () => ({}) }];
  if (by === 'status') {
    if (teamId) {
      return teamStates(doc, teamId).map((st) => ({ key: st.id, label: st.name, icon: <StateIcon state={st} />, issues: sorted.filter((i) => i.stateId === st.id), patch: () => ({ stateId: st.id }) }));
    }
    return STATE_TYPES.map((t) => ({
      key: t,
      label: TYPE_NAMES[t],
      icon: <StateIcon state={{ type: t, color: TYPE_COLORS[t] }} />,
      issues: sorted.filter((i) => doc.state[i.stateId]?.type === t),
      patch: (i: Issue) => {
        const st = teamStates(doc, i.teamId).find((x) => x.type === t);
        return st ? { stateId: st.id } : null;
      },
    }));
  }
  if (by === 'priority') {
    return [1, 2, 3, 4, 0].map((p) => ({ key: String(p), label: PRIORITIES[p], icon: <PriorityIcon p={p} />, issues: sorted.filter((i) => i.priority === p), patch: () => ({ priority: p }) }));
  }
  if (by === 'assignee') {
    const people = [...new Set([...c.s.people, ...issues.map((i) => i.assignee).filter((x): x is string => !!x)])];
    return [...people.map((u) => u as string | null), null].map((u) => ({
      key: u ?? '~none',
      label: personName(u),
      icon: <Avatar name={u} />,
      issues: sorted.filter((i) => i.assignee === u),
      patch: () => ({ assignee: u }),
    }));
  }
  const projects = Object.values(doc.project).sort((a, b) => a.name.localeCompare(b.name));
  return [...projects.map((p) => ({ key: p.id, label: p.name, icon: <Icon name="project" size={14} />, issues: sorted.filter((i) => i.projectId === p.id), patch: () => ({ projectId: p.id }) })),
    { key: '~none', label: 'No project', icon: <Icon name="project" size={14} />, issues: sorted.filter((i) => !i.projectId), patch: () => ({ projectId: null }) }];
}

function matches(c: DeltaCtx, i: Issue, f: Filters): boolean {
  const { doc } = c.s;
  if (f.states.length && !f.states.includes(doc.state[i.stateId]?.type)) return false;
  if (f.assignees.length && !f.assignees.includes(i.assignee)) return false;
  if (f.priorities.length && !f.priorities.includes(i.priority)) return false;
  if (f.labels.length && !f.labels.some((l) => i.labelIds.includes(l))) return false;
  if (f.projects.length && !f.projects.includes(i.projectId)) return false;
  const q = f.q.trim().toLowerCase();
  if (q && !i.title.toLowerCase().includes(q) && !issueKey(doc, i).toLowerCase().includes(q) && !i.description.toLowerCase().includes(q)) return false;
  return true;
}

function useStored<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    const raw = recall(`delta.${key}`);
    try {
      return raw ? { ...initial, ...JSON.parse(raw) } : initial;
    } catch {
      return initial;
    }
  });
  return [v, (n: T) => { setV(n); remember(`delta.${key}`, JSON.stringify(n)); }];
}

/** A page of issues with filters and a list or board. Projects and cycles use it too. */
export function IssuesPage({ title, crumb, source, storageKey, defaults, tabs, header, embedded }: {
  title: string;
  crumb?: string;
  source: (i: Issue) => boolean;
  storageKey: string;
  defaults: Partial<Issue>;
  tabs?: ReactNode;
  header?: ReactNode;
  embedded?: boolean;
}) {
  const c = useDelta();
  const { s, v } = c;
  const [display, setDisplay] = useStored(`display:${storageKey}`, { mode: 'list' as 'list' | 'board', group: 'status' as Grouping, done: true });
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const all = useMemo(() => Object.values(s.doc.issue).filter(source), [v, source]);
  const issues = useMemo(() => all.filter((i) => matches(c, i, filters) && (display.done || !isDone(s.doc.state[i.stateId]))), [all, filters, display.done, v]);
  const teamIds = [...new Set(all.map((i) => i.teamId))];
  const teamId = defaults.teamId && (teamIds.length === 0 || (teamIds.length === 1 && teamIds[0] === defaults.teamId)) ? defaults.teamId : teamIds.length === 1 ? teamIds[0] : null;
  const groups = useMemo(() => groupIssues(c, issues, display.group, teamId), [issues, display.group, teamId, v]);
  const shownGroups = display.mode === 'board' || display.group === 'none' ? groups : groups.filter((g) => g.issues.length);
  const active = filters.states.length + filters.assignees.length + filters.priorities.length + filters.labels.length + filters.projects.length;

  return (
    <div className={`d-page ${embedded ? 'embedded' : ''}`}>
      {!embedded && (
        <header className="d-head">
          {crumb && <span className="d-team-key">{crumb}</span>}
          <h1 className="d-ellipsis">{title}</h1>
          {tabs}
          <span className="d-grow" />
        </header>
      )}
      {header}
      <div className="d-toolbar">
        <label className="d-search">
          <Icon name="search" size={14} />
          <input placeholder="Filter by title or ID" value={filters.q} onChange={(e) => setFilters({ ...filters, q: e.currentTarget.value })} />
        </label>
        <FilterMenu filters={filters} onChange={setFilters} />
        {active > 0 && <button type="button" className="d-btn ghost small" onClick={() => setFilters({ ...NO_FILTERS, q: filters.q })}>Clear {active}</button>}
        <span className="d-grow" />
        <div className="d-seg">
          <button type="button" className={display.mode === 'list' ? 'on' : ''} onClick={() => setDisplay({ ...display, mode: 'list' })} title="List"><Icon name="list" size={14} /></button>
          <button type="button" className={display.mode === 'board' ? 'on' : ''} onClick={() => setDisplay({ ...display, mode: 'board' })} title="Board"><Icon name="board" size={14} /></button>
        </div>
        <Menu width={220} align="end" trigger={(open, toggle) => <button type="button" className={`d-btn small ${open ? 'on' : ''}`} onClick={toggle}>Display<Icon name="chevronDown" size={12} /></button>}>
          {() => (
            <>
              <div className="d-menu-label">Group by</div>
              {(['status', 'assignee', 'priority', 'project', 'none'] as Grouping[]).map((g) => (
                <MenuItem key={g} selected={display.group === g} onClick={() => setDisplay({ ...display, group: g })}>{g === 'none' ? 'No grouping' : g.charAt(0).toUpperCase() + g.slice(1)}</MenuItem>
              ))}
              <hr />
              <MenuItem selected={display.done} onClick={() => setDisplay({ ...display, done: !display.done })}>Show done and canceled</MenuItem>
            </>
          )}
        </Menu>
      </div>
      {all.length === 0 ? (
        <Empty icon="issues" title="No issues yet" text="Issues are the work your team tracks: bugs, features, chores." action={<button type="button" className="d-btn primary" onClick={() => c.create(defaults)}><Icon name="plus" size={14} />New issue</button>} />
      ) : issues.length === 0 ? (
        <Empty icon="search" title="Nothing matches" text="Try other filters." />
      ) : display.mode === 'list' ? (
        <div className="d-list">
          {shownGroups.map((g) => <ListGroup key={g.key} g={g} showHead={display.group !== 'none'} defaults={{ ...defaults, ...(g.patch(g.issues[0] ?? ({ teamId: teamId ?? '' } as Issue)) ?? {}) }} />)}
        </div>
      ) : (
        <Board groups={shownGroups} defaults={defaults} />
      )}
    </div>
  );
}

function FilterMenu({ filters, onChange }: { filters: Filters; onChange: (f: Filters) => void }) {
  const { s } = useDelta();
  const toggle = <K extends keyof Filters>(k: K, val: Filters[K] extends (infer U)[] ? U : never) => {
    const list = filters[k] as unknown[];
    onChange({ ...filters, [k]: list.includes(val) ? list.filter((x) => x !== val) : [...list, val] });
  };
  const [tab, setTab] = useState<'status' | 'assignee' | 'priority' | 'label' | 'project'>('status');
  return (
    <Menu width={260} trigger={(open, toggleMenu) => <button type="button" className={`d-btn small ${open ? 'on' : ''}`} onClick={toggleMenu}><Icon name="list" size={13} />Filter</button>}>
      {() => (
        <>
          <div className="d-menu-tabs">
            {(['status', 'assignee', 'priority', 'label', 'project'] as const).map((t) => <button type="button" key={t} className={tab === t ? 'on' : ''} onClick={() => setTab(t)}>{t.charAt(0).toUpperCase() + t.slice(1)}</button>)}
          </div>
          <div className="d-menu-scroll">
            {tab === 'status' && STATE_TYPES.map((t) => <Check key={t} on={filters.states.includes(t)} onClick={() => toggle('states', t)}><StateIcon state={{ type: t, color: TYPE_COLORS[t] }} />{TYPE_NAMES[t]}</Check>)}
            {tab === 'assignee' && [...s.people, SCOUT, null].map((u) => <Check key={u ?? '~'} on={filters.assignees.includes(u)} onClick={() => toggle('assignees', u)}><Avatar name={u} />{personName(u)}</Check>)}
            {tab === 'priority' && [1, 2, 3, 4, 0].map((p) => <Check key={p} on={filters.priorities.includes(p)} onClick={() => toggle('priorities', p)}><PriorityIcon p={p} />{PRIORITIES[p]}</Check>)}
            {tab === 'label' && labelOptions(s.doc).map((l) => <Check key={l.value} on={filters.labels.includes(l.value)} onClick={() => toggle('labels', l.value)}>{l.icon}{l.label}</Check>)}
            {tab === 'project' && [...Object.values(s.doc.project).map((p) => ({ id: p.id as string | null, name: p.name })), { id: null, name: 'No project' }].map((p) => (
              <Check key={p.id ?? '~'} on={filters.projects.includes(p.id)} onClick={() => toggle('projects', p.id)}><Icon name="project" size={14} />{p.name}</Check>
            ))}
          </div>
        </>
      )}
    </Menu>
  );
}

function Check({ on, onClick, children }: { on: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" className="d-menu-item" onClick={onClick}>
      <span className={`d-box ${on ? 'on' : ''}`}>{on && <Icon name="check" size={11} />}</span>
      {children}
    </button>
  );
}

export function Empty({ icon, title, text, action }: { icon: string; title: string; text: string; action?: ReactNode }) {
  return (
    <div className="d-empty">
      <Icon name={icon} size={28} />
      <b>{title}</b>
      <span className="d-muted">{text}</span>
      {action}
    </div>
  );
}

function ListGroup({ g, showHead, defaults }: { g: Group; showHead: boolean; defaults: Partial<Issue> }) {
  const c = useDelta();
  const [shut, setShut] = useState(false);
  // While the group folds or unfolds its rows stay mounted and clipped; once it's shut they unmount.
  const [moving, setMoving] = useState(false);
  const toggle = () => {
    setMoving(true);
    setShut(!shut);
    setTimeout(() => setMoving(false), 320);
  };
  return (
    <section className={`d-group ${shut ? 'shut' : ''}`}>
      {showHead && (
        <div className="d-group-head">
          <button type="button" className="d-group-toggle" onClick={toggle}>
            <Icon name="chevronDown" size={12} className="d-chev" />
            {g.icon}
            <b>{g.label}</b>
            <span className="d-dim">{g.issues.length}</span>
          </button>
          <span className="d-grow" />
          <button type="button" className="d-icon-btn" title="New issue here" onClick={() => c.create(defaults)}><Icon name="plus" size={14} /></button>
        </div>
      )}
      <div className={`d-group-body ${moving ? 'moving' : ''}`}>
        <div>{(!shut || moving) && g.issues.map((i, n) => <IssueRow key={i.id} i={i} n={n} />)}</div>
      </div>
    </section>
  );
}

/** One issue in a list: its priority, ID, state, title, labels and dates, and its assignee. Each opens a picker. */
export function IssueRow({ i, compact, n = 0 }: { i: Issue; compact?: boolean; n?: number }) {
  const c = useDelta();
  const { s } = c;
  const doc = s.doc;
  const st = doc.state[i.stateId];
  const set = (patch: Partial<Issue>) => s.dispatch({ type: 'update', kind: 'issue', id: i.id, patch });
  const subs = Object.values(doc.issue).filter((x) => x.parentId === i.id);
  const overdue = i.dueDate && !isDone(st) && i.dueDate < today();
  return (
    <div className="d-row-issue" style={{ ['--n' as string]: Math.min(n, 14) }} role="button" tabIndex={0} onClick={() => c.open(i.id)} onKeyDown={(e) => e.key === 'Enter' && c.open(i.id)} onContextMenu={(e) => openContextMenu(e, issueMenu(c, i))}>
      <span onClick={(e) => e.stopPropagation()}>
        <Picker options={priorityOptions()} value={i.priority} onPick={(p) => set({ priority: p })} width={190} trigger={(_, t) => <button type="button" className="d-icon-btn" title={PRIORITIES[i.priority]} onClick={t}><PriorityIcon p={i.priority} /></button>} />
      </span>
      <span className="d-key">{issueKey(doc, i)}</span>
      <span onClick={(e) => e.stopPropagation()}>
        <Picker options={stateOptions(teamStates(doc, i.teamId))} value={i.stateId} onPick={(v) => set({ stateId: v })} width={200} trigger={(_, t) => <button type="button" className="d-icon-btn" title={st?.name} onClick={t}><StateIcon state={st} /></button>} />
      </span>
      <span className="d-title d-ellipsis">{i.title}</span>
      {subs.length > 0 && <span className="d-chip"><Icon name="parent" size={12} />{subs.filter((x) => isDone(doc.state[x.stateId])).length}/{subs.length}</span>}
      {i.agent && <span className="d-chip scout" title={`Scout is on it${i.agent.pc ? ` on ${i.agent.pc}` : ''}`}><Icon name="sparkle" size={11} />Scout</span>}
      <span className="d-grow" />
      {!compact && i.labelIds.slice(0, 3).map((l) => doc.label[l] && <span key={l} className="d-chip"><LabelDot color={doc.label[l].color} />{doc.label[l].name}</span>)}
      {!compact && i.projectId && doc.project[i.projectId] && <span className="d-chip"><Icon name="project" size={12} />{doc.project[i.projectId].name}</span>}
      {!compact && i.cycleId && doc.cycle[i.cycleId] && <span className="d-chip"><Icon name="cycle" size={12} />{cycleName(doc.cycle[i.cycleId])}</span>}
      {i.estimate != null && <span className="d-chip"><Icon name="estimate" size={12} />{i.estimate}</span>}
      {i.dueDate && <span className={`d-chip ${overdue ? 'late' : ''}`}><Icon name="calendar" size={12} />{shortDay(i.dueDate)}</span>}
      <span className="d-date d-dim">{shortDay(today(new Date(i.createdAt)))}</span>
      <span onClick={(e) => e.stopPropagation()}>
        <Picker options={peopleOptions(s.people)} value={i.assignee} onPick={(u) => set({ assignee: u })} width={220} trigger={(_, t) => <button type="button" className="d-icon-btn" onClick={t}><Avatar name={i.assignee} /></button>} />
      </span>
    </div>
  );
}

/** Columns of cards. Dragging a card to another column changes what the columns are grouped by; within one, its place. */
function Board({ groups, defaults }: { groups: Group[]; defaults: Partial<Issue> }) {
  const c = useDelta();
  const { s } = c;
  const drag = useRef<string | null>(null);
  const [over, setOver] = useState<{ col: string; before: string | null } | null>(null);
  const drop = (g: Group, before: string | null) => {
    const id = drag.current;
    drag.current = null;
    setOver(null);
    const i = id && s.doc.issue[id];
    if (!i) return;
    const patch = g.patch(i);
    if (!patch) return;
    const list = g.issues.filter((x) => x.id !== i.id);
    const at = before ? list.findIndex((x) => x.id === before) : list.length;
    const prev = list[at - 1]?.sortOrder;
    const next = list[at]?.sortOrder;
    const sortOrder = prev == null && next == null ? i.sortOrder : prev == null ? next! - 1000 : next == null ? prev + 1000 : (prev + next) / 2;
    s.dispatch({ type: 'update', kind: 'issue', id: i.id, patch: { ...patch, sortOrder } });
  };
  return (
    <div className="d-board">
      {groups.map((g) => (
        <div
          key={g.key}
          className={`d-col ${over?.col === g.key ? 'over' : ''}`}
          onDragOver={(e) => { e.preventDefault(); if (over?.col !== g.key || over.before !== null) setOver({ col: g.key, before: null }); }}
          onDrop={(e) => { e.preventDefault(); drop(g, over?.col === g.key ? over.before : null); }}
        >
          <div className="d-col-head">
            {g.icon}
            <b className="d-ellipsis">{g.label}</b>
            <span className="d-dim">{g.issues.length}</span>
            <span className="d-grow" />
            <button type="button" className="d-icon-btn" title="New issue here" onClick={() => c.create({ ...defaults, ...(g.patch((g.issues[0] ?? { teamId: defaults.teamId ?? '' }) as Issue) ?? {}) })}><Icon name="plus" size={14} /></button>
          </div>
          <div className="d-col-body">
            {g.issues.map((i, n) => (
              <div key={i.id} className="d-card-slot" style={{ ['--n' as string]: Math.min(n, 10) }}>
                {over?.col === g.key && over.before === i.id && <div className="d-drop-line" />}
                <Card
                  i={i}
                  onDragStart={() => { drag.current = i.id; }}
                  onDragOver={(e) => { e.preventDefault(); e.stopPropagation(); if (over?.before !== i.id) setOver({ col: g.key, before: i.id }); }}
                />
              </div>
            ))}
            {over?.col === g.key && over.before === null && <div className="d-drop-line" />}
          </div>
        </div>
      ))}
    </div>
  );
}

function Card({ i, onDragStart, onDragOver }: { i: Issue; onDragStart: () => void; onDragOver: (e: DragEvent) => void }) {
  const c = useDelta();
  const doc = c.s.doc;
  const st = doc.state[i.stateId];
  return (
    <div className="d-card" draggable onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', i.id); const el = e.currentTarget; requestAnimationFrame(() => el.classList.add('dragging')); onDragStart(); }} onDragEnd={(e) => e.currentTarget.classList.remove('dragging')} onDragOver={onDragOver} onClick={() => c.open(i.id)} onContextMenu={(e) => openContextMenu(e, issueMenu(c, i))}>
      <div className="d-row">
        <span className="d-key">{issueKey(doc, i)}</span>
        <span className="d-grow" />
        <Avatar name={i.assignee} size={16} />
      </div>
      <div className="d-row d-card-title">
        <StateIcon state={st} />
        <span>{i.title}</span>
      </div>
      <div className="d-row d-wrap">
        <PriorityIcon p={i.priority} />
        {i.agent && <span className="d-chip scout"><Icon name="sparkle" size={11} />Scout</span>}
        {i.labelIds.map((l) => doc.label[l] && <span key={l} className="d-chip"><LabelDot color={doc.label[l].color} />{doc.label[l].name}</span>)}
        {i.projectId && doc.project[i.projectId] && <span className="d-chip"><Icon name="project" size={12} />{doc.project[i.projectId].name}</span>}
        {i.estimate != null && <span className="d-chip"><Icon name="estimate" size={12} />{i.estimate}</span>}
        {i.dueDate && <span className="d-chip"><Icon name="calendar" size={12} />{shortDay(i.dueDate)}</span>}
      </div>
    </div>
  );
}

// ---------- a new issue ----------

function NewIssue({ defaults, onClose, closing }: { defaults: Partial<Issue>; onClose: () => void; closing?: boolean }) {
  const { s, open } = useDelta();
  const doc = s.doc;
  const teams = Object.values(doc.team).sort((a, b) => a.createdAt - b.createdAt);
  const [teamId, setTeamId] = useState(defaults.teamId && doc.team[defaults.teamId] ? defaults.teamId : teams[0]?.id);
  const states = teamStates(doc, teamId);
  const pickState = (id: string | undefined) => (id && doc.state[id]?.teamId === teamId ? id : (states.find((x) => x.type === 'unstarted') ?? states[0])?.id);
  const [d, setD] = useState<Partial<Issue>>({ priority: 0, assignee: null, labelIds: [], projectId: null, cycleId: null, estimate: null, dueDate: null, ...defaults });
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const bodyBox = useRef<HTMLTextAreaElement>(null);
  const img = imageHandlers(s, bodyBox, setBody);
  const [more, setMore] = useState(false);
  const stateId = pickState(d.stateId);
  const cycles = Object.values(doc.cycle).filter((x) => x.teamId === teamId && cycleStatus(x) !== 'past').sort((a, b) => a.startsAt.localeCompare(b.startsAt));
  const submit = () => {
    if (!title.trim()) return;
    const id = newId();
    const ok = s.dispatch({
      type: 'create',
      kind: 'issue',
      id,
      data: { teamId, title: title.trim(), description: body, stateId, priority: d.priority, assignee: d.assignee, labelIds: d.labelIds, projectId: d.projectId, cycleId: d.cycleId && doc.cycle[d.cycleId]?.teamId === teamId ? d.cycleId : null, estimate: d.estimate, dueDate: d.dueDate, parentId: d.parentId ?? null },
    });
    if (!ok) return;
    if (more) {
      setTitle('');
      setBody('');
    } else {
      onClose();
      open(id);
    }
  };
  const st = doc.state[stateId ?? ''];
  return (
    <Modal onClose={onClose} closing={closing} className="d-new-issue">
      <div className="d-row">
        <Picker options={teams.map((t) => ({ value: t.id, label: t.name, icon: <span className="d-team-key">{t.key}</span> }))} value={teamId} onPick={(v) => setTeamId(v)} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><span className="d-team-key">{doc.team[teamId]?.key}</span><Icon name="chevronDown" size={12} /></button>} />
        <span className="d-dim">New issue{d.parentId && doc.issue[d.parentId] ? ` under ${issueKey(doc, doc.issue[d.parentId])}` : ''}</span>
        <span className="d-grow" />
        <button type="button" className="d-icon-btn" onClick={onClose} aria-label="Close"><Icon name="close" /></button>
      </div>
      <input className="d-big-input" autoFocus placeholder="Issue title" value={title} onChange={(e) => setTitle(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); submit(); } }} />
      <textarea ref={bodyBox} className="d-body-input" placeholder="Add a description… (paste or drop pictures)" onPaste={img.onPaste} onDrop={img.onDrop} onDragOver={img.onDragOver} value={body} onChange={(e) => setBody(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) submit(); }} />
      <div className="d-row d-wrap">
        <Picker options={stateOptions(states)} value={stateId ?? null} onPick={(v) => setD({ ...d, stateId: v ?? undefined })} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><StateIcon state={st} />{st?.name}</button>} />
        <Picker options={priorityOptions()} value={d.priority ?? 0} onPick={(p) => setD({ ...d, priority: p })} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><PriorityIcon p={d.priority ?? 0} />{PRIORITIES[d.priority ?? 0]}</button>} />
        <Picker options={peopleOptions(s.people)} value={d.assignee ?? null} onPick={(u) => setD({ ...d, assignee: u })} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><Avatar name={d.assignee ?? null} size={16} />{personName(d.assignee ?? null)}</button>} />
        <Picker multi options={labelOptions(doc)} value={d.labelIds ?? []} onPick={(l) => setD({ ...d, labelIds: (d.labelIds ?? []).includes(l) ? d.labelIds!.filter((x) => x !== l) : [...(d.labelIds ?? []), l] })} trigger={(_, t) => (
          <button type="button" className="d-chip-btn" onClick={t}><Icon name="tag" size={13} />{d.labelIds?.length ? d.labelIds.map((l) => doc.label[l]?.name).join(', ') : 'Labels'}</button>
        )} />
        <Picker options={[{ value: null as string | null, label: 'No project', icon: <Icon name="project" size={14} /> }, ...Object.values(doc.project).filter((p) => p.status !== 'completed' && p.status !== 'canceled').map((p) => ({ value: p.id as string | null, label: p.name, icon: <Icon name="project" size={14} /> }))]} value={d.projectId ?? null} onPick={(p) => setD({ ...d, projectId: p })} trigger={(_, t) => (
          <button type="button" className="d-chip-btn" onClick={t}><Icon name="project" size={13} />{d.projectId ? doc.project[d.projectId]?.name : 'Project'}</button>
        )} />
        {cycles.length > 0 && (
          <Picker options={[{ value: null as string | null, label: 'No cycle' }, ...cycles.map((x) => ({ value: x.id as string | null, label: cycleName(x), hint: cycleStatus(x) === 'current' ? 'current' : shortDay(x.startsAt) }))]} value={d.cycleId ?? null} onPick={(x) => setD({ ...d, cycleId: x })} trigger={(_, t) => (
            <button type="button" className="d-chip-btn" onClick={t}><Icon name="cycle" size={13} />{d.cycleId && doc.cycle[d.cycleId]?.teamId === teamId ? cycleName(doc.cycle[d.cycleId]) : 'Cycle'}</button>
          )} />
        )}
        <label className="d-chip-btn"><Icon name="calendar" size={13} /><input type="date" className="d-date-input" value={d.dueDate ?? ''} onChange={(e) => setD({ ...d, dueDate: e.currentTarget.value || null })} /></label>
      </div>
      <div className="d-row d-modal-foot">
        <label className="d-row d-small d-muted d-toggle"><input type="checkbox" checked={more} onChange={(e) => setMore(e.currentTarget.checked)} />Create more</label>
        <span className="d-grow" />
        <button type="button" className="d-btn primary" disabled={!title.trim()} onClick={submit}>Create issue</button>
      </div>
    </Modal>
  );
}

// ---------- the command palette ----------

function Palette({ onClose, closing }: { onClose: () => void; closing?: boolean }) {
  const c = useDelta();
  const { s } = c;
  const doc = s.doc;
  const [q, setQ] = useState('');
  const [at, setAt] = useState(0);
  type Row = { key: string; icon: ReactNode; label: string; hint?: string; run: () => void };
  const rows = useMemo<Row[]>(() => {
    const t = q.trim().toLowerCase();
    const out: Row[] = [];
    const cmd = (key: string, icon: ReactNode, label: string, run: () => void, hint?: string) => {
      if (!t || label.toLowerCase().includes(t)) out.push({ key, icon, label, run, hint });
    };
    cmd('new', <Icon name="plus" size={14} />, 'Create new issue', () => c.create(defaultsFor(c)), 'C');
    cmd('my', <Icon name="inbox" size={14} />, 'Go to My issues', () => c.go({ view: 'my' }));
    cmd('projects', <Icon name="project" size={14} />, 'Go to Projects', () => c.go({ view: 'projects' }));
    cmd('roadmap', <Icon name="roadmap" size={14} />, 'Go to Roadmap', () => c.go({ view: 'roadmap' }));
    cmd('docs', <Icon name="doc" size={14} />, 'Go to Docs', () => c.go({ view: 'docs' }));
    cmd('newdoc', <Icon name="plus" size={14} />, 'Create new doc', () => newPage(c));
    if (c.route.view === 'doc') {
      const here = c.route.id;
      cmd('subdoc', <Icon name="plus" size={14} />, 'Add a sub-page to this doc', () => newPage(c, here));
    }
    cmd('settings', <Icon name="gear" size={14} />, 'Open workspace settings', () => c.go({ view: 'settings' }));
    for (const team of Object.values(doc.team)) {
      cmd(`t${team.id}`, <span className="d-team-key">{team.key}</span>, `Go to ${team.name} issues`, () => c.go({ view: 'team', teamId: team.id, tab: 'all' }));
      cmd(`c${team.id}`, <Icon name="cycle" size={14} />, `Go to ${team.name} cycles`, () => c.go({ view: 'cycles', teamId: team.id }));
    }
    for (const p of Object.values(doc.project)) cmd(`p${p.id}`, <Icon name="project" size={14} />, `Project: ${p.name}`, () => c.go({ view: 'project', id: p.id }));
    const exact = findByKey(doc, q);
    const issues = Object.values(doc.issue)
      .filter((i) => !t || i === exact || i.title.toLowerCase().includes(t) || issueKey(doc, i).toLowerCase().includes(t))
      .sort((a, b) => (a === exact ? -1 : b === exact ? 1 : b.updatedAt - a.updatedAt))
      .slice(0, t ? 30 : 8);
    for (const i of issues) out.push({ key: i.id, icon: <StateIcon state={doc.state[i.stateId]} />, label: i.title, hint: issueKey(doc, i), run: () => c.open(i.id) });
    const pages = Object.values(doc.page)
      .filter((p) => !t || pageTitle(p).toLowerCase().includes(t) || (t.length > 2 && pageText(p.body).toLowerCase().includes(t)))
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, t ? 20 : 5);
    for (const p of pages) out.push({ key: `doc${p.id}`, icon: p.icon ? <span className="d-page-emoji">{p.icon}</span> : <Icon name="doc" size={14} />, label: pageTitle(p), hint: 'Doc', run: () => c.go({ view: 'doc', id: p.id }) });
    return out;
  }, [q, c.v]);
  const run = (r: Row | undefined) => {
    if (!r) return;
    onClose();
    r.run();
  };
  return (
    <Modal onClose={onClose} closing={closing} className="d-palette">
      <input
        className="d-palette-input"
        autoFocus
        placeholder="Search issues or type a command…"
        value={q}
        onChange={(e) => { setQ(e.currentTarget.value); setAt(0); }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown') { e.preventDefault(); setAt((a) => Math.min(rows.length - 1, a + 1)); }
          if (e.key === 'ArrowUp') { e.preventDefault(); setAt((a) => Math.max(0, a - 1)); }
          if (e.key === 'Enter') { e.preventDefault(); run(rows[at]); }
        }}
      />
      <div className="d-palette-list">
        {rows.map((r, n) => (
          <button type="button" key={r.key} className={`d-menu-item ${n === at ? 'hot' : ''}`} onMouseEnter={() => setAt(n)} onClick={() => run(r)}>
            {r.icon}
            <span className="d-grow d-ellipsis">{r.label}</span>
            {r.hint && <span className="d-tiny d-dim">{r.hint}</span>}
          </button>
        ))}
        {!rows.length && <div className="d-menu-empty">Nothing found</div>}
      </div>
    </Modal>
  );
}

export const statusLabel = (st: (typeof PROJECT_STATUSES)[number]) => st.charAt(0).toUpperCase() + st.slice(1);
