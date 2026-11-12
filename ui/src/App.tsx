import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { act, invoke, openExternal } from './api';
import { AsciiBackdrop } from './AsciiBackdrop';
import { Composer, PermissionBody } from './Composer';
import { Dock } from './Dock';
import { closeGame, FloatingGame, gameShowing, openGame, useGameAutomation } from './Games';
import { closeMusic, FloatingMusic, openMusic, useMusicPlaying } from './Music';
import { Onboarding } from './Onboarding';
import { Settings, restartBlocker } from './Settings';
import { addProject, Sidebar } from './Sidebar';
import { Hideable, HideMenu } from './Hideable';
import { core, focusComposer, ui, useCore, useUi, useView, type DockTab } from './store';
import { Transcript } from './Transcript';
import { DeltaPage } from './DeltaPage';
import { Inbox } from './Inbox';
import { NavRail } from './NavRail';
import { DeltaTabsBar } from './delta/DeltaTabsBar';
import { deltaTabs } from './delta/tabs';
import { FolderPicker } from './FolderPicker';
import { UsageAlert } from './UsageAlert';
import { Icon, IconButton, Modal } from './widgets';
import { useActivityPing } from './Dreams';

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
  return <IconButton icon="game" tip={showing ? 'Close the game' : 'Play a game while Scout works'} active={showing} onClick={() => (showing ? closeGame() : openGame(games, true))} />;
}

function MusicButton() {
  const { music } = useView('cfg');
  const showing = useUi((u) => u.musicOpen);
  const playing = useMusicPlaying();
  if (!music.on) return null;
  return (
    <IconButton
      icon="music"
      tip={showing ? 'Close music player' : playing ? 'Music playing — open player' : 'Open music player'}
      active={showing || playing}
      onClick={() => (showing ? closeMusic() : openMusic())}
    />
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
  const dockOpen = useUi((u) => u.dockOpen);
  const dockTab = useUi((u) => u.dockTab);
  const chat = useView('chat');
  const page = useUi((u) => u.page);
  // Issues and Inbox have no chats sidebar (the rail gets you back), so no button to show it either.
  if (page === 'inbox') {
    return (
      <header className="top-bar" data-tauri-drag-region>
        <Icon name="inbox" size={15} />
        <b className="chat-title inbox-title" data-tauri-drag-region>Inbox</b>
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
      <Hideable id="top.music"><MusicButton /></Hideable>
      <Hideable id="top.game"><GameButton /></Hideable>
      <Hideable id="top.panel"><IconButton icon="panel" tip={dockOpen ? 'Hide panel' : 'Show panel'} active={dockOpen} onClick={() => ui.set({ dockOpen: !dockOpen })} /></Hideable>
      <Hideable id="top.terminal"><IconButton icon="terminal" tip="Terminal (Ctrl+`)" active={dockOpen && dockTab === 'terminal'} onClick={() => toggleDock('terminal')} /></Hideable>
      <Hideable id="top.changes"><IconButton icon="changes" tip="Changes" active={dockOpen && dockTab === 'changes'} onClick={() => toggleDock('changes')} /></Hideable>
      <Hideable id="top.files"><IconButton icon="files" tip="Files" active={dockOpen && dockTab === 'files'} onClick={() => toggleDock('files')} /></Hideable>
      <Hideable id="top.review"><IconButton icon="shield" tip="Code Review: a second opinion from another model" active={dockOpen && dockTab === 'review'} onClick={() => toggleDock('review')} /></Hideable>
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
      // The Inbox has no chat on screen: only Ctrl+N, which makes one.
      if (ui.get().page === 'inbox') {
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
      // Only from the chat box: everywhere else Shift+Tab moves focus back, as usual.
      if (e.shiftKey && e.key === 'Tab' && document.activeElement?.matches('.composer textarea.prompt')) {
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
      <div className="main">
        <TopBar />
        {page === 'delta' ? <DeltaPage /> : page === 'inbox' ? <Inbox /> : (
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
      <Settings />
      <DeleteDialog />
      <UncensoredDialog />
      <HideMenu />
      <FolderPicker />
      <LinkPermission />
      <WhatsNew />
      <UsageAlert />
      {dropping && <div className="drop-overlay"><div><Icon name="paperclip" /> Drop files to attach them</div></div>}
    </div>
  );
}
