import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { act, invoke } from './api';
import { AdditionalOptions, BrowserEntry, KeyField, NvidiaEntry, OPENCODE_WARNING, OpencodeEntry, ScoutKeyEntry, SignInEntry } from './Accounts';
import { Composer, PermissionBody } from './Composer';
import { Dock } from './Dock';
import { Settings, restartBlocker } from './Settings';
import { addProject, Sidebar } from './Sidebar';
import { core, focusComposer, ui, useCore, useUi, useView, type DockTab } from './store';
import { Transcript } from './Transcript';
import { Icon, IconButton, Modal } from './widgets';
import logo from '../../assets/icon.png';

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
  return (
    <button type="button" className="button primary pill" disabled={!!blocker} title={blocker ?? `Scout ${status.version} is ready. Your chat reopens after the restart.`} onClick={() => invoke('restart_to_update')}>Restart to update</button>
  );
}

function TopBar() {
  const sidebar = useUi((u) => u.sidebar);
  const dockOpen = useUi((u) => u.dockOpen);
  const dockTab = useUi((u) => u.dockTab);
  const chat = useView('chat');
  return (
    <header className="top-bar" data-tauri-drag-region>
      {!sidebar && <IconButton icon="sidebar" tip="Show sidebar" onClick={() => ui.set({ sidebar: true })} />}
      <b className="chat-title ellipsis" data-tauri-drag-region>{chat.title}</b>
      <span className="grow" data-tauri-drag-region />
      {/* The sidebar shows the folder; without it, the bar does. */}
      {!sidebar && <button type="button" className="chip" title={`${chat.cwd}\nClick to add another project folder`} onClick={addProject}><Icon name="folder" size={14} />{chat.project}</button>}
      <UpdatePill />
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

function Divider({ children }: { children: string }) {
  return <div className="divider"><span>{children}</span></div>;
}

function Login() {
  const kiloWarning = useCore((s) => s.statics.kiloWarning);
  return (
    <div className="login-screen">
      <header className="login-bar" data-tauri-drag-region><WindowButtons /></header>
      <div className="login-scroll">
        <div className="login-column">
          <img src={logo} width={52} height={52} alt="" />
          <h1>Welcome to Scout</h1>
          <p className="dim">A coding agent for your projects, powered by ScoutCode.</p>
          <div className="login-card stack">
            <BrowserEntry />
            <div className="tiny dim">No key needed: uses your account's monthly desktop credit.</div>
            <Divider>or use an API key</Divider>
            <ScoutKeyEntry />
            <Divider>or use your OpenRouter key</Divider>
            <KeyField hint="sk-or-…" button="Save OpenRouter key" onSave={(key) => act('addOpenrouterKey', { name: 'OpenRouter', key })} />
            <Divider>or use your NVIDIA NIM key</Divider>
            <NvidiaEntry />
            <Divider>or use OpenCode</Divider>
            <OpencodeEntry />
            <div className="tiny dim">No account needed for its free models. {OPENCODE_WARNING}</div>
            <Divider>or use your own GitHub Copilot plan</Divider>
            <div className="tiny dim">About 150 MB to install.</div>
            <SignInEntry which="copilot" />
            <Divider>or use Kilo Code's free models</Divider>
            <button type="button" className="button wide" onClick={() => act('saveKilo', { enabled: true, key: null })}>Use Kilo Code</button>
            <div className="tiny dim">No account or key. {kiloWarning}</div>
            <Divider>or use one of these</Divider>
            <AdditionalOptions />
          </div>
        </div>
      </div>
    </div>
  );
}

/** The window's own shortcuts. The terminal and the editor want most keys for themselves. */
function useShortcuts() {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
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
      if (e.key === 'Escape' && !e.defaultPrevented && !ui.get().settings && !ui.get().deleting) act('stop');
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
}

export function App() {
  const { loginNeeded } = useView('cfg');
  const sidebar = useUi((u) => u.sidebar);
  const dropping = useUi((u) => u.dropping);
  useShortcuts();
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

  if (loginNeeded) return <Login />;
  return (
    <div className="app">
      {sidebar && <Sidebar />}
      <div className="main">
        <TopBar />
        <div className="main-body">
          <div className="chat">
            <Transcript />
            <Composer />
          </div>
          <Dock />
        </div>
      </div>
      <Settings />
      <DeleteDialog />
      <LinkPermission />
      {dropping && <div className="drop-overlay"><div><Icon name="paperclip" /> Drop files to attach them</div></div>}
    </div>
  );
}
