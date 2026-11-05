// One issue, in a panel over the page: its title and description, sub-issues, comments and history on the left, and its
// properties on the right. "Hand to Scout" starts a Scout chat on it in one of the user's projects.
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { imageHandlers } from './images';
import { agentPrompt, cycleName, cycleStatus, isDone, issueKey, newId, PRIORITIES, SCOUT, teamStates, type Activity, type Comment, type Issue } from './model';
import { message, type AgentTarget } from './sync';
import { useDelta, IssueRow, byOrder } from './Delta';
import { ago, Avatar, Icon, InlineText, LabelDot, labelOptions, Menu, MenuItem, Modal, peopleOptions, personName, Picker, PriorityIcon, priorityOptions, RichText, shortDay, StateIcon, stateOptions } from './ui';

export function IssueDetail({ id, onClose, closing }: { id: string; onClose: () => void; closing?: boolean }) {
  const c = useDelta();
  const { s } = c;
  const doc = s.doc;
  const i = doc.issue[id];
  const [handing, setHanding] = useState(false);
  const [editing, setEditing] = useState(false);
  if (!i) return null;
  const set = (patch: Partial<Issue> & { teamId?: string }) => s.dispatch({ type: 'update', kind: 'issue', id, patch });
  const st = doc.state[i.stateId];
  const team = doc.team[i.teamId];
  const parent = i.parentId ? doc.issue[i.parentId] : null;
  const subs = Object.values(doc.issue).filter((x) => x.parentId === id).sort(byOrder);
  const cycles = Object.values(doc.cycle).filter((x) => x.teamId === i.teamId && (cycleStatus(x) !== 'past' || x.id === i.cycleId)).sort((a, b) => a.startsAt.localeCompare(b.startsAt));
  const key = issueKey(doc, i);

  return (
    <div className={`d-drawer-back ${closing ? 'closing' : ''}`} onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <aside className="d-drawer" role="dialog" aria-label={key}>
        <div className="d-drawer-head">
          <span className="d-team-key">{team?.key}</span>
          <span className="d-dim">›</span>
          <span className="d-key">{key}</span>
          <button type="button" className="d-icon-btn" title="Copy ID" onClick={() => navigator.clipboard?.writeText(key)}><Icon name="copy" size={13} /></button>
          <span className="d-grow" />
          {c.host.kind === 'desktop' && c.host.agent && <button type="button" className="d-btn small scout" onClick={() => setHanding(true)}><Icon name="sparkle" size={13} />Hand to Scout</button>}
          <Menu width={200} align="end" trigger={(_, t) => <button type="button" className="d-icon-btn" onClick={t} aria-label="More"><Icon name="more" /></button>}>
            {(close) => (
              <>
                <MenuItem onClick={() => { close(); c.create({ teamId: i.teamId, parentId: i.id, projectId: i.projectId, cycleId: i.cycleId }); }}><Icon name="parent" size={14} />Add sub-issue</MenuItem>
                <MenuItem onClick={() => { close(); navigator.clipboard?.writeText(`${key} ${i.title}`); }}><Icon name="copy" size={14} />Copy ID and title</MenuItem>
                <hr />
                <MenuItem danger onClick={() => { close(); if (confirm(`Delete ${key}? Its comments and history go too.`)) { s.dispatch({ type: 'delete', kind: 'issue', id }); onClose(); } }}><Icon name="trash" size={14} />Delete issue</MenuItem>
              </>
            )}
          </Menu>
          <button type="button" className="d-icon-btn" onClick={onClose} aria-label="Close"><Icon name="close" /></button>
        </div>
        <div className="d-drawer-body">
          <div className="d-detail-main">
            {parent && (
              <button type="button" className="d-parent" onClick={() => c.open(parent.id)}>
                <Icon name="parent" size={13} /><span className="d-dim">Sub-issue of</span><StateIcon state={doc.state[parent.stateId]} size={12} /><span className="d-key">{issueKey(doc, parent)}</span><span className="d-ellipsis">{parent.title}</span>
              </button>
            )}
            <InlineText className="d-title-edit" value={i.title} onSave={(t) => set({ title: t })} />
            {editing ? (
              <Description value={i.description} onDone={(v) => { setEditing(false); if (v !== i.description) set({ description: v }); }} />
            ) : (
              <div className="d-desc" role="button" tabIndex={0} onClick={(e) => { if (!(e.target as HTMLElement).closest('a')) setEditing(true); }} onKeyDown={(e) => e.key === 'Enter' && setEditing(true)}>
                {i.description.trim() ? <RichText text={i.description} images={doc.image} /> : <span className="d-dim">Add a description…</span>}
              </div>
            )}
            {i.agent && <AgentNote i={i} />}
            <section className="d-subs">
              <div className="d-row d-sub-head">
                <b>Sub-issues</b>
                {subs.length > 0 && <span className="d-dim">{subs.filter((x) => isDone(doc.state[x.stateId])).length}/{subs.length}</span>}
                <span className="d-grow" />
                <button type="button" className="d-btn ghost small" onClick={() => c.create({ teamId: i.teamId, parentId: i.id, projectId: i.projectId, cycleId: i.cycleId })}><Icon name="plus" size={13} />Add</button>
              </div>
              {subs.map((x) => <IssueRow key={x.id} i={x} compact />)}
            </section>
            <Timeline issueId={id} />
          </div>
          <div className="d-props">
            <Prop label="Status">
              <Picker options={stateOptions(teamStates(doc, i.teamId))} value={i.stateId} onPick={(v) => set({ stateId: v })} trigger={(_, t) => <button type="button" className="d-prop-btn" onClick={t}><StateIcon state={st} />{st?.name}</button>} />
            </Prop>
            <Prop label="Priority">
              <Picker options={priorityOptions()} value={i.priority} onPick={(p) => set({ priority: p })} trigger={(_, t) => <button type="button" className="d-prop-btn" onClick={t}><PriorityIcon p={i.priority} />{PRIORITIES[i.priority]}</button>} />
            </Prop>
            <Prop label="Assignee">
              <Picker options={peopleOptions(s.people)} value={i.assignee} onPick={(u) => set({ assignee: u })} trigger={(_, t) => <button type="button" className="d-prop-btn" onClick={t}><Avatar name={i.assignee} />{personName(i.assignee)}</button>} />
            </Prop>
            <Prop label="Labels">
              <div className="d-row d-wrap">
                {i.labelIds.map((l) => doc.label[l] && <span key={l} className="d-chip"><LabelDot color={doc.label[l].color} />{doc.label[l].name}</span>)}
                <Picker multi options={labelOptions(doc)} value={i.labelIds} onPick={(l) => set({ labelIds: i.labelIds.includes(l) ? i.labelIds.filter((x) => x !== l) : [...i.labelIds, l] })} trigger={(_, t) => (
                  <button type="button" className="d-prop-btn" onClick={t}>{i.labelIds.length ? <Icon name="plus" size={13} /> : <><Icon name="tag" size={13} />Add label</>}</button>
                )} />
              </div>
            </Prop>
            <Prop label="Project">
              <Picker options={[{ value: null as string | null, label: 'No project', icon: <Icon name="project" size={14} /> }, ...Object.values(doc.project).map((p) => ({ value: p.id as string | null, label: p.name, icon: <Icon name="project" size={14} /> }))]} value={i.projectId} onPick={(p) => set({ projectId: p })} trigger={(_, t) => (
                <button type="button" className="d-prop-btn" onClick={t}><Icon name="project" size={14} />{i.projectId ? doc.project[i.projectId]?.name : <span className="d-dim">Add to project</span>}</button>
              )} />
            </Prop>
            <Prop label="Cycle">
              <Picker options={[{ value: null as string | null, label: 'No cycle' }, ...cycles.map((x) => ({ value: x.id as string | null, label: cycleName(x), hint: cycleStatus(x) === 'current' ? 'current' : `${shortDay(x.startsAt)} – ${shortDay(x.endsAt)}` }))]} value={i.cycleId} onPick={(x) => set({ cycleId: x })} trigger={(_, t) => (
                <button type="button" className="d-prop-btn" onClick={t}><Icon name="cycle" size={14} />{i.cycleId && doc.cycle[i.cycleId] ? cycleName(doc.cycle[i.cycleId]) : <span className="d-dim">Add to cycle</span>}</button>
              )} />
            </Prop>
            <Prop label="Estimate">
              <Picker options={[{ value: null as number | null, label: 'No estimate' }, ...[1, 2, 3, 5, 8, 13].map((n) => ({ value: n as number | null, label: `${n} ${n === 1 ? 'point' : 'points'}` }))]} value={i.estimate} onPick={(n) => set({ estimate: n })} trigger={(_, t) => (
                <button type="button" className="d-prop-btn" onClick={t}><Icon name="estimate" size={14} />{i.estimate != null ? `${i.estimate} ${i.estimate === 1 ? 'point' : 'points'}` : <span className="d-dim">Estimate</span>}</button>
              )} />
            </Prop>
            <Prop label="Due date">
              <label className="d-prop-btn"><Icon name="calendar" size={14} /><input type="date" className="d-date-input" value={i.dueDate ?? ''} onChange={(e) => set({ dueDate: e.currentTarget.value || null })} /></label>
            </Prop>
            <Prop label="Parent">
              <Picker
                options={[{ value: null as string | null, label: 'No parent' }, ...Object.values(doc.issue).filter((x) => x.id !== i.id && x.parentId !== i.id).sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 200).map((x) => ({ value: x.id as string | null, label: x.title, hint: issueKey(doc, x) }))]}
                value={i.parentId}
                onPick={(p) => set({ parentId: p })}
                width={300}
                trigger={(_, t) => <button type="button" className="d-prop-btn" onClick={t}><Icon name="parent" size={14} />{parent ? issueKey(doc, parent) : <span className="d-dim">Set parent</span>}</button>}
              />
            </Prop>
            {Object.keys(doc.team).length > 1 && (
              <Prop label="Team">
                <Picker options={Object.values(doc.team).map((t) => ({ value: t.id, label: t.name, icon: <span className="d-team-key">{t.key}</span> }))} value={i.teamId} onPick={(t) => { if (t !== i.teamId) set({ teamId: t }); }} trigger={(_, t) => (
                  <button type="button" className="d-prop-btn" onClick={t}><span className="d-team-key">{team?.key}</span>{team?.name}</button>
                )} />
              </Prop>
            )}
            <div className="d-props-foot d-tiny d-dim">
              Opened by {personName(i.creator)} {ago(i.createdAt)}
              <br />
              Updated {ago(i.updatedAt)}
            </div>
          </div>
        </div>
      </aside>
      {handing && <HandToScout i={i} onClose={() => setHanding(false)} />}
    </div>
  );
}

function Prop({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="d-prop">
      <span className="d-prop-label">{label}</span>
      <div className="d-prop-value">{children}</div>
    </div>
  );
}

function Description({ value, onDone }: { value: string; onDone: (v: string) => void }) {
  const { s } = useDelta();
  const [draft, setDraft] = useState(value);
  const box = useRef<HTMLTextAreaElement>(null);
  const img = imageHandlers(s, box, setDraft);
  return (
    <div className="d-stack">
      <textarea
        ref={box}
        className="d-desc-edit"
        autoFocus
        onPaste={img.onPaste}
        onDrop={img.onDrop}
        onDragOver={img.onDragOver}
        value={draft}
        placeholder="Add a description…"
        onChange={(e) => setDraft(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onDone(value); }
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); onDone(draft); }
        }}
      />
      <div className="d-row d-end">
        <button type="button" className="d-btn small ghost" onClick={img.pick} title="Add a picture (or paste or drop one)"><Icon name="image" size={13} />Image</button>
        <span className="d-grow" />
        <span className="d-tiny d-dim">Ctrl+Enter to save</span>
        <button type="button" className="d-btn small" onClick={() => onDone(value)}>Cancel</button>
        <button type="button" className="d-btn small primary" onClick={() => onDone(draft)}>Save</button>
      </div>
    </div>
  );
}

function AgentNote({ i }: { i: Issue }) {
  const { s, host } = useDelta();
  const a = i.agent!;
  const project = a.cwd.split(/[\\/]/).filter(Boolean).pop() ?? a.cwd;
  return (
    <div className="d-agent">
      <Avatar name={SCOUT} size={22} />
      <div className="d-grow d-min0">
        <div><b>Scout</b> is working on this in <b>{project}</b>{a.pc ? <> on <b>{a.pc}</b></> : ''}.</div>
        <div className="d-tiny d-dim">Handed over by {personName(a.by)} {ago(a.startedAt)}</div>
      </div>
      {host.agent?.open && <button type="button" className="d-btn small" onClick={() => host.agent!.open!(a)}>Open chat</button>}
      <button type="button" className="d-btn small ghost" title="Stop showing Scout on this issue" onClick={() => s.dispatch({ type: 'update', kind: 'issue', id: i.id, patch: { agent: null } })}>Clear</button>
    </div>
  );
}

/** Comments and the issue's history, oldest first, with a box for a new comment. */
function Timeline({ issueId }: { issueId: string }) {
  const { s, v } = useDelta();
  const doc = s.doc;
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  const img = imageHandlers(s, box, setText);
  const entries = useMemo(() => {
    const out: ({ t: 'c'; at: number; c: Comment } | { t: 'a'; at: number; a: Activity })[] = [];
    for (const c of Object.values(doc.comment)) if (c.issueId === issueId) out.push({ t: 'c', at: c.createdAt, c });
    for (const a of Object.values(doc.activity)) if (a.issueId === issueId) out.push({ t: 'a', at: a.at, a });
    return out.sort((x, y) => x.at - y.at);
  }, [v, issueId]);
  const post = () => {
    if (!text.trim()) return;
    if (s.dispatch({ type: 'create', kind: 'comment', id: newId(), data: { issueId, body: text.trim() } })) setText('');
  };
  return (
    <section className="d-timeline">
      <b className="d-section-title">Activity</b>
      {entries.map((e) => (e.t === 'c' ? <CommentView key={e.c.id} c={e.c} /> : <ActivityLine key={e.a.id} a={e.a} />))}
      <div className="d-comment-box">
        <textarea ref={box} placeholder="Leave a comment… (paste or drop pictures)" onPaste={img.onPaste} onDrop={img.onDrop} onDragOver={img.onDragOver} value={text} onChange={(e) => setText(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); post(); } }} />
        <div className="d-row d-end">
          <button type="button" className="d-icon-btn" onClick={img.pick} title="Add a picture"><Icon name="image" size={14} /></button>
          <span className="d-grow" />
          <span className="d-tiny d-dim">Ctrl+Enter</span>
          <button type="button" className="d-btn small primary" disabled={!text.trim()} onClick={post}>Comment</button>
        </div>
      </div>
    </section>
  );
}

function CommentView({ c }: { c: Comment }) {
  const { s } = useDelta();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.body);
  const box = useRef<HTMLTextAreaElement>(null);
  const img = imageHandlers(s, box, setDraft);
  const mine = c.author === s.me;
  return (
    <div className="d-comment">
      <div className="d-row">
        <Avatar name={c.author} size={18} />
        <b>{personName(c.author)}</b>
        <span className="d-tiny d-dim">{ago(c.createdAt)}{c.editedAt ? ' · edited' : ''}</span>
        <span className="d-grow" />
        {mine && !editing && (
          <Menu width={140} align="end" trigger={(_, t) => <button type="button" className="d-icon-btn" onClick={t} aria-label="Comment actions"><Icon name="more" size={14} /></button>}>
            {(close) => (
              <>
                <MenuItem onClick={() => { close(); setDraft(c.body); setEditing(true); }}>Edit</MenuItem>
                <MenuItem danger onClick={() => { close(); s.dispatch({ type: 'delete', kind: 'comment', id: c.id }); }}>Delete</MenuItem>
              </>
            )}
          </Menu>
        )}
      </div>
      {editing ? (
        <div className="d-stack">
          <textarea ref={box} className="d-desc-edit small" autoFocus onPaste={img.onPaste} onDrop={img.onDrop} onDragOver={img.onDragOver} value={draft} onChange={(e) => setDraft(e.currentTarget.value)} />
          <div className="d-row d-end">
            <button type="button" className="d-btn small" onClick={() => setEditing(false)}>Cancel</button>
            <button type="button" className="d-btn small primary" disabled={!draft.trim()} onClick={() => { s.dispatch({ type: 'update', kind: 'comment', id: c.id, patch: { body: draft.trim() } }); setEditing(false); }}>Save</button>
          </div>
        </div>
      ) : (
        <RichText className="d-comment-body" text={c.body} images={s.doc.image} />
      )}
    </div>
  );
}

function ActivityLine({ a }: { a: Activity }) {
  const { s } = useDelta();
  const doc = s.doc;
  const name = (kind: 'state' | 'project' | 'cycle' | 'team' | 'issue', id: unknown) => {
    if (typeof id !== 'string') return 'none';
    const r = (doc as any)[kind][id];
    if (!r) return 'something deleted';
    return kind === 'cycle' ? cycleName(r) : kind === 'issue' ? issueKey(doc, r) : r.name;
  };
  let what: ReactNode;
  switch (a.field) {
    case 'created': what = 'created the issue'; break;
    case 'title': what = <>renamed it to <b>{String(a.to)}</b></>; break;
    case 'description': what = 'updated the description'; break;
    case 'stateId': what = <>moved it from <b>{name('state', a.from)}</b> to <b>{name('state', a.to)}</b></>; break;
    case 'priority': what = <>set priority to <b>{PRIORITIES[Number(a.to)]}</b></>; break;
    case 'assignee': what = a.to ? <>assigned it to <b>{personName(a.to as string)}</b></> : 'removed the assignee'; break;
    case 'labelIds': {
      const from = (a.from as string[]) ?? [];
      const to = (a.to as string[]) ?? [];
      const added = to.filter((l) => !from.includes(l)).map((l) => doc.label[l]?.name ?? 'a label');
      const removed = from.filter((l) => !to.includes(l)).map((l) => doc.label[l]?.name ?? 'a label');
      what = <>{added.length > 0 && <>added <b>{added.join(', ')}</b></>}{added.length > 0 && removed.length > 0 && ' and '}{removed.length > 0 && <>removed <b>{removed.join(', ')}</b></>}</>;
      break;
    }
    case 'projectId': what = a.to ? <>added it to <b>{name('project', a.to)}</b></> : 'removed it from its project'; break;
    case 'cycleId': what = a.to ? <>added it to <b>{name('cycle', a.to)}</b></> : 'removed it from its cycle'; break;
    case 'parentId': what = a.to ? <>made it a sub-issue of <b>{name('issue', a.to)}</b></> : 'removed its parent'; break;
    case 'estimate': what = a.to != null ? <>set the estimate to <b>{String(a.to)}</b></> : 'removed the estimate'; break;
    case 'dueDate': what = a.to ? <>set the due date to <b>{shortDay(a.to as string)}</b></> : 'removed the due date'; break;
    case 'team': what = <>moved it to <b>{name('team', a.to)}</b></>; break;
    default: what = `changed ${a.field}`;
  }
  return (
    <div className="d-activity">
      <Avatar name={a.actor} size={14} />
      <span><b>{personName(a.actor)}</b> {what}</span>
      <span className="d-dim">· {ago(a.at)}</span>
    </div>
  );
}

/** Picks a project folder on this PC or one of the account's computers, and starts Scout there with the issue. */
function HandToScout({ i, onClose }: { i: Issue; onClose: () => void }) {
  const { s, host } = useDelta();
  const doc = s.doc;
  const [targets, setTargets] = useState<AgentTarget[] | null>(null);
  const [pick, setPick] = useState(0);
  const [prompt, setPrompt] = useState(() => agentPrompt(doc, i));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    host.agent!.targets().then((t) => {
      setTargets(t);
      const first = t.findIndex((x) => x.online);
      setPick(first < 0 ? 0 : first);
    }).catch((e) => { setTargets([]); setErr(message(e)); });
  }, []);
  const target = targets?.[pick];
  const start = async () => {
    if (!target) return;
    setBusy(true);
    setErr(null);
    try {
      const r = await host.agent!.start(target, prompt);
      const started = teamStates(doc, i.teamId).find((x) => x.type === 'started');
      const startedType = doc.state[i.stateId]?.type;
      s.dispatch([
        {
          type: 'update',
          kind: 'issue',
          id: i.id,
          patch: {
            agent: { where: host.kind === 'desktop' ? 'desktop' : 'scoutmote', pc: target.pc ? target.pcName : null, cwd: target.cwd, chatId: r.chatId, startedAt: Date.now(), by: s.me },
            ...(started && (startedType === 'backlog' || startedType === 'unstarted') ? { stateId: started.id } : {}),
          },
        },
        { type: 'create', kind: 'comment', id: newId(), data: { issueId: i.id, body: `Handed to Scout in ${target.project}${target.pc ? ` on ${target.pcName}` : ''}.` } },
      ]);
      onClose();
      host.showChat?.();
    } catch (e) {
      setErr(message(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal onClose={onClose} className="d-hand">
      <div className="d-row">
        <Avatar name={SCOUT} size={22} />
        <b className="d-big">Hand {issueKey(doc, i)} to Scout</b>
        <span className="d-grow" />
        <button type="button" className="d-icon-btn" onClick={onClose} aria-label="Close"><Icon name="close" /></button>
      </div>
      <p className="d-muted d-small">Scout starts a new chat with this issue in the project you pick, and the issue moves to In Progress.</p>
      <div className="d-field">
        <span>Project</span>
        {targets === null ? <span className="d-spinner small" /> : targets.length === 0 ? (
          <div className="d-error">{host.kind === 'desktop' ? 'Add a project folder in Scout first.' : 'Connect a computer with Scout to hand issues to it. Turn on ScoutMote in Scout\'s settings.'}</div>
        ) : (
          <div className="d-targets">
            {targets.map((t, n) => (
              <button type="button" key={`${t.pc}:${t.cwd}`} className={`d-target ${n === pick ? 'on' : ''}`} disabled={!t.online} onClick={() => setPick(n)}>
                <Icon name="laptop" size={14} />
                <span className="d-grow d-min0"><b className="d-ellipsis">{t.project}</b><span className="d-tiny d-dim d-ellipsis">{t.pc ? `${t.pcName}${t.online ? '' : ' · offline'} · ` : ''}{t.cwd}</span></span>
                {n === pick && <Icon name="check" size={14} />}
              </button>
            ))}
          </div>
        )}
      </div>
      <label className="d-field">
        <span>What Scout is asked</span>
        <textarea className="d-desc-edit" value={prompt} onChange={(e) => setPrompt(e.currentTarget.value)} />
      </label>
      {err && <div className="d-error">{err}</div>}
      <div className="d-row d-end">
        <button type="button" className="d-btn" onClick={onClose}>Cancel</button>
        <button type="button" className="d-btn primary" disabled={!target || !target.online || busy || !prompt.trim()} onClick={start}><Icon name="sparkle" size={13} />{busy ? 'Starting…' : 'Start Scout'}</button>
      </div>
    </Modal>
  );
}
