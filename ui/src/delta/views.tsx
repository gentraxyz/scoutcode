// Delta's other pages: projects, one project, the roadmap, a team's cycles, one cycle, and the workspace's settings.
import { useMemo, useState, type ReactNode } from 'react';
import { addDays, cycleName, cycleStatus, isDone, newId, PROJECT_STATUSES, progress, STATE_TYPES, teamStates, today, type Cycle, type Issue, type Project, type ProjectStatus, type StateType } from './model';
import { message, uploadWorkspace, type Role, type WorkspaceInfo } from './sync';
import { Empty, IssuesPage, statusLabel, useDelta } from './Delta';
import { cycleMenu, projectMenu } from './menus';
import { Avatar, ColorPicker, COLORS, Icon, openContextMenu, InlineText, Menu, MenuItem, Modal, peopleOptions, Picker, shortDay, StateIcon } from './ui';

const STATUS_COLORS: Record<ProjectStatus, string> = { planned: '#8b8d98', active: '#f5a524', paused: '#b0b4ba', completed: '#5e6ad2', canceled: '#6f6e77' };

export function ProjectStatusIcon({ status }: { status: ProjectStatus }) {
  const type: StateType = status === 'planned' ? 'backlog' : status === 'active' ? 'started' : status === 'paused' ? 'unstarted' : status;
  return <StateIcon state={{ type, color: STATUS_COLORS[status] }} />;
}

function Bar({ done, total, started = 0 }: { done: number; total: number; started?: number }) {
  const pct = total ? Math.round((done / total) * 100) : 0;
  return (
    <span className="d-progress" title={`${done} of ${total} done`}>
      <span className="d-progress-track">
        <span className="d-progress-started" style={{ width: `${total ? ((done + started) / total) * 100 : 0}%` }} />
        <span className="d-progress-done" style={{ width: `${pct}%` }} />
      </span>
      <span className="d-tiny d-dim">{pct}%</span>
    </span>
  );
}

function Head({ title, children, crumb }: { title: ReactNode; children?: ReactNode; crumb?: ReactNode }) {
  return (
    <header className="d-head">
      {crumb}
      <h1 className="d-ellipsis">{title}</h1>
      <span className="d-grow" />
      {children}
    </header>
  );
}

// ---------- projects ----------

export function ProjectsView() {
  const c = useDelta();
  const { s, go, v } = c;
  const doc = s.doc;
  const [making, setMaking] = useState(false);
  const [show, setShow] = useState<'open' | 'all'>('open');
  const projects = useMemo(() => Object.values(doc.project)
    .filter((p) => show === 'all' || (p.status !== 'completed' && p.status !== 'canceled'))
    .sort((a, b) => PROJECT_STATUSES.indexOf(a.status) - PROJECT_STATUSES.indexOf(b.status) || (a.targetDate ?? '9').localeCompare(b.targetDate ?? '9') || a.name.localeCompare(b.name)), [v, show]);
  return (
    <div className="d-page">
      <Head title="Projects">
        <div className="d-seg text">
          <button type="button" className={show === 'open' ? 'on' : ''} onClick={() => setShow('open')}>Open</button>
          <button type="button" className={show === 'all' ? 'on' : ''} onClick={() => setShow('all')}>All</button>
        </div>
        <button type="button" className="d-btn primary small" onClick={() => setMaking(true)}><Icon name="plus" size={13} />New project</button>
      </Head>
      {projects.length === 0 ? (
        <Empty icon="project" title="No projects" text="Projects group issues toward a goal, across teams, with a target date." action={<button type="button" className="d-btn primary" onClick={() => setMaking(true)}>New project</button>} />
      ) : (
        <div className="d-table">
          <div className="d-table-head"><span className="d-grow">Name</span><span className="w-lead">Lead</span><span className="w-date">Target</span><span className="w-issues">Issues</span><span className="w-prog">Progress</span></div>
          {projects.map((p) => {
            const issues = Object.values(doc.issue).filter((i) => i.projectId === p.id);
            const pr = progress(doc, issues);
            return (
              <button
                type="button"
                key={p.id}
                className="d-table-row"
                onClick={(e) => {
                  if (c.openTab && (e.ctrlKey || e.metaKey)) {
                    e.preventDefault();
                    c.openTab({ view: 'project', id: p.id });
                  } else {
                    go({ view: 'project', id: p.id });
                  }
                }}
                onAuxClick={(e) => {
                  if (e.button === 1 && c.openTab) {
                    e.preventDefault();
                    c.openTab({ view: 'project', id: p.id });
                  }
                }}
                onContextMenu={(e) => openContextMenu(e, projectMenu(c, p))}
              >
                <span className="d-grow d-row d-min0"><span className="d-project-color" style={{ background: p.color }} /><ProjectStatusIcon status={p.status} /><b className="d-ellipsis">{p.name}</b></span>
                <span className="w-lead d-row">{p.lead ? <><Avatar name={p.lead} size={16} /><span className="d-ellipsis">{p.lead}</span></> : <span className="d-dim">—</span>}</span>
                <span className={`w-date ${p.targetDate && p.targetDate < today() && p.status !== 'completed' ? 'd-late' : ''}`}>{shortDay(p.targetDate) || <span className="d-dim">—</span>}</span>
                <span className="w-issues d-dim">{issues.length}</span>
                <span className="w-prog"><Bar {...pr} /></span>
              </button>
            );
          })}
        </div>
      )}
      {making && <NewProject onClose={() => setMaking(false)} onMade={(id) => go({ view: 'project', id })} />}
    </div>
  );
}

function NewProject({ onClose, onMade }: { onClose: () => void; onMade: (id: string) => void }) {
  const { s } = useDelta();
  const doc = s.doc;
  const teams = Object.values(doc.team);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [teamIds, setTeamIds] = useState<string[]>(teams.length === 1 ? [teams[0].id] : []);
  const [lead, setLead] = useState<string | null>(s.me);
  const [startDate, setStart] = useState('');
  const [targetDate, setTarget] = useState('');
  const [color, setColor] = useState(COLORS[Object.keys(doc.project).length % COLORS.length]);
  const make = () => {
    if (!name.trim()) return;
    const id = newId();
    if (s.dispatch({ type: 'create', kind: 'project', id, data: { name: name.trim(), description, teamIds, lead, startDate: startDate || null, targetDate: targetDate || null, color, status: 'planned' } })) {
      onClose();
      onMade(id);
    }
  };
  return (
    <Modal onClose={onClose} className="d-new-issue">
      <div className="d-row"><b className="d-big">New project</b><span className="d-grow" /><button type="button" className="d-icon-btn" onClick={onClose} aria-label="Close"><Icon name="close" /></button></div>
      <div className="d-row"><ColorPicker value={color} onPick={setColor} /><input className="d-big-input" autoFocus placeholder="Project name" value={name} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && make()} /></div>
      <textarea className="d-body-input" placeholder="What's it for?" value={description} onChange={(e) => setDescription(e.currentTarget.value)} />
      <div className="d-row d-wrap">
        <Picker multi options={teams.map((t) => ({ value: t.id, label: t.name, icon: <span className="d-team-key">{t.key}</span> }))} value={teamIds} onPick={(t) => setTeamIds(teamIds.includes(t) ? teamIds.filter((x) => x !== t) : [...teamIds, t])} trigger={(_, t) => (
          <button type="button" className="d-chip-btn" onClick={t}><Icon name="users" size={13} />{teamIds.length ? teamIds.map((x) => doc.team[x]?.key).join(', ') : 'Teams'}</button>
        )} />
        <Picker options={peopleOptions(s.people, false)} value={lead} onPick={setLead} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><Avatar name={lead} size={16} />{lead ? `Lead: ${lead}` : 'Lead'}</button>} />
        <label className="d-chip-btn">Start <input type="date" className="d-date-input" value={startDate} onChange={(e) => setStart(e.currentTarget.value)} /></label>
        <label className="d-chip-btn">Target <input type="date" className="d-date-input" value={targetDate} onChange={(e) => setTarget(e.currentTarget.value)} /></label>
      </div>
      <div className="d-row d-modal-foot"><span className="d-grow" /><button type="button" className="d-btn primary" disabled={!name.trim()} onClick={make}>Create project</button></div>
    </Modal>
  );
}

export function ProjectView({ id }: { id: string }) {
  const { s, go } = useDelta();
  const doc = s.doc;
  const p = doc.project[id];
  const set = (patch: Partial<Project>) => s.dispatch({ type: 'update', kind: 'project', id, patch });
  const issues = Object.values(doc.issue).filter((i) => i.projectId === id);
  const pr = progress(doc, issues);
  const header = (
    <div className="d-project-head">
      <div className="d-row">
        <ColorPicker value={p.color} onPick={(color) => set({ color })} />
        <InlineText className="d-title-edit" value={p.name} onSave={(name) => set({ name })} />
      </div>
      <InlineText className="d-desc-inline" multiline value={p.description} placeholder="Add a description…" onSave={(description) => set({ description })} />
      <div className="d-row d-wrap">
        <Picker options={PROJECT_STATUSES.map((st) => ({ value: st, label: statusLabel(st), icon: <ProjectStatusIcon status={st} /> }))} value={p.status} onPick={(status) => set({ status })} trigger={(_, t) => (
          <button type="button" className="d-chip-btn" onClick={t}><ProjectStatusIcon status={p.status} />{statusLabel(p.status)}</button>
        )} />
        <Picker options={peopleOptions(s.people, false)} value={p.lead} onPick={(lead) => set({ lead })} trigger={(_, t) => <button type="button" className="d-chip-btn" onClick={t}><Avatar name={p.lead} size={16} />{p.lead ? `Lead: ${p.lead}` : 'No lead'}</button>} />
        <Picker multi options={Object.values(doc.team).map((t) => ({ value: t.id, label: t.name, icon: <span className="d-team-key">{t.key}</span> }))} value={p.teamIds} onPick={(t) => set({ teamIds: p.teamIds.includes(t) ? p.teamIds.filter((x) => x !== t) : [...p.teamIds, t] })} trigger={(_, t) => (
          <button type="button" className="d-chip-btn" onClick={t}><Icon name="users" size={13} />{p.teamIds.length ? p.teamIds.map((x) => doc.team[x]?.key).join(', ') : 'No teams'}</button>
        )} />
        <label className="d-chip-btn">Start <input type="date" className="d-date-input" value={p.startDate ?? ''} onChange={(e) => set({ startDate: e.currentTarget.value || null })} /></label>
        <label className="d-chip-btn">Target <input type="date" className="d-date-input" value={p.targetDate ?? ''} onChange={(e) => set({ targetDate: e.currentTarget.value || null })} /></label>
        <span className="d-grow" />
        <Bar {...pr} />
      </div>
    </div>
  );
  return (
    <div className="d-page">
      <Head crumb={<button type="button" className="d-crumb" onClick={() => go({ view: 'projects' })}>Projects ›</button>} title={p.name}>
        <Menu width={180} align="end" trigger={(_, t) => <button type="button" className="d-icon-btn" onClick={t} aria-label="More"><Icon name="more" /></button>}>
          {(close) => <MenuItem danger onClick={() => { close(); if (confirm(`Delete the project "${p.name}"? Its issues stay, without a project.`)) s.dispatch({ type: 'delete', kind: 'project', id }); }}><Icon name="trash" size={14} />Delete project</MenuItem>}
        </Menu>
      </Head>
      <IssuesPage embedded title={p.name} header={header} source={(i: Issue) => i.projectId === id} storageKey={`project:${id}`} defaults={{ projectId: id, teamId: p.teamIds[0] }} />
    </div>
  );
}

// ---------- roadmap ----------

/** Projects on a timeline of months, from their start to their target date. */
export function RoadmapView() {
  const c = useDelta();
  const { s, go, v } = c;
  const doc = s.doc;
  const now = today();
  const projects = useMemo(() => Object.values(doc.project).filter((p) => p.status !== 'canceled').sort((a, b) => (a.startDate ?? a.targetDate ?? '9').localeCompare(b.startDate ?? b.targetDate ?? '9')), [v]);
  const dated = projects.filter((p) => p.startDate || p.targetDate);
  const days = dated.flatMap((p) => [p.startDate, p.targetDate].filter((d): d is string => !!d));
  const first = [now, ...days].sort()[0];
  const last = [now, ...days].sort().pop()!;
  const start = new Date(`${first.slice(0, 7)}-01T12:00:00`);
  start.setMonth(start.getMonth() - 1);
  const end = new Date(`${last.slice(0, 7)}-01T12:00:00`);
  end.setMonth(end.getMonth() + 3);
  const months: Date[] = [];
  for (const d = new Date(start); d < end; d.setMonth(d.getMonth() + 1)) months.push(new Date(d));
  const total = (end.getTime() - start.getTime()) / 86_400_000;
  const x = (day: string) => ((new Date(`${day}T12:00:00`).getTime() - start.getTime()) / 86_400_000 / total) * 100;
  const MONTH_W = 120;
  return (
    <div className="d-page">
      <Head title="Roadmap" />
      {projects.length === 0 ? (
        <Empty icon="roadmap" title="Nothing on the roadmap" text="Give projects start and target dates to see them here." action={<button type="button" className="d-btn primary" onClick={() => go({ view: 'projects' })}>Go to projects</button>} />
      ) : (
        <div className="d-roadmap">
          <div className="d-roadmap-names">
            <div className="d-roadmap-corner" />
            {projects.map((p) => (
              <button
                type="button"
                key={p.id}
                className="d-roadmap-name"
                onClick={(e) => {
                  if (c.openTab && (e.ctrlKey || e.metaKey)) {
                    e.preventDefault();
                    c.openTab({ view: 'project', id: p.id });
                  } else {
                    go({ view: 'project', id: p.id });
                  }
                }}
                onAuxClick={(e) => {
                  if (e.button === 1 && c.openTab) {
                    e.preventDefault();
                    c.openTab({ view: 'project', id: p.id });
                  }
                }}
                onContextMenu={(e) => openContextMenu(e, projectMenu(c, p))}
              >
                <ProjectStatusIcon status={p.status} /><span className="d-ellipsis">{p.name}</span>
              </button>
            ))}
          </div>
          <div className="d-roadmap-scroll">
            <div className="d-roadmap-grid" style={{ width: months.length * MONTH_W }}>
              <div className="d-roadmap-months">
                {months.map((m) => <span key={m.toISOString()} style={{ width: MONTH_W }}>{m.toLocaleDateString(undefined, { month: 'short', ...(m.getMonth() === 0 || m === months[0] ? { year: 'numeric' } : {}) })}</span>)}
              </div>
              <div className="d-roadmap-today" style={{ left: `${x(now)}%` }} title="Today" />
              {projects.map((p) => {
                const issues = Object.values(doc.issue).filter((i) => i.projectId === p.id);
                const pr = progress(doc, issues);
                const a = p.startDate ?? (p.targetDate ? addDays(p.targetDate, -14) : null);
                const b = p.targetDate ?? (p.startDate ? addDays(p.startDate, 30) : null);
                return (
                  <div key={p.id} className="d-roadmap-row">
                    {a && b ? (
                      <button
                        type="button"
                        className={`d-roadmap-bar ${!p.startDate || !p.targetDate ? 'open' : ''}`}
                        style={{ left: `${x(a)}%`, width: `${Math.max(1.2, x(b) - x(a))}%`, ['--c' as string]: p.color }}
                        onClick={(e) => {
                          if (c.openTab && (e.ctrlKey || e.metaKey)) {
                            e.preventDefault();
                            c.openTab({ view: 'project', id: p.id });
                          } else {
                            go({ view: 'project', id: p.id });
                          }
                        }}
                        onAuxClick={(e) => {
                          if (e.button === 1 && c.openTab) {
                            e.preventDefault();
                            c.openTab({ view: 'project', id: p.id });
                          }
                        }}
                        onContextMenu={(e) => openContextMenu(e, projectMenu(c, p))}
                        title={`${p.name}: ${shortDay(p.startDate) || '?'} – ${shortDay(p.targetDate) || '?'}`}
                      >
                        <span className="d-roadmap-fill" style={{ width: `${pr.total ? (pr.done / pr.total) * 100 : 0}%` }} />
                        <span className="d-ellipsis">{p.name}</span>
                      </button>
                    ) : <span className="d-tiny d-dim d-roadmap-nodate">No dates</span>}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------- cycles ----------

export function CyclesView({ teamId }: { teamId: string }) {
  const ctx = useDelta();
  const { s, go, v } = ctx;
  const doc = s.doc;
  const team = doc.team[teamId];
  const cycles = useMemo(() => Object.values(doc.cycle).filter((c) => c.teamId === teamId).sort((a, b) => b.startsAt.localeCompare(a.startsAt)), [v, teamId]);
  const [making, setMaking] = useState(false);
  const groups: [string, Cycle[]][] = [
    ['Current', cycles.filter((c) => cycleStatus(c) === 'current')],
    ['Upcoming', cycles.filter((c) => cycleStatus(c) === 'upcoming').reverse()],
    ['Past', cycles.filter((c) => cycleStatus(c) === 'past')],
  ];
  return (
    <div className="d-page">
      <Head crumb={<span className="d-team-key">{team.key}</span>} title={`${team.name} cycles`}>
        <button type="button" className="d-btn primary small" onClick={() => setMaking(true)}><Icon name="plus" size={13} />New cycle</button>
      </Head>
      {cycles.length === 0 ? (
        <Empty icon="cycle" title="No cycles yet" text="Cycles are fixed stretches of time, like two-week sprints, that the team plans issues into." action={<button type="button" className="d-btn primary" onClick={() => setMaking(true)}>Start a cycle</button>} />
      ) : (
        <div className="d-scroll-body">
          {groups.map(([label, list]) => list.length > 0 && (
            <section key={label} className="d-group">
              <div className="d-group-head"><b>{label}</b><span className="d-dim">{list.length}</span></div>
              {list.map((c) => {
                const issues = Object.values(doc.issue).filter((i) => i.cycleId === c.id);
                const pr = progress(doc, issues);
                return (
                  <button
                    type="button"
                    key={c.id}
                    className="d-table-row"
                    onClick={(e) => {
                      if (ctx.openTab && (e.ctrlKey || e.metaKey)) {
                        e.preventDefault();
                        ctx.openTab({ view: 'cycle', id: c.id });
                      } else {
                        go({ view: 'cycle', id: c.id });
                      }
                    }}
                    onAuxClick={(e) => {
                      if (e.button === 1 && ctx.openTab) {
                        e.preventDefault();
                        ctx.openTab({ view: 'cycle', id: c.id });
                      }
                    }}
                    onContextMenu={(e) => openContextMenu(e, cycleMenu(ctx, c))}
                  >
                    <span className="d-grow d-row d-min0"><Icon name="cycle" size={14} /><b>{cycleName(c)}</b>{c.name && <span className="d-dim">Cycle {c.number}</span>}</span>
                    <span className="w-date2 d-dim">{shortDay(c.startsAt)} – {shortDay(c.endsAt)}</span>
                    <span className="w-issues d-dim">{issues.length} issues</span>
                    <span className="w-prog"><Bar {...pr} /></span>
                  </button>
                );
              })}
            </section>
          ))}
        </div>
      )}
      {making && <NewCycle teamId={teamId} onClose={() => setMaking(false)} />}
    </div>
  );
}

function NewCycle({ teamId, onClose }: { teamId: string; onClose: () => void }) {
  const { s, go } = useDelta();
  const last = Object.values(s.doc.cycle).filter((c) => c.teamId === teamId).sort((a, b) => b.endsAt.localeCompare(a.endsAt))[0];
  const first = last && last.endsAt >= today() ? addDays(last.endsAt, 1) : today();
  const [startsAt, setStart] = useState(first);
  const [weeks, setWeeks] = useState(2);
  const [endsAt, setEnd] = useState(addDays(first, 13));
  const [name, setName] = useState('');
  const make = () => {
    const id = newId();
    if (s.dispatch({ type: 'create', kind: 'cycle', id, data: { teamId, startsAt, endsAt, name } })) {
      onClose();
      go({ view: 'cycle', id });
    }
  };
  return (
    <Modal onClose={onClose} className="d-small-modal">
      <b className="d-big">New cycle</b>
      <label className="d-field"><span>Name (optional)</span><input className="d-input" placeholder={`Cycle ${s.doc.team[teamId].nextCycle}`} value={name} onChange={(e) => setName(e.currentTarget.value)} /></label>
      <div className="d-field-row">
        <label className="d-field d-grow"><span>Starts</span><input type="date" className="d-input" value={startsAt} onChange={(e) => { setStart(e.currentTarget.value); setEnd(addDays(e.currentTarget.value, weeks * 7 - 1)); }} /></label>
        <label className="d-field"><span>Length</span>
          <select className="d-input" value={weeks} onChange={(e) => { const w = Number(e.currentTarget.value); setWeeks(w); setEnd(addDays(startsAt, w * 7 - 1)); }}>
            {[1, 2, 3, 4, 6, 8].map((w) => <option key={w} value={w}>{w} {w === 1 ? 'week' : 'weeks'}</option>)}
          </select>
        </label>
        <label className="d-field d-grow"><span>Ends</span><input type="date" className="d-input" value={endsAt} onChange={(e) => setEnd(e.currentTarget.value)} /></label>
      </div>
      <div className="d-row d-end"><button type="button" className="d-btn" onClick={onClose}>Cancel</button><button type="button" className="d-btn primary" onClick={make}>Create cycle</button></div>
    </Modal>
  );
}

export function CycleView({ id }: { id: string }) {
  const { s, go } = useDelta();
  const doc = s.doc;
  const c = doc.cycle[id];
  const team = doc.team[c.teamId];
  const issues = Object.values(doc.issue).filter((i) => i.cycleId === id);
  const pr = progress(doc, issues);
  const status = cycleStatus(c);
  const left = Math.max(0, Math.round((new Date(`${c.endsAt}T12:00:00`).getTime() - new Date(`${today()}T12:00:00`).getTime()) / 86_400_000));
  const unfinished = issues.filter((i) => !isDone(doc.state[i.stateId]));
  const next = Object.values(doc.cycle).filter((x) => x.teamId === c.teamId && x.startsAt > c.endsAt).sort((a, b) => a.startsAt.localeCompare(b.startsAt))[0];
  const header = (
    <div className="d-cycle-head">
      <div className="d-cycle-stats">
        <div><span className="d-tiny d-dim">Dates</span><b>{shortDay(c.startsAt)} – {shortDay(c.endsAt)}</b></div>
        <div><span className="d-tiny d-dim">{status === 'current' ? 'Days left' : 'Status'}</span><b>{status === 'current' ? left : status === 'past' ? 'Finished' : 'Upcoming'}</b></div>
        <div><span className="d-tiny d-dim">Scope</span><b>{pr.total} {issues.some((i) => i.estimate != null) ? 'points' : 'issues'}</b></div>
        <div><span className="d-tiny d-dim">Done</span><b>{pr.done} <span className="d-dim">({pr.total ? Math.round((pr.done / pr.total) * 100) : 0}%)</span></b></div>
        <div><span className="d-tiny d-dim">Started</span><b>{pr.started}</b></div>
      </div>
      <Burndown c={c} issues={issues} />
      {status === 'past' && unfinished.length > 0 && next && (
        <div className="d-row d-note">
          <span className="d-grow">{unfinished.length} unfinished {unfinished.length === 1 ? 'issue' : 'issues'} left in this cycle.</span>
          <button type="button" className="d-btn small" onClick={() => s.dispatch(unfinished.map((i) => ({ type: 'update' as const, kind: 'issue' as const, id: i.id, patch: { cycleId: next.id } })))}>Move to {cycleName(next)}</button>
        </div>
      )}
    </div>
  );
  return (
    <div className="d-page">
      <Head crumb={<button type="button" className="d-crumb" onClick={() => go({ view: 'cycles', teamId: c.teamId })}><span className="d-team-key">{team.key}</span> Cycles ›</button>} title={cycleName(c)}>
        <Menu width={200} align="end" trigger={(_, t) => <button type="button" className="d-icon-btn" onClick={t} aria-label="More"><Icon name="more" /></button>}>
          {(close) => (
            <>
              <MenuItem onClick={() => { close(); const n = prompt('Cycle name', c.name); if (n != null) s.dispatch({ type: 'update', kind: 'cycle', id, patch: { name: n } }); }}>Rename</MenuItem>
              <MenuItem danger onClick={() => { close(); if (confirm(`Delete ${cycleName(c)}? Its issues stay, without a cycle.`)) s.dispatch({ type: 'delete', kind: 'cycle', id }); }}><Icon name="trash" size={14} />Delete cycle</MenuItem>
            </>
          )}
        </Menu>
      </Head>
      <IssuesPage embedded title={cycleName(c)} header={header} source={(i: Issue) => i.cycleId === id} storageKey={`cycle:${c.teamId}`} defaults={{ cycleId: id, teamId: c.teamId }} />
    </div>
  );
}

/** Work left each day of the cycle against an even line down to zero. Scope counts issues by when they were made. */
function Burndown({ c, issues }: { c: Cycle; issues: Issue[] }) {
  const { s } = useDelta();
  const doc = s.doc;
  const days: string[] = [];
  for (let d = c.startsAt; d <= c.endsAt && days.length < 120; d = addDays(d, 1)) days.push(d);
  if (days.length < 2) return null;
  const w = (i: Issue) => i.estimate ?? 1;
  const live = issues.filter((i) => doc.state[i.stateId]?.type !== 'canceled');
  const end = (d: string) => new Date(`${d}T23:59:59`).getTime();
  const now = today();
  const left = days.map((d) => (d > now ? null : live.filter((i) => i.createdAt <= end(d) || d === days[0]).reduce((n, i) => n + (i.completedAt && i.completedAt <= end(d) && doc.state[i.stateId]?.type === 'completed' ? 0 : w(i)), 0)));
  const scope = days.map((d) => live.filter((i) => i.createdAt <= end(d) || d === days[0]).reduce((n, i) => n + w(i), 0));
  const max = Math.max(1, ...scope);
  const W = 600, H = 140, P = 6;
  const x = (n: number) => P + (n / (days.length - 1)) * (W - 2 * P);
  const y = (v: number) => H - P - (v / max) * (H - 2 * P);
  const line = left.map((v, n) => (v == null ? '' : `${n === 0 || left[n - 1] == null ? 'M' : 'L'}${x(n)},${y(v)}`)).join('');
  return (
    <div className="d-burndown">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label="Burndown chart">
        <line x1={x(0)} y1={y(scope[0])} x2={x(days.length - 1)} y2={y(0)} className="d-bd-ideal" />
        <path d={scope.map((v, n) => `${n ? 'L' : 'M'}${x(n)},${y(v)}`).join('')} className="d-bd-scope" />
        <path d={line} className="d-bd-left" />
      </svg>
      <div className="d-row d-tiny d-dim"><span className="d-legend left" />Remaining<span className="d-legend scope" />Scope<span className="d-legend ideal" />Ideal<span className="d-grow" />{shortDay(days[0])} – {shortDay(days[days.length - 1])}</div>
    </div>
  );
}

// ---------- settings ----------

export function SettingsView({ onGone, onMade }: { onGone: () => void; onMade: (w: WorkspaceInfo) => void }) {
  const { s, host } = useDelta();
  const doc = s.doc;
  const admin = s.canManage;
  const cloud = s.info.storage === 'cloud';
  const [who, setWho] = useState('');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<[boolean, string] | null>(null);
  const run = async (f: () => Promise<void>, ok?: string) => {
    setBusy(true);
    setNote(null);
    try {
      await f();
      if (ok) setNote([true, ok]);
    } catch (e) {
      setNote([false, message(e)]);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="d-page">
      <Head title="Settings" />
      <div className="d-scroll-body d-settings">
        <section className="d-card-section">
          <h2>Workspace</h2>
          <label className="d-field"><span>Name</span><InlineText className="d-input" value={doc.workspace.name} onSave={(name) => s.dispatch({ type: 'update', kind: 'workspace', id: 'ws', patch: { name } })} /></label>
          <div className="d-row d-small d-muted">
            <Icon name={cloud ? 'cloud' : 'laptop'} size={14} />
            {cloud ? 'Kept on ScoutCode Cloud. Everyone in it sees changes within a few seconds.' : 'Kept in a file on this computer (in .scoutcode\\delta). Only you can see it.'}
          </div>
          {!cloud && host.me && (
            <div className="d-row">
              <button type="button" className="d-btn" disabled={busy} onClick={() => run(async () => onMade(await uploadWorkspace(host, doc)))}><Icon name="upload" size={14} />Copy to ScoutCode Cloud</button>
              <span className="d-small d-dim">Makes a cloud copy you can share. This one stays on your computer.</span>
            </div>
          )}
        </section>

        <section className="d-card-section">
          <h2>Members</h2>
          {s.members.map((m) => (
            <div key={m.username} className="d-row d-member">
              <Avatar name={m.username} size={22} />
              <b className="d-grow">{m.username}{m.username === s.me && <span className="d-dim"> (you)</span>}</b>
              {cloud && admin && m.role !== 'owner' ? (
                <select className="d-input small" value={m.role} disabled={busy} onChange={(e) => run(() => s.admin('role', { username: m.username, role: e.currentTarget.value as Role }))}>
                  <option value="member">Member</option>
                  <option value="admin">Admin</option>
                </select>
              ) : <span className="d-dim d-small">{m.role.charAt(0).toUpperCase() + m.role.slice(1)}</span>}
              {cloud && m.role !== 'owner' && (admin || m.username === s.me) && (
                <button type="button" className="d-btn small ghost" disabled={busy} onClick={() => {
                  if (!confirm(m.username === s.me ? 'Leave this workspace?' : `Remove ${m.username}?`)) return;
                  run(async () => { await s.admin('remove', { username: m.username }); if (m.username === s.me) onGone(); });
                }}>{m.username === s.me ? 'Leave' : 'Remove'}</button>
              )}
            </div>
          ))}
          {cloud && admin && (
            <div className="d-row">
              <input className="d-input d-grow" placeholder="ScoutCode username" value={who} onChange={(e) => setWho(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === 'Enter' && who.trim()) run(async () => { await s.admin('invite', { username: who.trim() }); setWho(''); }, 'Added.'); }} />
              <button type="button" className="d-btn primary" disabled={busy || !who.trim()} onClick={() => run(async () => { await s.admin('invite', { username: who.trim() }); setWho(''); }, 'Added.')}>Add member</button>
            </div>
          )}
          {!cloud && <p className="d-small d-dim">Local workspaces are just for you. Copy it to ScoutCode Cloud to work on it with others.</p>}
          {note && <div className={note[0] ? 'd-ok' : 'd-error'}>{note[1]}</div>}
        </section>

        <TeamsSettings />
        <LabelsSettings />

        <section className="d-card-section danger">
          <h2>Data</h2>
          <div className="d-row d-wrap">
            <button type="button" className="d-btn" onClick={() => {
              const blob = new Blob([s.exportJson()], { type: 'application/json' });
              const a = document.createElement('a');
              a.href = URL.createObjectURL(blob);
              a.download = `${doc.workspace.name.replace(/[^\w.-]+/g, '-')}-delta.json`;
              a.click();
              setTimeout(() => URL.revokeObjectURL(a.href), 1000);
            }}><Icon name="download" size={14} />Export as JSON</button>
            {(s.role === 'owner' || !cloud) && (
              <button type="button" className="d-btn danger" disabled={busy} onClick={() => {
                if (prompt(`This deletes "${doc.workspace.name}" and every issue in it${cloud ? ' for everyone' : ''}. Type the workspace's name to confirm.`) !== doc.workspace.name) return;
                run(async () => { await s.destroy(); onGone(); });
              }}><Icon name="trash" size={14} />Delete workspace</button>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}

function TeamsSettings() {
  const { s } = useDelta();
  const doc = s.doc;
  const admin = s.canManage;
  const teams = Object.values(doc.team).sort((a, b) => a.createdAt - b.createdAt);
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [open, setOpen] = useState<string | null>(null);
  const add = () => {
    if (name.trim() && s.dispatch({ type: 'create', kind: 'team', id: newId(), data: { name: name.trim(), ...(key ? { key } : {}) } })) {
      setName('');
      setKey('');
    }
  };
  return (
    <section className="d-card-section">
      <h2>Teams</h2>
      {!admin && <p className="d-small d-dim">Only workspace admins can change teams and workflows.</p>}
      {teams.map((t) => {
        const states = teamStates(doc, t.id);
        const count = Object.values(doc.issue).filter((i) => i.teamId === t.id).length;
        return (
          <div key={t.id} className="d-team-settings">
            <div className="d-row">
              <span className="d-team-key">{t.key}</span>
              {admin ? <InlineText className="d-inline-name" value={t.name} onSave={(n) => s.dispatch({ type: 'update', kind: 'team', id: t.id, patch: { name: n } })} /> : <b className="d-grow">{t.name}</b>}
              <span className="d-small d-dim">{count} issues</span>
              <button type="button" className="d-btn small ghost" onClick={() => setOpen(open === t.id ? null : t.id)}>Workflow<Icon name={open === t.id ? 'chevronDown' : 'chevronRight'} size={12} /></button>
              {admin && (
                <Menu width={200} align="end" trigger={(_, tg) => <button type="button" className="d-icon-btn" onClick={tg} aria-label="Team actions"><Icon name="more" size={14} /></button>}>
                  {(close) => (
                    <>
                      <MenuItem onClick={() => { close(); const k = prompt('Team key (shows in issue IDs, like ENG-12)', t.key); if (k) s.dispatch({ type: 'update', kind: 'team', id: t.id, patch: { key: k } }); }}>Change key</MenuItem>
                      <MenuItem danger onClick={() => { close(); if (confirm(`Delete the team ${t.name}?`)) s.dispatch({ type: 'delete', kind: 'team', id: t.id }); }}>Delete team</MenuItem>
                    </>
                  )}
                </Menu>
              )}
            </div>
            {open === t.id && (
              <div className="d-states">
                {states.map((st) => (
                  <div key={st.id} className="d-row d-state-row">
                    <StateIcon state={st} />
                    {admin ? <InlineText className="d-inline-name" value={st.name} onSave={(n) => s.dispatch({ type: 'update', kind: 'state', id: st.id, patch: { name: n } })} /> : <span className="d-grow">{st.name}</span>}
                    {admin && <ColorPicker value={st.color} onPick={(color) => s.dispatch({ type: 'update', kind: 'state', id: st.id, patch: { color } })} />}
                    <span className="d-tiny d-dim w-type">{st.type}</span>
                    {admin && <button type="button" className="d-icon-btn" title="Delete state" onClick={() => s.dispatch({ type: 'delete', kind: 'state', id: st.id })}><Icon name="trash" size={13} /></button>}
                  </div>
                ))}
                {admin && (
                  <Menu width={200} trigger={(_, tg) => <button type="button" className="d-btn small ghost" onClick={tg}><Icon name="plus" size={13} />Add state</button>}>
                    {(close) => STATE_TYPES.map((type) => (
                      <MenuItem key={type} onClick={() => { close(); const n = prompt(`Name of the new ${type} state`); if (n?.trim()) s.dispatch({ type: 'create', kind: 'state', id: newId(), data: { teamId: t.id, name: n.trim(), type } }); }}>
                        <StateIcon state={{ type, color: '#8b8d98' }} />{type.charAt(0).toUpperCase() + type.slice(1)}
                      </MenuItem>
                    ))}
                  </Menu>
                )}
              </div>
            )}
          </div>
        );
      })}
      {admin && (
        <div className="d-row">
          <input className="d-input d-grow" placeholder="New team name" value={name} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && add()} />
          <input className="d-input" style={{ width: 90 }} placeholder="KEY" maxLength={8} value={key} onChange={(e) => setKey(e.currentTarget.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))} />
          <button type="button" className="d-btn" disabled={!name.trim()} onClick={add}>Add team</button>
        </div>
      )}
    </section>
  );
}

function LabelsSettings() {
  const { s } = useDelta();
  const labels = Object.values(s.doc.label).sort((a, b) => a.name.localeCompare(b.name));
  const [name, setName] = useState('');
  const [color, setColor] = useState(COLORS[labels.length % COLORS.length]);
  const add = () => {
    if (name.trim() && s.dispatch({ type: 'create', kind: 'label', id: newId(), data: { name: name.trim(), color } })) {
      setName('');
      setColor(COLORS[(labels.length + 1) % COLORS.length]);
    }
  };
  return (
    <section className="d-card-section">
      <h2>Labels</h2>
      <div className="d-labels">
        {labels.map((l) => (
          <div key={l.id} className="d-row d-label-row">
            <ColorPicker value={l.color} onPick={(c) => s.dispatch({ type: 'update', kind: 'label', id: l.id, patch: { color: c } })} />
            <InlineText className="d-inline-name" value={l.name} onSave={(n) => s.dispatch({ type: 'update', kind: 'label', id: l.id, patch: { name: n } })} />
            <span className="d-small d-dim">{Object.values(s.doc.issue).filter((i) => i.labelIds.includes(l.id)).length}</span>
            <button type="button" className="d-icon-btn" title="Delete label" onClick={() => { if (confirm(`Delete the label ${l.name}?`)) s.dispatch({ type: 'delete', kind: 'label', id: l.id }); }}><Icon name="trash" size={13} /></button>
          </div>
        ))}
      </div>
      <div className="d-row">
        <ColorPicker value={color} onPick={setColor} />
        <input className="d-input d-grow" placeholder="New label" value={name} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && add()} />
        <button type="button" className="d-btn" disabled={!name.trim()} onClick={add}>Add label</button>
      </div>
    </section>
  );
}

