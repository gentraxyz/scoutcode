// What a right-click offers on an issue, a project or a cycle. The same in the desktop app and on the website.
import { cycleName, cycleStatus, issueKey, PRIORITIES, PROJECT_STATUSES, teamStates, type Cycle, type Issue, type Project } from './model';
import { statusLabel, type DeltaCtx } from './Delta';
import { Avatar, Icon, PriorityIcon, StateIcon, type CtxItem } from './ui';
import { ProjectStatusIcon } from './views';
import { SCOUT } from './model';

const copy = (text: string) => navigator.clipboard?.writeText(text);

export function issueMenu(c: DeltaCtx, i: Issue): CtxItem[] {
  const { s } = c;
  const doc = s.doc;
  const key = issueKey(doc, i);
  const set = (patch: Partial<Issue>) => s.dispatch({ type: 'update', kind: 'issue', id: i.id, patch });
  const projects = Object.values(doc.project).filter((p) => p.status !== 'canceled' || p.id === i.projectId).sort((a, b) => a.name.localeCompare(b.name));
  const cycles = Object.values(doc.cycle).filter((x) => x.teamId === i.teamId && (cycleStatus(x) !== 'past' || x.id === i.cycleId)).sort((a, b) => a.startsAt.localeCompare(b.startsAt));
  const people: (string | null)[] = [null, ...s.people, SCOUT];
  return [
    { label: 'Open', icon: <Icon name="external" size={14} />, onClick: () => c.open(i.id) },
    ...(c.openTab ? [{ label: 'Open in new tab', icon: <Icon name="plus" size={14} />, onClick: () => c.openTab?.({ view: 'team', teamId: i.teamId, tab: 'all' }, i.id) }] : []),
    'sep',
    { label: 'Status', icon: <StateIcon state={doc.state[i.stateId]} />, sub: teamStates(doc, i.teamId).map((st) => ({ label: st.name, icon: <StateIcon state={st} />, selected: st.id === i.stateId, onClick: () => set({ stateId: st.id }) })) },
    { label: 'Priority', icon: <PriorityIcon p={i.priority} />, sub: PRIORITIES.map((name, p) => ({ label: name, icon: <PriorityIcon p={p} />, selected: p === i.priority, onClick: () => set({ priority: p }) })) },
    { label: 'Assignee', icon: <Avatar name={i.assignee} />, sub: people.map((u) => ({ label: u === null ? 'Unassigned' : u === SCOUT ? 'Scout' : u, icon: <Avatar name={u} />, selected: u === i.assignee, onClick: () => set({ assignee: u }) })) },
    ...(s.me && i.assignee !== s.me ? [{ label: 'Assign to me', icon: <Icon name="user" size={14} />, onClick: () => set({ assignee: s.me }) }] : []),
    { label: 'Project', icon: <Icon name="project" size={14} />, sub: [
      { label: 'No project', selected: !i.projectId, onClick: () => set({ projectId: null }) },
      ...projects.map((p) => ({ label: p.name, icon: <span className="d-project-color" style={{ background: p.color }} />, selected: p.id === i.projectId, onClick: () => set({ projectId: p.id }) })),
    ] },
    ...(cycles.length || i.cycleId ? [{ label: 'Cycle', icon: <Icon name="cycle" size={14} />, sub: [
      { label: 'No cycle', selected: !i.cycleId, onClick: () => set({ cycleId: null }) },
      ...cycles.map((x) => ({ label: cycleName(x), selected: x.id === i.cycleId, onClick: () => set({ cycleId: x.id }) })),
    ] as CtxItem[] }] : []),
    'sep',
    { label: 'Add sub-issue', icon: <Icon name="parent" size={14} />, onClick: () => c.create({ teamId: i.teamId, parentId: i.id, projectId: i.projectId, cycleId: i.cycleId }) },
    { label: 'Copy ID', icon: <Icon name="copy" size={14} />, onClick: () => copy(key) },
    { label: 'Copy ID and title', icon: <Icon name="copy" size={14} />, onClick: () => copy(`${key} ${i.title}`) },
    'sep',
    { label: 'Delete issue', danger: true, icon: <Icon name="trash" size={14} />, onClick: () => { if (confirm(`Delete ${key}? Its comments and history go too.`)) s.dispatch({ type: 'delete', kind: 'issue', id: i.id }); } },
  ];
}

export function projectMenu(c: DeltaCtx, p: Project): CtxItem[] {
  const { s } = c;
  return [
    { label: 'Open', icon: <Icon name="external" size={14} />, onClick: () => c.go({ view: 'project', id: p.id }) },
    ...(c.openTab ? [{ label: 'Open in new tab', icon: <Icon name="plus" size={14} />, onClick: () => c.openTab?.({ view: 'project', id: p.id }) }] : []),
    { label: 'New issue in project', icon: <Icon name="plus" size={14} />, onClick: () => c.create({ projectId: p.id, ...(p.teamIds[0] ? { teamId: p.teamIds[0] } : {}) }) },
    'sep',
    { label: 'Status', icon: <ProjectStatusIcon status={p.status} />, sub: PROJECT_STATUSES.map((st) => ({ label: statusLabel(st), icon: <ProjectStatusIcon status={st} />, selected: st === p.status, onClick: () => s.dispatch({ type: 'update', kind: 'project', id: p.id, patch: { status: st } }) })) },
    { label: 'Copy name', icon: <Icon name="copy" size={14} />, onClick: () => copy(p.name) },
    'sep',
    { label: 'Delete project', danger: true, icon: <Icon name="trash" size={14} />, onClick: () => { if (confirm(`Delete the project "${p.name}"? Its issues stay, without a project.`)) s.dispatch({ type: 'delete', kind: 'project', id: p.id }); } },
  ];
}

export function cycleMenu(c: DeltaCtx, x: Cycle): CtxItem[] {
  const { s } = c;
  const name = cycleName(x);
  return [
    { label: 'Open', icon: <Icon name="external" size={14} />, onClick: () => c.go({ view: 'cycle', id: x.id }) },
    ...(c.openTab ? [{ label: 'Open in new tab', icon: <Icon name="plus" size={14} />, onClick: () => c.openTab?.({ view: 'cycle', id: x.id }) }] : []),
    { label: 'New issue in cycle', icon: <Icon name="plus" size={14} />, onClick: () => c.create({ teamId: x.teamId, cycleId: x.id }) },
    'sep',
    { label: 'Delete cycle', danger: true, icon: <Icon name="trash" size={14} />, onClick: () => { if (confirm(`Delete ${name}? Its issues stay, without a cycle.`)) s.dispatch({ type: 'delete', kind: 'cycle', id: x.id }); } },
  ];
}
