import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { act, invoke } from './api';
import { Composer, PermissionBody } from './Composer';
import { Dock } from './Dock';
import { closeGame, FloatingGame, gameShowing, openGame, useGameAutomation } from './Games';
import { Onboarding } from './Onboarding';
import { Settings, restartBlocker } from './Settings';
import { addProject, Sidebar } from './Sidebar';
import { core, focusComposer, ui, useCore, useUi, useView, type DockTab } from './store';
import { Transcript } from './Transcript';
import { DeltaPage } from './DeltaPage';
import { FolderPicker } from './FolderPicker';
import { Icon, IconButton, Modal } from './widgets';
import { ScoutLogo } from './logo';

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
  ui.set(dockOpen && dockTab === tab ? { dockOpen: false } : { dockOpen: true, dockTab: tab });
}

function UpdatePill() {
  const { status } = useView('updater');
  const { anyRunning } = useView('run');
  const dirty = useUi((u) => u.editorDirty);
  if (status.state !== 'ready') return null;
  const blocker = restartBlocker(anyRunning, dirty);
  const summary = status.summary.trim() ? `\n${status.summary.trim()}` : '';
  return (
    <button type="button" className="button primary pill" disabled={!!blocker} title={blocker ?? `Scout ${status.version} is ready. Your chat reopens after the restart.${summary}`} onClick={() => invoke('restart_to_update')}>Restart to update</button>
  );
}

/** Shown once after Scout updates itself: what changed since the version that ran before. */
function WhatsNew() {
  const releases = useView('whatsNew');
  if (!releases.length) return null;
  const close = () => act('dismissWhatsNew');
  return (
    <Modal onClose={close} className="whats-new">
      <b className="strong big">Scout updated to {releases[0].version}</b>
      <div className="whats-new-list stack-wide">
        {releases.map((r) => (
          <div key={r.version} className="stack">
            {releases.length > 1 && <div className="section-label">Version {r.version}</div>}
            {r.summary.trim() && <div className="strong">{r.summary}</div>}
            {r.changelog.trim() && <div className="muted pre-wrap small">{r.changelog}</div>}
          </div>
        ))}
      </div>
      <div className="row end"><button type="button" className="button primary" onClick={close}>Got it</button></div>
    </Modal>
  );
}

function GameButton() {
  const { games } = useView('cfg');
  const showing = useUi(() => gameShowing(games));
  if (!games.on) return null;
  return <IconButton icon="game" tip={showing ? 'Close the game' : 'Play 2048 while Scout works'} active={showing} onClick={() => (showing ? closeGame() : openGame(games, true))} />;
}

function TopBar() {
  const sidebar = useUi((u) => u.sidebar);
  const dockOpen = useUi((u) => u.dockOpen);
  const dockTab = useUi((u) => u.dockTab);
  const chat = useView('chat');
  const page = useUi((u) => u.page);
  if (page === 'delta') {
    return (
      <header className="top-bar" data-tauri-drag-region>
        {!sidebar && <IconButton icon="sidebar" tip="Show sidebar" onClick={() => ui.set({ sidebar: true })} />}
        <b className="chat-title ellipsis" data-tauri-drag-region>Delta</b>
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
      {!sidebar && <button type="button" className="chip" title={`${chat.cwd}\nClick to add another project folder`} onClick={addProject}><Icon name="folder" size={14} />{chat.project}</button>}
      <UpdatePill />
      <GameButton />
      <IconButton icon="panel" tip={dockOpen ? 'Hide panel' : 'Show panel'} active={dockOpen} onClick={() => ui.set({ dockOpen: !dockOpen })} />
      <IconButton icon="terminal" tip="Terminal (Ctrl+`)" active={dockOpen && dockTab === 'terminal'} onClick={() => toggleDock('terminal')} />
      <IconButton icon="changes" tip="Changes" active={dockOpen && dockTab === 'changes'} onClick={() => toggleDock('changes')} />
      <IconButton icon="files" tip="Files" active={dockOpen && dockTab === 'files'} onClick={() => toggleDock('files')} />
      <WindowButtons />
    </header>
  );
}

function DeleteDialog() {
  const deleting = useUi((u) => u.deleting);
  useEffect(() => {
    if (!deleting) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Enter') {
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
  return (
    <div className="login-screen">
      <header className="login-bar" data-tauri-drag-region><WindowButtons /></header>
      <div className="login-scroll">
        <div className="login-column">
          <ScoutLogo size={52} />
          <h1>Welcome to Scout</h1>
          <p className="dim">A coding agent for your projects, powered by ScoutCode.</p>
          <Onboarding onDone={onDone} />
        </div>
      </div>
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
        }
        return;
      }
      if (e.ctrlKey && e.key === '`') {
        e.preventDefault();
        return toggleDock('terminal');
      }
      if (document.activeElement?.closest('.dock')) return;
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        return ui.set({ findOpen: true, findEpoch: ui.get().findEpoch + 1 });
      }
      if (e.ctrlKey && e.shiftKey && e.code === 'Space') {
        e.preventDefault();
        return act('toggleVoice');
      }
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        return act('newChat');
      }
      if (e.shiftKey && e.key === 'Tab' && !document.activeElement?.closest('.modal, .popover')) {
        e.preventDefault();
        return act('cycleMode');
      }
      // Esc closes whatever is open on top before it stops Scout.
      if (e.key === 'Escape' && !e.defaultPrevented && !ui.get().settings && !ui.get().deleting && !ui.get().pickingFolder) act('stop');
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
  useShortcuts();
  useGameAutomation();
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
      listen<string>('dictated', (e) => {
        const input = ui.get().input;
        ui.set({ input: input && !/\s$/.test(input) ? `${input} ${e.payload}` : input + e.payload });
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
  const [onboarding, setOnboarding] = useState(false);
  useEffect(() => {
    if (loginNeeded) setOnboarding(true);
  }, [loginNeeded]);
  useEffect(() => {
    if (team) setOnboarding(false);
  }, [!!team]);
  if (!team && (loginNeeded || onboarding)) return <Login onDone={() => setOnboarding(false)} />;
  return (
    <div className="app">
      {sidebar && <Sidebar />}
      <div className="main">
        <TopBar />
        {page === 'delta' ? <DeltaPage /> : (
          <div className="main-body">
            <div className={`chat ${fresh ? 'fresh' : ''}`}>
              <Transcript />
              <Composer />
            </div>
            <Dock />
          </div>
        )}
      </div>
      <FloatingGame />
      <Settings />
      <DeleteDialog />
      <FolderPicker />
      <LinkPermission />
      <WhatsNew />
      {dropping && <div className="drop-overlay"><div><Icon name="paperclip" /> Drop files to attach them</div></div>}
    </div>
  );
}
