// Inbox: everything in progress in one place. Chats that need an answer, are working, or haven't been marked done, and the
// Issues (Delta) assigned to you or being worked by a Scout agent, across every workspace.
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { act, type View } from './api';
import { ui, useView } from './store';
import { Icon, IconButton, SectionLabel, Spinner } from './widgets';
import { SessionRow, useProjectName } from './Sidebar';
import { useDeltaHost } from './DeltaPage';
import { rememberWorkspace } from './delta/Delta';
import { deltaTabs } from './delta/tabs';
import { listWorkspaces, message, Session, type WorkspaceInfo } from './delta/sync';
import { isDone, issueKey, SCOUT, type Issue, type State } from './delta/model';
import { PriorityIcon, StateIcon } from './delta/ui';
import { DreamEntry, DreamingNow, useUnseenDreams } from './Dreams';

type SessionInfo = View['sessions'][number];

/** The chats the Inbox lists, by how much they want you. Done chats aren't in any of them. */
export function inboxChats(sessions: SessionInfo[]) {
  const open = sessions.filter((s) => !s.done);
  return {
    waiting: open.filter((s) => s.waiting),
    working: open.filter((s) => s.running && !s.waiting),
    unread: open.filter((s) => !s.running && !s.waiting && s.unread),
    rest: open.filter((s) => !s.running && !s.waiting && !s.unread),
  };
}

interface Found {
  ws: WorkspaceInfo;
  wsKey: string;
  issue: Issue;
  key: string;
  state: State | undefined;
  mine: boolean;
}

// In-progress work first, then what's next, then the backlog.
const STATE_ORDER: Record<string, number> = { started: 0, review: 1, unstarted: 2, backlog: 3 };
const PRIORITY_ORDER = (p: number) => (p === 0 ? 5 : p);
const byUrgency = (a: Found, b: Found) =>
  (STATE_ORDER[a.state?.type ?? 'unstarted'] ?? 9) - (STATE_ORDER[b.state?.type ?? 'unstarted'] ?? 9) ||
  PRIORITY_ORDER(a.issue.priority) - PRIORITY_ORDER(b.issue.priority) ||
  b.issue.updatedAt - a.issue.updatedAt;

/** Reads every workspace once and keeps the open issues that are yours or a Scout agent's. */
function useInboxIssues() {
  const host = useDeltaHost();
  const [found, setFound] = useState<Found[] | null>(null);
  const [errors, setErrors] = useState<string[]>([]);
  const [epoch, setEpoch] = useState(0);
  useEffect(() => {
    let live = true;
    (async () => {
      const { list } = await listWorkspaces(host);
      const errs: string[] = [];
      const per = await Promise.all(list.map(async (ws) => {
        try {
          const s = await Session.open(host, ws);
          s.close();
          const wsKey = `${ws.storage}:${ws.id}`;
          const out: Found[] = [];
          for (const i of Object.values(s.doc.issue)) {
            const state = s.doc.state[i.stateId];
            if (isDone(state)) continue;
            const mine = i.assignee === s.me;
            if (!mine && !i.agent && i.assignee !== SCOUT) continue;
            out.push({ ws, wsKey, issue: i, key: issueKey(s.doc, i), state, mine });
          }
          return out;
        } catch (e) {
          errs.push(`${ws.name}: ${message(e)}`);
          return [];
        }
      }));
      if (!live) return;
      setFound(per.flat().sort(byUrgency));
      setErrors(errs);
    })();
    return () => { live = false; };
  }, [host, epoch]);
  return { found, errors, refresh: useCallback(() => setEpoch((n) => n + 1), []) };
}

function openIssue(f: Found) {
  rememberWorkspace(f.wsKey);
  deltaTabs.openOnLoad(f.wsKey, f.issue.id);
  ui.set({ page: 'delta' });
}

function IssueItem({ f, showWs }: { f: Found; showWs: boolean }) {
  const run = f.issue.agent;
  return (
    <div className="inbox-issue">
      <button type="button" className="inbox-issue-open" onClick={() => openIssue(f)} title={`${f.key} ${f.issue.title}`}>
        <PriorityIcon p={f.issue.priority} />
        <span className="inbox-key tiny dim">{f.key}</span>
        <StateIcon state={f.state} />
        <span className="ellipsis grow">{f.issue.title}</span>
        {showWs && <span className="tiny dim ellipsis inbox-ws">{f.ws.name}</span>}
        <span className="tiny dim inbox-state">{f.state?.name ?? ''}</span>
      </button>
      {run?.chatId && run.where === 'desktop' && (
        <IconButton icon="chat" tip="Open the chat working on this" size={26} onClick={() => { act('openChat', { id: run.chatId }); ui.set({ page: 'chat' }); }} />
      )}
    </div>
  );
}

function Section({ title, count, children, action }: { title: string; count?: number; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="inbox-section">
      <div className="row inbox-head">
        <SectionLabel>{title}</SectionLabel>
        {count !== undefined && <span className="tiny dim">{count}</span>}
        <span className="grow" />
        {action}
      </div>
      {children}
    </section>
  );
}

const REST_SHOWN = 8;

export function Inbox() {
  const sessions = useView('sessions');
  const chat = useView('chat');
  const projectName = useProjectName();
  const { waiting, working, unread, rest } = inboxChats(sessions);
  const { found, errors, refresh } = useInboxIssues();
  const [allRest, setAllRest] = useState(false);
  const mine = found?.filter((f) => f.mine) ?? [];
  const agents = found?.filter((f) => !f.mine) ?? [];
  const showWs = new Set(found?.map((f) => f.wsKey)).size > 1;
  const row = (s: SessionInfo) => <SessionRow key={s.id} s={s} current={s.id === chat.id} project={projectName(s.cwd)} />;
  const dreams = useUnseenDreams();
  const nothing = !waiting.length && !working.length && !unread.length && !rest.length && !dreams.length && found !== null && !found.length;

  return (
    <div className="inbox">
      <div className="inbox-body">
        {waiting.length > 0 && <Section title="Needs your answer" count={waiting.length}><div className="inbox-list">{waiting.map(row)}</div></Section>}
        {working.length > 0 && <Section title="Working" count={working.length}><div className="inbox-list">{working.map(row)}</div></Section>}
        {unread.length > 0 && <Section title="Finished, not read" count={unread.length}><div className="inbox-list">{unread.map(row)}</div></Section>}
        <DreamingNow />
        {dreams.length > 0 && (
          <Section
            title="While you were away"
            count={dreams.length}
            action={<button type="button" className="button compact" onClick={() => act('dreamSeen', { id: 'all' })}>Dismiss all</button>}
          >
            <div className="stack">{dreams.map((d) => <DreamEntry key={d.id} d={d} inbox />)}</div>
          </Section>
        )}

        <Section
          title="Assigned to you"
          count={found ? mine.length : undefined}
          action={found ? <IconButton icon="refresh" tip="Check Issues again" size={24} onClick={refresh} /> : <Spinner size={11} />}
        >
          <div className="inbox-list">
            {mine.map((f) => <IssueItem key={`${f.wsKey}:${f.issue.id}`} f={f} showWs={showWs} />)}
            {found && !mine.length && <div className="small dim inbox-empty">No open issues assigned to you.</div>}
          </div>
        </Section>

        {agents.length > 0 && (
          <Section title="Scout agents on issues" count={agents.length}>
            <div className="inbox-list">{agents.map((f) => <IssueItem key={`${f.wsKey}:${f.issue.id}`} f={f} showWs={showWs} />)}</div>
          </Section>
        )}

        {rest.length > 0 && (
          <Section title="Not marked done" count={rest.length}>
            <div className="inbox-list">
              {(allRest ? rest : rest.slice(0, REST_SHOWN)).map(row)}
              {rest.length > REST_SHOWN && (
                <button type="button" className="done-footer" onClick={() => setAllRest(!allRest)}>
                  <span className="grow">{allRest ? 'Show fewer' : `Show all ${rest.length}`}</span>
                  <Icon name={allRest ? 'chevronUp' : 'chevronDown'} size={13} />
                </button>
              )}
            </div>
          </Section>
        )}

        {errors.length > 0 && <div className="tiny dim inbox-errors">Couldn't read: {errors.join(' · ')}</div>}
        {nothing && (
          <div className="inbox-zero">
            <Icon name="inbox" size={28} />
            <div className="strong">You're all caught up</div>
            <div className="small dim">Chats in progress and issues assigned to you show up here.</div>
          </div>
        )}
      </div>
    </div>
  );
}
