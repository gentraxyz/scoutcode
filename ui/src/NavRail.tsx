// The narrow rail down the left edge, like the website's: Chats, Inbox and Issues, with the account at the
// bottom. It stays when the chats sidebar is hidden.
import { act, openExternal } from './api';
import { ui, useCore, useUi, useView } from './store';
import { Icon, MenuItem, Popover } from './widgets';
import { ScoutLogo } from './logo';
import { DeltaMark } from './delta/Delta';
import { Hideable } from './Hideable';
import { inboxChats } from './Inbox';
import { useUnseenDreams } from './Dreams';
import type { ReactNode } from 'react';

function RailButton({ on, tip, onClick, badge, children }: { on: boolean; tip: string; onClick: () => void; badge?: number; children: ReactNode }) {
  return (
    <button type="button" className={`rail-btn ${on ? 'on' : ''}`} onClick={onClick} title={tip} aria-label={tip}>
      {children}
      {!!badge && <span className="rail-badge">{badge > 99 ? '99+' : badge}</span>}
    </button>
  );
}

/** Who's signed in, as the account menu names them. */
function useAccountName() {
  const cfg = useView('cfg');
  const { additionalProviders } = useCore((s) => s.statics);
  const linked = additionalProviders.find((a) => a.id === cfg.additional[0]?.id)?.name;
  return cfg.username ?? (cfg.browserSignIn ? 'Signed in' : cfg.hasKey ? 'API key' : cfg.openrouterKeys.length ? 'OpenRouter key' : cfg.nvidiaKey ? 'NVIDIA key' : cfg.claude ? 'Claude Code' : cfg.codex ? 'Codex' : cfg.opencode ? 'OpenCode' : cfg.grokSignedIn || cfg.grokKey ? 'Grok Build' : cfg.hermesSignedIn || cfg.hermesKey ? 'Hermes' : linked ?? (cfg.guest ? 'No account' : 'Kilo Code'));
}

function Account() {
  const cfg = useView('cfg');
  const name = useAccountName();
  const { version } = useView('updater');
  return (
    <Popover side="top" className="profile-menu" width={248} trigger={(open, toggle) => (
      <button type="button" className={`rail-avatar ${open ? 'open' : ''}`} onClick={toggle} title={name} aria-label="Account menu">
        <span className="avatar">{name.charAt(0).toUpperCase()}</span>
      </button>
    )}>
      {(close) => (
        <>
          <div className="profile-head">
            <span className="avatar big">{name.charAt(0).toUpperCase()}</span>
            <div className="min0">
              <div className="strong ellipsis">{name}</div>
              <div className="tiny dim">Scout {version}</div>
            </div>
          </div>
          <hr />
          <MenuItem icon="gear" onClick={() => { close(); ui.set({ settings: 'appearance' }); }}>Settings</MenuItem>
          <MenuItem icon="chart" onClick={() => { close(); ui.set({ settings: 'usage' }); }}>Usage</MenuItem>
          <hr />
          <MenuItem icon="download" onClick={() => { close(); ui.set({ settings: 'updates' }); act('checkUpdates'); }}>Check for updates</MenuItem>
          <MenuItem icon="sparkle" onClick={() => { close(); ui.set({ settings: null, onboarding: true }); }}>Run setup again</MenuItem>
          <MenuItem icon="user" onClick={() => { close(); openExternal(`${cfg.baseUrl.replace(/\/v1$/, '')}/account`); }}>Account page</MenuItem>
        </>
      )}
    </Popover>
  );
}

export function NavRail() {
  const page = useUi((u) => u.page);
  const sessions = useView('sessions');
  // The badge counts what wants you: chats waiting on an answer, and finished ones you haven't read.
  const { waiting, unread } = inboxChats(sessions);
  const dreams = useUnseenDreams();
  return (
    <nav className="nav-rail" aria-label="Scout">
      <div className="rail-logo" data-tauri-drag-region><ScoutLogo size={24} drag /></div>
      <RailButton on={page === 'chat'} tip="Chats" onClick={() => ui.set({ page: 'chat' })}><Icon name="chat" size={18} /></RailButton>
      <Hideable id="rail.inbox">
        <RailButton on={page === 'inbox'} tip="Inbox: what's in progress and what's yours" badge={waiting.length + unread.length + dreams.length} onClick={() => ui.set({ page: 'inbox' })}><Icon name="inbox" size={18} /></RailButton>
      </Hideable>
      <Hideable id="rail.issues">
        <RailButton on={page === 'delta'} tip="Issues: projects, cycles and docs" onClick={() => ui.set({ page: 'delta' })}><DeltaMark size={17} /></RailButton>
      </Hideable>
      <span className="grow" data-tauri-drag-region />
      <Account />
    </nav>
  );
}
