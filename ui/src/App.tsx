import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { act, invoke, openExternal } from './api';
import { AsciiBackdrop } from './AsciiBackdrop';
import { Composer, PermissionBody } from './Composer';
import { Dock } from './Dock';
import { handleDockKeys, isTermTab, newDockTab, openDockTab, useAutoTaskTabs, useTasks } from './DockTabs';
import { FloatingGame } from './Games';
import { FloatingMusic } from './Music';
import { Onboarding } from './Onboarding';
import { Settings, UsagePage, restartBlocker } from './Settings';
import { addProject, Sidebar } from './Sidebar';
import { Hideable, HideMenu } from './Hideable';
import { core, focusComposer, ui, useCore, useUi, useView, type DockTab } from './store';
import { Markdown, Transcript } from './Transcript';
import { DeltaPage } from './DeltaPage';
import { Inbox } from './Inbox';
import { NavRail } from './NavRail';
import { DeltaTabsBar } from './delta/DeltaTabsBar';
import { deltaTabs } from './delta/tabs';
import { FolderPicker } from './FolderPicker';
import { UsageAlert } from './UsageAlert';
import { Icon, IconButton, Modal, Spinner } from './widgets';
import { useActivityPing } from './Dreams';
import { CloudChat, CloudDoneButton, CloudTitle, EpsilonSidebar, NewProjectDialog, VmBar, VmPage } from './Cloud';
import { cloud, LIVE, newCloudChat, stopTask, useEpsilonSync } from './epsilon';

const appWindow = getCurrentWindow();

function WindowButtons() {
  const [maximized, setMaximized] = useState(true);
  useEffect(() => {
    const check = () => appWindow.isMaximized().then(setMaximized);
    check();
    const off = appWindow.onResized(check);
    return () => { off.then((f) => f()); };
  }, []);
  return (
    <div className="window-buttons">
      <button type="button" aria-label="Minimize" onClick={() => appWindow.minimize()}><Icon name="minimize" size={14} /></button>
      <button type="button" aria-label={maximized ? 'Restore' : 'Maximize'} onClick={() => appWindow.toggleMaximize()}><Icon name={maximized ? 'restore' : 'maximize'} size={14} /></button>
      <button type="button" className="close" aria-label="Close" onClick={() => appWindow.close()}><Icon name="close" size={14} /></button>
    </div>
  );
}

function toggleDock(tab: DockTab) {
  const { dockOpen, dockTab } = ui.get();
  if (tab === 'terminal') {
    // Ctrl+` shows a terminal already open (the one in front closes the panel), or opens one.
    if (dockOpen && isTermTab(dockTab)) return ui.set({ dockOpen: false });
    const term = ui.get().dockTabs.find(isTermTab);
    return term ? ui.set({ dockOpen: true, dockTab: term }) : openDockTab('terminal');
  }
  if (dockOpen && dockTab === tab) ui.set({ dockOpen: false });
  else openDockTab(tab);
}

/** Opens and closes the panel. Opened with no tabs, it starts on a New Tab page. */
function togglePanel() {
  const { dockOpen, dockTabs } = ui.get();
  if (dockOpen) ui.set({ dockOpen: false });
  else if (!dockTabs.length) newDockTab();
  else ui.set({ dockOpen: true });
}

/** The panel's button: what used to be a button per page is now the New Tab page inside it. */
function PanelButton() {
  const dockOpen = useUi((u) => u.dockOpen);
  const running = useTasks().some((t) => !t.result);
  return (
    <span className="panel-button">
      <IconButton icon="panel" tip={dockOpen ? 'Close the panel' : 'Open the panel: terminal, browser, files and more'} active={dockOpen} onClick={togglePanel} />
      {running && !dockOpen && <span className="panel-button-dot" title="A sub-agent is working" />}
    </span>
  );
}

/** Versions whose update card the user put off with Later, for this run: the top bar's pill stays. */
const laterVersions = new Set<string>();

function restartNow() {
  ui.set({ restarting: true });
  // A restart that didn't happen (the core said why in the status line) leaves the window as it was.
  invoke('restart_to_update').then(() => setTimeout(() => ui.set({ restarting: false }), 20_000), () => ui.set({ restarting: false }));
}

function UpdatePill() {
  const { status } = useView('updater');
  const { anyRunning } = useView('run');
  const dirty = useUi((u) => u.editorDirty);
  if (status.state !== 'ready') return null;
  const blocker = restartBlocker(anyRunning, dirty);
  const summary = status.summary.trim() ? `\n${status.summary.trim()}` : '';
  return (
    <button type="button" className="button primary pill" disabled={!!blocker} title={blocker ?? `Scout ${status.version} is ready. Your chat reopens after the restart.${summary}`} onClick={restartNow}>Restart to update</button>
  );
}

/** A card in the corner while an update downloads and once it's ready, with what's in it and a restart button. */
function UpdateCard() {
  const { status } = useView('updater');
  const { anyRunning } = useView('run');
  const dirty = useUi((u) => u.editorDirty);
  const restarting = useUi((u) => u.restarting);
  const [, redraw] = useState(0);
  const version = 'version' in status ? status.version : '';
  if (restarting) {
    return (
      <div className="update-restarting" role="status">
        <div className="update-restarting-card">
          <Spinner size={18} />
          <b className="strong">Installing Scout {version}</b>
          <span className="small dim">Scout reopens on its own in a few seconds, right where you left off.</span>
        </div>
      </div>
    );
  }
  if ((status.state !== 'ready' && status.state !== 'downloading') || laterVersions.has(version)) return null;
  const later = () => { laterVersions.add(version); redraw((n) => n + 1); };
  const blocker = restartBlocker(anyRunning, dirty);
  const pct = status.state === 'downloading' && status.total > 0 ? Math.min(100, Math.round((status.got / status.total) * 100)) : null;
  const summary = status.state === 'ready' ? status.summary.trim() : '';
  return (
    <div className="update-card" role="status">
      <div className="update-card-head">
        <span className="update-card-icon"><Icon name="download" size={15} /></span>
        <div className="stack grow" style={{ minWidth: 0, gap: 2 }}>
          <b className="strong">{status.state === 'ready' ? `Scout ${version} is ready` : `Downloading Scout ${version}`}</b>
          <span className="tiny dim">{status.state === 'ready' ? 'Restart to install it. Your chats reopen as they were.' : pct != null ? `${pct}%` : 'Starting…'}</span>
        </div>
        <IconButton icon="close" tip="Later" size={24} onClick={later} />
      </div>
      {status.state === 'downloading' && <div className="update-card-bar"><span style={{ width: `${pct ?? 4}%` }} /></div>}
      {summary && <p className="small muted update-card-summary">{summary}</p>}
      {status.state === 'ready' && (
        <div className="row end" style={{ gap: 8 }}>
          <button type="button" className="button ghost compact" onClick={later}>Later</button>
          <button type="button" className="button primary compact" disabled={!!blocker} title={blocker ?? undefined} onClick={restartNow}>Restart now</button>
        </div>
      )}
      {status.state === 'ready' && blocker && <span className="tiny dim">{blocker}</span>}
    </div>
  );
}

/** Shown once after Scout updates itself: what changed since the version that ran before. */
function WhatsNew() {
  const releases = useView('whatsNew');
  if (!releases.length) return null;
  const close = () => act('dismissWhatsNew');
  return (
    <Modal onClose={close} className="whats-new">
      <div className="whats-new-head">
        <span className="whats-new-badge"><Icon name="check" size={16} /></span>
        <div className="stack" style={{ gap: 2 }}>
          <b className="strong big">You're on Scout {releases[0].version}</b>
          <span className="small dim">{releases.length > 1 ? `${releases.length} updates since you last opened Scout` : "Here's what's new"}</span>
        </div>
      </div>
      <div className="whats-new-list stack-wide">
        {releases.map((r) => (
          <div key={r.version} className="whats-new-release stack">
            {releases.length > 1 && <span className="whats-new-version tiny">{r.version}</span>}
            {r.summary.trim() && <div className="strong">{r.summary}</div>}
            {r.changelog.trim() && <div className="whats-new-body small muted"><Markdown text={r.changelog} /></div>}
          </div>
        ))}
      </div>
      <div className="row end"><button type="button" className="button primary" onClick={close}>Got it</button></div>
    </Modal>
  );
}

/** Marks the open chat Done, which files it under Done in the sidebar until it gets new activity. */
function DoneButton() {
  const chat = useView('chat');
  const s = useView('sessions').find((x) => x.id === chat.id);
  // A new chat isn't saved until its first message, so there's nothing to mark yet.
  if (!s) return null;
  const busy = s.running || s.waiting;
  return s.done ? (
    <button type="button" className="chip top-done on" title="Done. Click to move it back to Recent" onClick={() => act('setDone', { id: s.id, done: false })}>
      <Icon name="check" size={14} />Done
    </button>
  ) : (
    <button type="button" className="chip top-done" disabled={busy} title={busy ? 'Scout is still working on this chat' : 'Mark this chat as done'} onClick={() => act('setDone', { id: s.id, done: true })}>
      <Icon name="check" size={14} />Done
    </button>
  );
}

function TopBar() {
  const sidebar = useUi((u) => u.sidebar);
  const chat = useView('chat');
  const page = useUi((u) => u.page);
  const inCloud = useUi((u) => !!u.cloud);
  if (page === 'epsilon') {
    return (
      <header className="top-bar" data-tauri-drag-region>
        {!sidebar && <IconButton icon="sidebar" tip="Show sidebar" onClick={() => ui.set({ sidebar: true })} />}
        <VmBar />
        <UpdatePill />
        <WindowButtons />
      </header>
    );
  }
  if (page === 'chat' && inCloud) {
    return (
      <header className="top-bar" data-tauri-drag-region>
        {!sidebar && <IconButton icon="sidebar" tip="Show sidebar" onClick={() => ui.set({ sidebar: true })} />}
        <CloudTitle />
        <span className="grow" data-tauri-drag-region />
        <CloudDoneButton />
        <UpdatePill />
        <WindowButtons />
      </header>
    );
  }
  // Issues and Inbox have no chats sidebar (the rail gets you back), so no button to show it either.
  if (page === 'inbox' || page === 'usage') {
    return (
      <header className="top-bar" data-tauri-drag-region>
        <Icon name={page === 'inbox' ? 'inbox' : 'chart'} size={15} />
        <b className="chat-title inbox-title" data-tauri-drag-region>{page === 'inbox' ? 'Inbox' : 'Usage'}</b>
        <span className="grow" data-tauri-drag-region />
        <UpdatePill />
        <WindowButtons />
      </header>
    );
  }
  if (page === 'delta') {
    return (
      <header className="top-bar delta-top-bar" data-tauri-drag-region>
        <DeltaTabsBar />
        <span className="grow" data-tauri-drag-region />
        <UpdatePill />
        <IconButton icon="chat" tip="Back to the chat" onClick={() => ui.set({ page: 'chat' })} />
        <WindowButtons />
      </header>
    );
  }
  return (
    <header className="top-bar" data-tauri-drag-region>
      {!sidebar && <IconButton icon="sidebar" tip="Show sidebar" onClick={() => ui.set({ sidebar: true })} />}
      <b className="chat-title ellipsis" data-tauri-drag-region>{chat.title}</b>
      <span className="grow" data-tauri-drag-region />
      {/* The sidebar shows the folder; without it, the bar does. */}
      <DoneButton />
      {!sidebar && <Hideable id="top.project"><button type="button" className="chip" title={`${chat.cwd}\nClick to add another project folder`} onClick={addProject}><Icon name="folder" size={14} />{chat.project}</button></Hideable>}
      <UpdatePill />
      <Hideable id="top.panel"><PanelButton /></Hideable>
      <WindowButtons />
    </header>
  );
}

function DeleteDialog() {
  const deleting = useUi((u) => u.deleting);
  useEffect(() => {
    if (!deleting) return;
    const key = (e: KeyboardEvent) => {
      // Enter deletes unless another button has focus: Enter on Cancel cancels.
      const focused = document.activeElement;
      if (e.key === 'Enter' && !(focused instanceof HTMLButtonElement && !focused.classList.contains('danger'))) {
        e.preventDefault();
        act('deleteChat', { id: deleting.id });
        ui.set({ deleting: null });
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [deleting]);
  if (!deleting) return null;
  const close = () => ui.set({ deleting: null });
  return (
    <Modal onClose={close}>
      <b className="strong big">Delete this chat?</b>
      <div className="ellipsis">{deleting.title}</div>
      <p className="small dim">It's removed from this PC for good. Files Scout changed stay as they are.</p>
      <div className="row end">
        <button type="button" className="button" onClick={close}>Cancel</button>
        <button type="button" className="button danger" onClick={() => { act('deleteChat', { id: deleting.id }); close(); }}>Delete</button>
      </div>
    </Modal>
  );
}

/** A paragraph of the server's terms: text with <a href="https://…">links</a>, drawn without trusting it as HTML. */
function TermsText({ text }: { text: string }) {
  const parts: React.ReactNode[] = [];
  const re = /<a href="(https:\/\/[^"]+)">([^<]+)<\/a>/g;
  let at = 0;
  for (const m of text.matchAll(re)) {
    parts.push(text.slice(at, m.index).replace(/<[^>]*>/g, ''));
    const url = m[1];
    parts.push(<a key={m.index} href={url} onClick={(e) => { e.preventDefault(); openExternal(url); }}>{m[2]}</a>);
    at = m.index! + m[0].length;
  }
  parts.push(text.slice(at).replace(/<[^>]*>/g, ''));
  return <p>{parts}</p>;
}

/**
 * Picking a model an admin marked uncensored: it needs a ScoutCode account that agreed to the uncensored model terms.
 * Signed out, this says so and offers the sign-in; signed in, it shows the terms, and agreeing records it on the server
 * (for the whole account, API keys included) and then picks the model.
 */
function UncensoredDialog() {
  const pending = useUi((u) => u.uncensored);
  const terms = useView('uncensored');
  const login = useView('login');
  const [agreed, setAgreed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setAgreed(false); setError(null); }, [pending]);
  // Signing in from here carries on to the terms once the new list says so.
  useEffect(() => {
    if (pending && terms?.signedIn && terms.accepted) {
      pending.pick();
      ui.set({ uncensored: null });
    }
  }, [pending, terms?.signedIn, terms?.accepted]);
  if (!pending) return null;
  const close = () => ui.set({ uncensored: null });
  if (!terms?.signedIn) {
    return (
      <Modal onClose={close} className="uncensored-dialog">
        <b className="strong big">Sign in to use {pending.name}</b>
        <p className="small dim">{pending.name} is an uncensored model. Uncensored models need a ScoutCode account, and agreeing to their terms.</p>
        <div className="row end">
          <button type="button" className="button" onClick={close}>Cancel</button>
          <button type="button" className="button primary" disabled={login.busy} onClick={() => act('browserLogin')}>{login.busy ? 'Signing in…' : 'Sign in with your browser'}</button>
        </div>
      </Modal>
    );
  }
  const agree = () => {
    setBusy(true);
    setError(null);
    invoke('uncensored_accept', { version: terms.version })
      .then(() => { pending.pick(); close(); }, (e) => setError(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Modal onClose={close} className="uncensored-dialog">
      <b className="strong big">{pending.name} is uncensored</b>
      <div className="uncensored-terms small">
        {terms.text.map((t, i) => <TermsText key={i} text={t} />)}
      </div>
      <label className="row small uncensored-agree">
        <input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} />
        <span>I'm 18 or older and I agree to these terms. I won't use uncensored models for anything illegal.</span>
      </label>
      {error && <div className="small red">{error}</div>}
      <div className="row end">
        <button type="button" className="button" onClick={close}>Cancel</button>
        <button type="button" className="button primary" disabled={!agreed || busy} onClick={agree}>{busy ? 'Saving…' : 'Agree and use it'}</button>
      </div>
    </Modal>
  );
}

/** A linked chatbot's call that needs the user's OK. It can come while Scout is on something else. */
function LinkPermission() {
  const { permission } = useView('link');
  if (!permission) return null;
  return (
    <Modal onClose={() => {}}>
      <b className="strong big">A chatbot wants to use Scout</b>
      <div className="muted">{permission.tool === 'bash' ? 'Run this command?' : 'Make this change?'}</div>
      <PermissionBody p={permission} link onAnswer={(answer, feedback) => act('answerLink', { answer, feedback })} />
    </Modal>
  );
}

function Login({ onDone }: { onDone: () => void }) {
  // No header over the steps: the welcome says hello, and each step has its own title.
  return (
    <div className="login-screen">
      <header className="login-bar" data-tauri-drag-region><WindowButtons /></header>
      <div className="login-body">
        <Onboarding onDone={onDone} />
      </div>
      <FolderPicker />
    </div>
  );
}

/** The window's own shortcuts. The terminal and the editor want most keys for themselves. */
function useShortcuts() {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      // Delta has its own keys; Ctrl+N still makes a chat.
      if (ui.get().page === 'delta') {
        if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'n') {
          e.preventDefault();
          ui.set({ page: 'chat' });
          act('newChat');
          return;
        }
        if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 't') {
          e.preventDefault();
          deltaTabs.addTab();
          return;
        }
        if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'w') {
          e.preventDefault();
          deltaTabs.closeActiveTab();
          return;
        }
        if (e.ctrlKey && e.key === 'Tab') {
          e.preventDefault();
          if (e.shiftKey) deltaTabs.prevTab();
          else deltaTabs.nextTab();
          return;
        }
        if (e.ctrlKey && !e.shiftKey && e.key >= '1' && e.key <= '9') {
          e.preventDefault();
          deltaTabs.switchToIndex(parseInt(e.key, 10) - 1);
          return;
        }
        return;
      }
      // The Inbox and Usage have no chat on screen: only Ctrl+N, which makes one.
      if (ui.get().page === 'inbox' || ui.get().page === 'usage') {
        if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'n') {
          e.preventDefault();
          ui.set({ page: 'chat' });
          act('newChat');
        }
        return;
      }
      if (e.ctrlKey && e.key === '`') {
        e.preventDefault();
        return toggleDock('terminal');
      }
      // The panel's tabs: Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+Tab, Ctrl+1–9 (see handleDockKeys).
      if (handleDockKeys(e)) return;
      if (document.activeElement?.closest('.dock')) return;
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        return ui.set({ findOpen: true, findEpoch: ui.get().findEpoch + 1 });
      }
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        // In a cloud chat, a new one goes to the same repository.
        if (ui.get().cloud && ui.get().page === 'chat') return newCloudChat();
        ui.set({ page: 'chat', cloud: null });
        return act('newChat');
      }
      // Only from the chat box: everywhere else Shift+Tab moves focus back, as usual.
      if (e.shiftKey && e.key === 'Tab' && document.activeElement?.matches('.composer textarea.prompt')) {
        e.preventDefault();
        return act('cycleMode');
      }
      // Esc closes whatever is open on top before it stops Scout.
      if (e.key === 'Escape' && !e.defaultPrevented && !ui.get().settings && !ui.get().deleting && !ui.get().pickingFolder && !ui.get().pickingFiles && !ui.get().newProject) {
        const at = ui.get().cloud;
        if (!at) return act('stop');
        // In a cloud chat, Esc stops Epsilon, as it stops Scout in a local one.
        const t = cloud.get().task;
        if (at.taskId && t?.id === at.taskId && LIVE.has(t.status)) void stopTask(t.id);
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
}

export function App() {
  const { loginNeeded, team } = useView('cfg');
  const sidebar = useUi((u) => u.sidebar);
  const page = useUi((u) => u.page);
  const dropping = useUi((u) => u.dropping);
  // With no chat open yet, the box sits in the middle under the greeting; it moves down once there's a transcript.
  const fresh = useCore((s) => s.items.length === 0);
  const inCloud = useUi((u) => !!u.cloud);
  useShortcuts();
  useEpsilonSync();
  useAutoTaskTabs();
  // Opening or starting a chat on this PC (from anywhere: the Inbox, a notification) puts it back on screen.
  const chatEpoch = useView('chat').epoch;
  useEffect(() => { ui.set({ cloud: null }); }, [chatEpoch]);
  useActivityPing();
  useEffect(() => {
    const offs = [
      listen<string[]>('dropped', async (e) => {
        ui.set({ dropping: false });
        const cwd = core.get().view.chat.cwd;
        const shown = await Promise.all(e.payload.map((path) => invoke<string>('display_path', { cwd, path })));
        const had = ui.get().attachments;
        ui.set({ attachments: [...had, ...shown.filter((f) => !had.includes(f))] });
        focusComposer();
      }),
      listen('drop-hover', () => ui.set({ dropping: true })),
      listen('drop-leave', () => ui.set({ dropping: false })),
      // Scout opened a page in the built-in browser: the panel shows it, so the user sees what it does.
      listen('browser-show', () => {
        const u = ui.get();
        if (u.page === 'chat' && !(u.dockOpen && u.dockTab === 'browser')) ui.set({ dockOpen: true, dockTab: 'browser' });
      }),
    ];
    // Links never navigate the window; anything that gets past their own handlers stops here.
    const click = (e: MouseEvent) => {
      if ((e.target as Element).closest?.('a')) e.preventDefault();
    };
    document.addEventListener('click', click);
    return () => {
      offs.forEach((o) => o.then((f) => f()));
      document.removeEventListener('click', click);
    };
  }, []);

  // Once it's shown, the first-run screen stays up while the user links more providers, until they press Continue. A
  // Scout Enterprise team's member skips it as soon as they've signed in: the team has set their providers up.
  // It can also be reopened on purpose ("Run setup again"), which keeps the providers already linked.
  const onboarding = useUi((u) => u.onboarding);
  useEffect(() => {
    if (loginNeeded) ui.set({ onboarding: true });
  }, [loginNeeded]);
  useEffect(() => {
    if (team) ui.set({ onboarding: false });
  }, [!!team]);
  if (!team && (loginNeeded || onboarding)) return <Login onDone={() => ui.set({ onboarding: false })} />;
  return (
    <div className="app">
      <NavRail />
      {sidebar && page === 'chat' && <Sidebar />}
      {sidebar && page === 'epsilon' && <EpsilonSidebar />}
      <div className="main">
        <TopBar />
        {page === 'delta' ? <DeltaPage /> : page === 'inbox' ? <Inbox /> : page === 'usage' ? <UsagePage /> : page === 'epsilon' ? <VmPage /> : inCloud ? (
          <div className="main-body"><CloudChat /></div>
        ) : (
          <div className="main-body">
            <div className={`chat ${fresh ? 'fresh' : ''}`}>
              {fresh && <AsciiBackdrop />}
              <Transcript />
              <Composer />
            </div>
            <Dock />
          </div>
        )}
      </div>
      <FloatingGame />
      <FloatingMusic />
      <Settings windowButtons={<WindowButtons />} />
      <DeleteDialog />
      <UncensoredDialog />
      <HideMenu />
      <FolderPicker />
      <NewProjectDialog />
      <LinkPermission />
      <WhatsNew />
      <UpdateCard />
      <UsageAlert />
      {dropping && <div className="drop-overlay"><div><Icon name="paperclip" /> Drop files to attach them</div></div>}
    </div>
  );
}
