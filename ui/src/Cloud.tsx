// Epsilon in the window: cloud chats (repo tasks on a cloud computer), the "Working on …" picker under the prompt box
// that switches between this PC's projects and cloud repositories, the New project question, and the account's two
// Epsilon computers in the sidebar and on their own page. The data lives in epsilon.ts.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { act, openExternal } from './api';
import {
  cloud, createTask, ep, KIND_NAME, leaveCloud, LIVE, loadMachines, loadModels, loadRepos, newCloudChat, openCloudTask, repoShort,
  replyTask, setCloudDone, setMachine, setModel, STATUS, stopMachine, stopTask, useCloud, useSite, vmStateText, type CloudModel, type Machine, type Task, type TaskEvent, type VmKind,
} from './epsilon';
import { EpsilonScreen, type ScreenHost } from './epsilon/EpsilonScreen';
import { focusComposer, ui, useUi, useView } from './store';
import { Markdown, toolTitle } from './Transcript';
import { CopyIconButton, Icon, IconButton, MenuItem, Modal, Popover, SectionLabel, Spinner } from './widgets';
import { AsciiBackdrop } from './AsciiBackdrop';
import { ScoutLogo } from './logo';

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();

/** The small mark beside anything that runs in the cloud. */
export function CloudBadge() {
  return <span className="cloud-badge" title="Runs in the cloud on Epsilon">Cloud</span>;
}

// ---------- the sidebar ----------

/** A cloud chat in the sidebar's list, laid out like a chat on this PC (same lines, same buttons in the same places). */
export function CloudSessionRow({ t, current }: { t: Task; current: boolean }) {
  const done = useCloud((s) => s.done.has(t.id));
  const live = LIVE.has(t.status);
  const state = live ? 'working' : t.status === 'failed' ? 'error' : null;
  const title = t.prompt.split('\n')[0] || 'Cloud chat';
  return (
    <div className={`session-row two-line ${current ? 'selected' : ''}`} title={`${title}\n${t.repo} (cloud)`}>
      <button type="button" className="session-open" onClick={() => openCloudTask(t.id)}>
        <span className="session-line">
          <span className="session-title ellipsis grow">{title}</span>
          {state && <span className={`session-dot ${state}`} title={STATUS[t.status]} aria-label={STATUS[t.status]}>{state === 'error' ? '!' : ''}</span>}
        </span>
        <span className="session-line sub">
          <span className="ellipsis grow">{repoShort(t.repo)}</span>
          <CloudBadge />
          <span className="tiny when">{ago(t.updatedAt)}</span>
        </span>
      </button>
      <span className="row-actions">
        {done
          ? <IconButton icon="back" tip="Move back to Recent" size={24} onClick={() => setCloudDone(t.id, false)} />
          : <IconButton icon="check" tip={live ? 'Epsilon is still working on this chat' : 'Mark as done'} size={24} disabled={live} onClick={() => setCloudDone(t.id, true)} />}
        <Popover width={180} trigger={(_, toggle) => <IconButton icon="more" tip="More" size={24} onClick={toggle} />}>
          {(close) => (
            <>
              {done
                ? <MenuItem onClick={() => { close(); setCloudDone(t.id, false); }}>Move back to Recent</MenuItem>
                : <MenuItem disabled={live} onClick={() => { close(); setCloudDone(t.id, true); }}>Mark as done</MenuItem>}
              {t.pr
                ? <MenuItem onClick={() => { close(); openExternal(t.pr!.url); }}>Open pull request</MenuItem>
                : t.pushed && <MenuItem onClick={() => { close(); openExternal(`https://github.com/${t.repo}/tree/${t.branch}`); }}>Open branch on GitHub</MenuItem>}
              {live && <MenuItem onClick={() => { close(); void stopTask(t.id); }}>Stop</MenuItem>}
            </>
          )}
        </Popover>
      </span>
    </div>
  );
}

/** The top bar's Done chip for the open cloud chat, the same as a local chat's. */
export function CloudDoneButton() {
  const at = useUi((u) => u.cloud);
  const task = useCloud((s) => s.task);
  const id = at?.taskId;
  const done = useCloud((s) => !!id && s.done.has(id));
  if (!id || task?.id !== id) return null;
  const live = LIVE.has(task.status);
  return done ? (
    <button type="button" className="chip top-done on" title="Done. Click to move it back to Recent" onClick={() => setCloudDone(id, false)}>
      <Icon name="check" size={14} />Done
    </button>
  ) : (
    <button type="button" className="chip top-done" disabled={live} title={live ? 'Epsilon is still working on this chat' : 'Mark this chat as done'} onClick={() => setCloudDone(id, true)}>
      <Icon name="check" size={14} />Done
    </button>
  );
}

/** Cloud chats whose first message matches the sidebar search. */
export function useCloudSearch(q: string): Task[] {
  const tasks = useCloud((s) => s.tasks);
  const needle = q.trim().toLowerCase();
  return useMemo(() => (needle ? (tasks ?? []).filter((t) => t.prompt.toLowerCase().includes(needle) || t.repo.toLowerCase().includes(needle)) : []), [tasks, needle]);
}

function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** The cloud chats for the sidebar's Recent or Done list, newest first. */
export function useCloudRows(done: boolean): Task[] {
  const tasks = useCloud((s) => s.tasks);
  const marks = useCloud((s) => s.done);
  return useMemo(() => (tasks ?? []).filter((t) => marks.has(t.id) === done).sort((a, b) => b.updatedAt - a.updatedAt), [tasks, marks, done]);
}

function VmDot({ m }: { m: Machine | null | undefined }) {
  const s = m?.state;
  const cls = s === 'running' ? 'up' : s === 'starting' || s === 'saving' || s === 'stopping' ? 'busy' : s === 'error' ? 'bad' : 'off';
  return <span className={`vm-dot ${cls}`} aria-hidden />;
}

/** The Epsilon tab's side panel: the account's two computers. Opening one shows it beside the panel. */
export function EpsilonSidebar() {
  const { hasKey } = useView('cfg');
  const info = useCloud((s) => s.machines);
  const error = useCloud((s) => s.machinesError);
  const vm = useUi((u) => u.vm);
  const credits = info?.credits;
  return (
    <aside className="sidebar epsilon-side" style={{ width: 248 }}>
      <div className="side-head" data-tauri-drag-region>
        <b className="brand" data-tauri-drag-region>Epsilon</b>
        <CloudBadge />
        <span className="grow" data-tauri-drag-region />
        <IconButton icon="sidebar" tip="Hide sidebar" onClick={() => ui.set({ sidebar: false })} />
      </div>
      <div className="row side-label">
        <SectionLabel>Your computers</SectionLabel>
        <span className="grow" />
        {hasKey && <IconButton icon="refresh" tip="Refresh" size={24} onClick={() => loadMachines()} />}
      </div>
      {!hasKey ? (
        <button type="button" className="vm-row" onClick={() => act('browserLogin')}>
          <Icon name="cloud" size={15} /><span className="grow ellipsis small">Sign in to use Epsilon</span>
        </button>
      ) : !info && error ? (
        <div className="tiny dim vm-error" title={error}>{error}</div>
      ) : (
        (['persistent', 'temporary'] as VmKind[]).map((k) => {
          const m = info?.machines[k];
          return (
            <button key={k} type="button" className={`vm-row ${vm === k ? 'selected' : ''}`} onClick={() => ui.set({ page: 'epsilon', vm: k })} title={k === 'persistent' ? 'Yours to keep: stopping it saves everything' : 'A fresh machine each time: wiped when it stops'}>
              <Icon name="desktop" size={15} />
              <span className="grow min0">
                <span className="ellipsis small vm-name">{KIND_NAME[k]}</span>
                <span className="tiny dim vm-name">{k === 'persistent' ? 'Saved when it stops' : 'Wiped when it stops'}</span>
              </span>
              {info ? <><VmDot m={m} /><span className="tiny dim nowrap">{vmStateText(m)}</span></> : <Spinner size={10} />}
            </button>
          );
        })
      )}
      <span className="grow" />
      {credits && (
        <div className="tiny dim vm-credits">
          {credits.unlimited ? 'Unlimited Epsilon credits' : `${Math.round(credits.left)} Epsilon credits left`}
        </div>
      )}
    </aside>
  );
}

// ---------- "Working on …" under the prompt box ----------

/** Where the next chat runs: one of this PC's project folders, or a GitHub repository on Epsilon (Cloud). */
export function WorkingOn({ disabled }: { disabled?: boolean }) {
  const projects = useView('projects');
  const chat = useView('chat');
  const { hasKey } = useView('cfg');
  const at = useUi((u) => u.cloud);
  const repos = useCloud((s) => s.repos);
  const repoId = useCloud((s) => s.repoId);
  const task = useCloud((s) => s.task);
  const inCloud = !!at;
  const cloudRepo = at?.taskId ? task?.repo ?? null : repos?.repos.find((r) => r.id === (at?.repoId ?? repoId))?.name ?? null;
  const name = inCloud ? (cloudRepo ? repoShort(cloudRepo) : 'a repository') : chat.project;
  return (
    <span className="working-on">
      <span className="tiny dim">Working on</span>
      <Popover width={280} side="top" trigger={(open, toggle) => (
        <button type="button" className={`chip working-chip ${open ? 'open' : ''}`} disabled={disabled} title={inCloud ? 'In the cloud on Epsilon · click to switch' : `${chat.cwd} · click to switch`} onClick={() => { if (!open && hasKey) loadRepos(); toggle(); }}>
          <Icon name={inCloud ? 'cloud' : 'folder'} size={13} />
          <span className="ellipsis">{name}</span>
          {inCloud && <CloudBadge />}
          <Icon name="chevronDown" size={12} />
        </button>
      )}>
        {(close) => (
          <div className="working-menu">
            <div className="tiny dim menu-label">On this PC</div>
            {projects.map((p) => (
              <MenuItem key={p.path} icon="folder" selected={!inCloud && norm(p.path) === norm(chat.cwd)} onClick={() => {
                close();
                leaveCloud();
                if (norm(p.path) !== norm(chat.cwd)) act('changeProject', { dir: p.path });
                focusComposer();
              }}>{p.name}</MenuItem>
            ))}
            <hr />
            <div className="tiny dim menu-label row">Cloud <CloudBadge /></div>
            {!hasKey ? (
              <MenuItem icon="user" onClick={() => { close(); act('browserLogin'); }}>Sign in to use Epsilon</MenuItem>
            ) : !repos ? (
              <div className="tiny dim pad-x"><Spinner size={10} /> Loading your repositories…</div>
            ) : repos.repos.length === 0 ? (
              <MenuItem icon="plus" onClick={() => { close(); ui.set({ newProject: 'cloud' }); }}>Link a GitHub repository…</MenuItem>
            ) : (
              <div className="working-repos">
                {repos.repos.map((r) => (
                  <MenuItem key={r.id} icon="cloud" selected={inCloud && cloudRepo === r.name} onClick={() => { close(); newCloudChat(r.id); }}>
                    <span className="ellipsis" title={r.name}>{repoShort(r.name)}</span> <span className="tiny dim">{r.account}</span>
                  </MenuItem>
                ))}
              </div>
            )}
            <hr />
            <MenuItem icon="folderPlus" onClick={() => { close(); ui.set({ newProject: 'ask' }); }}>New project…</MenuItem>
          </div>
        )}
      </Popover>
    </span>
  );
}

// ---------- New project ----------

/** "New project": on this PC (a folder) or in the cloud (a GitHub repository on Epsilon). */
export function NewProjectDialog() {
  const step = useUi((u) => u.newProject);
  const { hasKey } = useView('cfg');
  const repos = useCloud((s) => s.repos);
  const site = useSite();
  const [q, setQ] = useState('');
  useEffect(() => {
    setQ('');
    if (step === 'cloud' && hasKey) loadRepos(true);
  }, [step]);
  if (!step) return null;
  const close = () => ui.set({ newProject: false });
  if (step === 'ask') {
    return (
      <Modal onClose={close} className="new-project">
        <b className="strong big">New project</b>
        <p className="small dim">Where should Scout work on it?</p>
        <div className="new-project-choices">
          <button type="button" className="new-project-choice" onClick={() => ui.set({ newProject: false, pickingFolder: true })}>
            <Icon name="desktop" size={22} />
            <b className="strong">Local</b>
            <span className="small dim">A folder on this PC. Scout works on your files directly.</span>
          </button>
          <button type="button" className="new-project-choice" onClick={() => ui.set({ newProject: 'cloud' })}>
            <Icon name="cloud" size={22} />
            <b className="strong">Cloud <CloudBadge /></b>
            <span className="small dim">A GitHub repository. Scout works on an Epsilon cloud computer and opens a pull request.</span>
          </button>
        </div>
        <div className="row end"><button type="button" className="button ghost" onClick={close}>Cancel</button></div>
      </Modal>
    );
  }
  const list = (repos?.repos ?? []).filter((r) => r.name.toLowerCase().includes(q.trim().toLowerCase()));
  return (
    <Modal onClose={close} className="new-project">
      <div className="row">
        <IconButton icon="back" tip="Back" size={26} onClick={() => ui.set({ newProject: 'ask' })} />
        <b className="strong big grow">Cloud project</b>
        <CloudBadge />
      </div>
      {!hasKey ? (
        <>
          <p className="small dim">Cloud projects run on Epsilon, ScoutCode's cloud computers. Sign in to your ScoutCode account to use them.</p>
          <div className="row end"><button type="button" className="button primary" onClick={() => act('browserLogin')}>Sign in with your browser</button></div>
        </>
      ) : !repos ? (
        <div className="row small dim"><Spinner /> Loading your repositories…</div>
      ) : repos.repos.length === 0 ? (
        <>
          <p className="small dim">
            Epsilon works on GitHub repositories you add to the ScoutCode app on GitHub, and opens pull requests there.
            {!repos.configured && ' GitHub isn’t set up on this server yet.'}
          </p>
          {repos.errors.map((e) => <div key={e} className="small red">{e}</div>)}
          <div className="row end">
            <button type="button" className="button" onClick={() => loadRepos(true)}>Refresh</button>
            {repos.configured && <button type="button" className="button primary" onClick={() => openExternal(`${site}/cloud/code-review/connect?install=1&next=epsilon`)}>Install the GitHub app</button>}
          </div>
        </>
      ) : (
        <>
          <p className="small dim">Pick a repository. Your chat starts on a fresh cloud computer with it cloned.</p>
          <input className="input" autoFocus placeholder="Search repositories" value={q} onChange={(e) => setQ(e.target.value)} />
          <div className="new-project-repos">
            {list.map((r) => (
              <button key={r.id} type="button" className="repo-row" onClick={() => { close(); newCloudChat(r.id); }}>
                <Icon name={r.private ? 'lock' : 'cloud'} size={14} />
                <span className="ellipsis grow">{r.name}</span>
                <span className="tiny dim">{r.defaultBranch}</span>
              </button>
            ))}
            {list.length === 0 && <div className="small dim pad-x">No repositories match.</div>}
          </div>
          <div className="row">
            <button type="button" className="text-button small" onClick={() => openExternal(`${site}/cloud/code-review/connect?install=1&next=epsilon`)}>Add more repositories on GitHub</button>
            <span className="grow" />
            <button type="button" className="text-button small" onClick={() => loadRepos(true)}>Refresh</button>
          </div>
        </>
      )}
    </Modal>
  );
}

// ---------- a cloud chat ----------

function EventRow({ e }: { e: TaskEvent }) {
  switch (e.kind) {
    case 'user':
      return (
        <div className="user-row">
          <div className="user-actions">{e.text && <CopyIconButton text={e.text} size={26} />}</div>
          <div className="user-bubble"><div className="pre-wrap">{e.text}</div></div>
        </div>
      );
    case 'say':
      return (
        <div className="assistant">
          <Markdown text={e.text} />
          {e.text && (
            <div className="assistant-foot">
              <div />
              <div className="assistant-actions"><CopyIconButton text={e.text} size={24} /></div>
            </div>
          )}
        </div>
      );
    case 'error':
      return <div className="error-box"><Icon name="close" /><span className="pre-wrap">{e.text}</span></div>;
    case 'tool':
      return (
        <div className="tool">
          <div className="tool-row" title={e.detail || undefined}>
            <span className={`tool-status ${e.ok === false ? 'bad' : 'good'}`}><Icon name={e.ok === false ? 'close' : 'check'} size={12} /></span>
            <b>{toolTitle(e.name ?? 'tool')}</b>
            {e.text && <span className="mono muted ellipsis tool-label">{e.text.split('\n')[0].slice(0, 160)}</span>}
            {e.summary && <span className={`tool-summary ellipsis ${e.ok === false ? 'red' : 'dim'}`}>· {e.summary}</span>}
          </div>
        </div>
      );
    default: {
      const url = e.detail && /^https?:\/\//.test(e.detail) ? e.detail : null;
      return (
        <div className="notice">
          <span>{e.text}{url && <>: <a href={url} onClick={(ev) => { ev.preventDefault(); openExternal(url); }}>{url}</a></>}</span>
        </div>
      );
    }
  }
}

/** The prompt box of a cloud chat: Enter sends, Shift+Enter makes a new line. */
function CloudInput({ placeholder, offPlaceholder, disabled, running, onSend, onStop }: {
  placeholder: string; offPlaceholder: string; disabled: boolean; running: boolean; onSend: (text: string) => Promise<void>; onStop?: () => void;
}) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLTextAreaElement>(null);
  const focusEpoch = useUi((u) => u.focusEpoch);
  useEffect(() => { box.current?.focus(); }, [focusEpoch, disabled]);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`;
  }, [text]);
  const ready = !!text.trim() && !disabled && !busy;
  const send = async () => {
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      await onSend(text.trim());
      setText('');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      {error && <div className="error-box"><Icon name="close" /><span className="pre-wrap">{error}</span></div>}
      <div className={`input-box ${running ? 'working' : ''}`}>
        <textarea
          ref={box}
          className="prompt"
          rows={2}
          value={text}
          disabled={disabled}
          placeholder={disabled ? offPlaceholder : placeholder}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.altKey) { e.preventDefault(); void send(); }
          }}
        />
        <div className="row composer-controls">
          <span className="grow" />
          <CloudModelChip disabled={running} />
          {running && onStop
            ? <button type="button" className="round stop" aria-label="Stop" title="Stop" onClick={onStop}><span className="stop-square" /></button>
            : <button type="button" className={`round send ${ready ? 'ready' : ''}`} disabled={!ready} aria-label="Send" onClick={() => void send()}>{busy ? <Spinner size={12} /> : <Icon name="arrowUp" />}</button>}
        </div>
      </div>
    </>
  );
}

/** The model a cloud chat's next message runs on: Scout's, or an agent connected on the website. */
function CloudModelChip({ disabled }: { disabled?: boolean }) {
  const models = useCloud((s) => s.models);
  const picked = useCloud((s) => s.model);
  const task = useCloud((s) => s.task);
  const at = useUi((u) => u.cloud);
  const site = useSite();
  const value = picked ?? (at?.taskId ? task?.model : null) ?? models?.default ?? null;
  const row = models?.models.find((m) => m.key === value);
  const label = row?.label ?? (value ? value.slice(value.indexOf(':') + 1) : 'Model');
  const groups = useMemo(() => {
    const g = new Map<string, CloudModel[]>();
    for (const m of models?.models ?? []) g.set(m.provider, [...(g.get(m.provider) ?? []), m]);
    return [...g.entries()];
  }, [models]);
  return (
    <Popover side="top" align="end" width={300} trigger={(open, toggle) => (
      <button type="button" className={`chip ${open ? 'open' : ''}`} disabled={disabled || !models} title="Model for cloud chats" onClick={() => { if (!open) loadModels(); toggle(); }}>
        {models ? (label.length > 26 ? `${label.slice(0, 25)}…` : label) : 'Loading…'}<Icon name="chevronDown" size={12} />
      </button>
    )}>
      {(close) => (
        <div className="working-menu">
          {groups.map(([provider, list]) => (
            <div key={provider}>
              <div className="tiny dim menu-label">{provider === 'scoutcode' ? 'ScoutCode' : provider}</div>
              {list.map((m) => (
                <MenuItem key={m.key} selected={m.key === value} onClick={() => { close(); setModel(m.key); }}>
                  <span className="ellipsis" title={m.description || m.label}>{m.label}</span>
                </MenuItem>
              ))}
            </div>
          ))}
          {groups.length === 0 && <div className="small dim pad-x">No models yet.</div>}
          <hr />
          <MenuItem icon="globe" onClick={() => { close(); openExternal(`${site}/scoutmote`); }}>Connect Codex, Claude Code and more…</MenuItem>
        </div>
      )}
    </Popover>
  );
}

/** A cloud chat in place of the local one: a new one ("< Building in repo? >" and the box) or a task's log. */
export function CloudChat() {
  const at = useUi((u) => u.cloud)!;
  const task = useCloud((s) => s.task);
  const repos = useCloud((s) => s.repos);
  const repoId = at.repoId ?? cloud.get().repoId;
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const shown = at.taskId && task?.id === at.taskId ? task : null;
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  });
  useEffect(() => { stick.current = true; }, [at.taskId]);

  if (!at.taskId) {
    const repo = repos?.repos.find((r) => r.id === repoId) ?? null;
    return (
      <div className="chat fresh">
        <AsciiBackdrop />
        <div className="transcript">
          <div className="empty">
            <ScoutLogo size={48} />
            <div className="splash typed">
              <span className="splash-text static">
                <span className="accent">{'<'}</span>
                <span className="strong"> Building in </span>
                <button type="button" className="splash-name" onClick={() => ui.set({ newProject: 'cloud' })} title="Switch repository">{repo ? repoShort(repo.name) : 'a repository'}</button>
                <span className="strong"> in the cloud?</span>
                <span className="accent">{' >'}</span>
              </span>
            </div>
            <div className="tiny dim">Epsilon clones it on your temporary cloud computer, picks the RAM it needs and opens a pull request.</div>
          </div>
        </div>
        <div className="composer">
          <div className="composer-column">
            <CloudInput
              placeholder={repo ? `Ask Epsilon to build or fix something in ${repoShort(repo.name)}` : 'Ask Epsilon to build or fix something'}
              offPlaceholder={repos ? 'Pick a repository to start' : 'Loading your repositories…'}
              disabled={!repo}
              running={false}
              onSend={(text) => createTask(repo!.id, text)}
            />
            <div className="footer"><WorkingOn /></div>
          </div>
        </div>
      </div>
    );
  }

  if (!shown) {
    return <div className="chat"><div className="transcript"><div className="empty"><Spinner size={18} /></div></div></div>;
  }
  const live = LIVE.has(shown.status);
  const branchUrl = `https://github.com/${shown.repo}/tree/${shown.branch}`;
  return (
    <div className="chat">
      <div className="transcript-wrap">
        <div className="transcript" ref={scroller} onScroll={(e) => { const el = e.currentTarget; stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60; }}>
          <div className="column">
            {shown.events.map((e, i) => <EventRow key={i} e={e} />)}
            {live && <div className="row small dim cloud-working"><Spinner size={11} /> {STATUS[shown.status]}</div>}
          </div>
        </div>
      </div>
      <div className="composer">
        <div className="composer-column">
          {shown.pr ? (
            <div className="card-note small">
              <span><b className="strong">Pull request #{shown.pr.number}</b> <span className="dim">on {shown.repo}.</span>{' '}
                <a href={shown.pr.url} onClick={(e) => { e.preventDefault(); openExternal(shown.pr!.url); }}>Open it on GitHub</a></span>
            </div>
          ) : shown.pushed && !live ? (
            <div className="card-note small">
              <span><span className="dim">The work so far is on</span> <a href={branchUrl} onClick={(e) => { e.preventDefault(); openExternal(branchUrl); }}>{shown.branch}</a>.</span>
            </div>
          ) : null}
          <CloudInput
            placeholder={shown.pr ? 'Reply to change the pull request' : 'Reply to carry on'}
            offPlaceholder="Epsilon is working… you can reply when it’s done"
            disabled={live}
            running={live}
            onSend={(text) => replyTask(shown.id, text)}
            onStop={() => void stopTask(shown.id)}
          />
          <div className="footer">
            <WorkingOn disabled={live} />
            <span className={`tiny ellipsis ${shown.status === 'failed' ? 'red' : 'dim'}`}>· {shown.error && shown.status === 'failed' ? shown.error : STATUS[shown.status]}</span>
            <span className="grow" />
            <span className="tiny dim nowrap">
              {[shown.memoryMB ? `${shown.memoryMB / 1024} GB RAM` : '', shown.cost > 0 ? `$${shown.cost < 0.01 ? shown.cost.toFixed(4) : shown.cost.toFixed(2)}` : ''].filter(Boolean).join(' · ')}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

/** The top bar's title while a cloud chat is open. */
export function CloudTitle() {
  const at = useUi((u) => u.cloud);
  const task = useCloud((s) => s.task);
  const title = at?.taskId ? (task?.id === at.taskId ? task.prompt.split('\n')[0] : '') : 'New cloud chat';
  return <><b className="chat-title ellipsis" data-tauri-drag-region>{title}</b><CloudBadge /></>;
}

// ---------- an Epsilon computer, open ----------

const gbOf = (mb?: number | null) => (mb ? `${mb >= 1024 ? mb / 1024 : (mb / 1024).toFixed(1)} GB` : '');

/** One of the account's computers: a way to start it, or the Epsilon server's own page for it in a frame. */
export function VmPage() {
  const kind = useUi((u) => u.vm);
  const info = useCloud((s) => s.machines);
  const error = useCloud((s) => s.machinesError);
  const { hasKey } = useView('cfg');
  const m = info?.machines[kind];
  const showing = m && ['starting', 'running', 'saving', 'stopping'].includes(m.state);
  if (!hasKey) {
    return (
      <div className="vm-page"><div className="vm-card">
        <Icon name="cloud" size={32} />
        <b className="strong big">Epsilon computers</b>
        <p className="small dim">Linux computers in the cloud, yours to use from Scout. Sign in to your ScoutCode account to use them.</p>
        <button type="button" className="button primary" onClick={() => act('browserLogin')}>Sign in with your browser</button>
      </div></div>
    );
  }
  if (!info) {
    return <div className="vm-page"><div className="vm-card">{error ? <><b className="strong">Couldn’t reach Epsilon</b><p className="small dim">{error}</p><button type="button" className="button" onClick={() => loadMachines()}>Try again</button></> : <Spinner size={18} />}</div></div>;
  }
  return showing && info.server ? <VmScreen kind={kind} info={info} server={info.server} /> : <VmStart kind={kind} info={info} />;
}

function VmStart({ kind, info }: { kind: VmKind; info: NonNullable<ReturnType<typeof cloud.get>['machines']> }) {
  const m = info.machines[kind];
  const [type, setType] = useState(m?.type ?? info.defaultType ?? info.types[0]?.id ?? '');
  const [memoryMB, setMemory] = useState<number>(m?.memoryMB ?? info.defaultMemoryMB ?? info.memory?.[0]?.memoryMB ?? 2048);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setError(null); setType(m?.type ?? info.defaultType ?? info.types[0]?.id ?? ''); }, [kind]);
  const rate = info.memory?.find((o) => o.memoryMB === memoryMB)?.rate;
  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await ep('POST', '/api/epsilon/machines', { kind, action: 'start', type, memoryMB });
      await loadMachines();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const saved = kind === 'persistent' && m && m.state === 'stopped';
  const kept = kind === 'temporary' && m?.state === 'stopped' && !!m.keptUntil;
  const wipe = async () => {
    setBusy(true);
    setError(null);
    try {
      await ep('POST', '/api/epsilon/machines', { kind, action: 'delete' });
      await loadMachines();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="vm-page">
      <div className="vm-card">
        <Icon name="desktop" size={30} />
        <b className="strong big">{KIND_NAME[kind]}</b>
        <p className="small dim">
          {kept
            ? `Kept from a cloud chat until ${new Date(m!.keptUntil!).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}, with its files as the chat left them. Start it to look around; it’s wiped after that.`
            : kind === 'persistent' ? 'Yours to keep. Stopping it saves your files and open apps, so it picks up where you left off.' : 'A fresh machine for one session. Everything on it is wiped when it stops.'}
        </p>
        {m?.state === 'error' && m.error && <div className="small red">{m.error}</div>}
        {!saved && !kept && info.types.length > 0 && (
          <label className="vm-field"><span className="small dim">Setup</span>
            <select className="input" value={type} onChange={(e) => setType(e.target.value)}>
              {info.types.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
            </select>
          </label>
        )}
        {(info.memory?.length ?? 0) > 0 && (
          <label className="vm-field"><span className="small dim">Memory</span>
            <select className="input" value={memoryMB} onChange={(e) => setMemory(Number(e.target.value))}>
              {info.memory!.map((o) => <option key={o.memoryMB} value={o.memoryMB}>{gbOf(o.memoryMB)} · {o.rate} credit{o.rate === 1 ? '' : 's'}/min</option>)}
            </select>
          </label>
        )}
        <div className="tiny dim">
          {info.credits.unlimited ? 'Unlimited Epsilon credits.' : `${Math.round(info.credits.left)} Epsilon credits left${rate ? `, about ${Math.max(1, Math.floor(info.credits.left / rate))} minutes` : ''}.`}
        </div>
        {error && <div className="small red">{error}</div>}
        <button type="button" className="button primary" disabled={busy} onClick={start}>{busy ? <Spinner size={12} /> : saved ? 'Resume it' : 'Start it'}</button>
        {kept && <button type="button" className="button ghost" disabled={busy} onClick={wipe}>Wipe it now</button>}
      </div>
    </div>
  );
}

/** One computer, drawn here (screen and terminal) and connected straight to the Epsilon server. */
function VmScreen({ kind, info, server }: { kind: VmKind; info: NonNullable<ReturnType<typeof cloud.get>['machines']>; server: string }) {
  const m = info.machines[kind];
  const type = info.types.find((t) => t.id === m.type);
  const host = useMemo<ScreenHost>(() => ({
    server,
    token: () => ep<{ token: string }>('POST', '/api/epsilon/token', {}).then((r) => r.token, () => null),
    // The terminal socket says when it stops on its own (idle, time limit, out of credits): show it at once.
    onStatus: (next) => {
      const cur = cloud.get().machines?.machines[kind];
      if (cur && cur.state !== next.state && !(cur.state !== 'running' && next.state === 'running')) setMachine(kind, next as Partial<Machine>);
      if (next.state !== 'running') loadMachines();
    },
  }), [server, kind]);
  return (
    <div className="vm-page framed">
      <EpsilonScreen kind={kind} machine={m} host={host} typeName={type?.name} fast={!!m.memoryMB && !!type?.fastStart?.includes(m.memoryMB)} />
    </div>
  );
}

/** The top bar's controls while a computer is open. */
export function VmBar() {
  const kind = useUi((u) => u.vm);
  const m = useCloud((s) => s.machines?.machines[kind]);
  const [busy, setBusy] = useState(false);
  // A temporary computer is wiped, so the first click asks on the button and a second click within 4 seconds stops it.
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setAsking(false); setError(null); }, [kind]);
  useEffect(() => {
    if (!asking) return;
    const t = setTimeout(() => setAsking(false), 4000);
    return () => clearTimeout(t);
  }, [asking]);
  useEffect(() => {
    if (!error) return;
    const t = setTimeout(() => setError(null), 10000);
    return () => clearTimeout(t);
  }, [error]);
  const stop = async () => {
    if (kind === 'temporary' && !asking) return setAsking(true);
    setAsking(false);
    setBusy(true);
    setError(null);
    try {
      await stopMachine(kind);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setBusy(false);
  };
  return (
    <>
      <Icon name="desktop" size={15} />
      <b className="chat-title ellipsis" data-tauri-drag-region>{KIND_NAME[kind]}</b>
      <CloudBadge />
      <span className="tiny dim nowrap" data-tauri-drag-region>{[vmStateText(m), gbOf(m?.memoryMB)].filter(Boolean).join(' · ')}</span>
      <span className="grow" data-tauri-drag-region />
      {error && <span className="tiny red ellipsis vm-bar-error" title={error}>Couldn’t stop it: {error}</span>}
      {(m?.state === 'running' || busy) && (
        <button type="button" className={`chip ${asking ? 'vm-asking' : ''}`} disabled={busy} onClick={stop} title={kind === 'persistent' ? 'Save it and stop it' : 'Stop it and wipe it'}>
          <Icon name="stop" size={13} />
          {busy ? (kind === 'persistent' ? 'Saving…' : 'Wiping…') : asking ? 'Wipe it? Click again' : kind === 'persistent' ? 'Save & stop' : 'Stop & wipe'}
        </button>
      )}
      <IconButton icon="chat" tip="Back to the chats" onClick={() => ui.set({ page: 'chat' })} />
    </>
  );
}
