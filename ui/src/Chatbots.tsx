import { useEffect, useState } from 'react';
import { act } from './api';
import { ui, useCore, useView } from './store';
import { CopyButton, SectionLabel, Spinner } from './widgets';

/** A chatbot that reached ScoutCode this recently is counted as connected. */
const ACTIVE_MS = 5 * 60_000;

function ago(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 10) return 'just now';
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

/** Re-renders now and then, so "2m ago" stays true. */
function useNow(every = 15_000): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), every);
    return () => clearInterval(t);
  }, [every]);
  return now;
}

function Dot({ tone }: { tone: 'on' | 'wait' | 'off' }) {
  return <span className={`link-dot ${tone}`} aria-hidden />;
}

function openSettings() {
  ui.set({ settings: 'chatbots' });
}

/**
 * The chatbots option in the model menu: the link that lets web chatbots like claude.ai and ChatGPT use Scout's tools
 * here over MCP. How to set it up, whether it's on, which chatbots are connected, and what they've done.
 */
export function ChatbotsPanel({ close }: { close: () => void }) {
  const link = useView('link');
  const cfg = useView('cfg');
  const modes = useCore((s) => s.statics.modes);
  const [mode, setMode] = useState('default');
  const now = useNow();
  const canLink = cfg.hasKey && !!link.cwd;
  const address = `${cfg.baseUrl}/mcp`;
  const tone = link.online ? 'on' : link.on ? 'wait' : 'off';
  const headline = link.online ? 'Connected to ScoutCode' : link.on ? 'Connecting…' : 'Off';
  const last = link.log[link.log.length - 1];
  const modeLabel = (id: string | null) => modes.find((m) => m.id === id)?.label ?? id ?? '';
  const added = link.edits.reduce((n, e) => n + e.added, 0);
  const removed = link.edits.reduce((n, e) => n + e.removed, 0);

  return (
    <div className="stack chatbots-panel">
      <div className="link-card stack">
        <div className="row">
          <Dot tone={tone} />
          <b className="strong grow">{headline}</b>
          <button type="button" className={`button ${link.on ? '' : 'primary'}`} disabled={!link.on && !canLink} onClick={() => act('toggleLink', { mode })}>
            {link.on ? 'Turn off' : 'Turn on'}
          </button>
        </div>
        {link.status && link.status !== headline && <div className="tiny dim">{link.status}</div>}
        {!cfg.hasKey && <div className="tiny dim">This goes through your ScoutCode account, so sign in first.</div>}
        <div className="stat-row"><span className="small dim">Folder</span><span className="small ellipsis" title={link.cwd}>{link.cwd || 'Open a project first'}</span></div>
        {link.on ? (
          <div className="stat-row"><span className="small dim">Allowed</span><span className="small">{modeLabel(link.mode)}</span></div>
        ) : (
          <div className="stat-row">
            <span className="small dim">Allowed</span>
            <select className="input select" value={mode} onChange={(e) => setMode(e.target.value)}>
              {modes.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
            </select>
          </div>
        )}
        {!link.on && mode === 'yolo' && <div className="tiny red">Yolo lets a chatbot run any command here without asking.</div>}
        {link.since && <div className="stat-row"><span className="small dim">On since</span><span className="small">{ago(link.since, now)}</span></div>}
      </div>

      {link.on && (
        <>
          <SectionLabel>Chatbots</SectionLabel>
          {link.chatbots.length === 0 ? (
            <div className="small dim">None linked to your account yet. Follow the steps below.</div>
          ) : link.chatbots.map((b) => {
            const active = b.seen !== null && now - b.seen < ACTIVE_MS;
            return (
              <div key={b.name} className="row">
                <Dot tone={active ? 'on' : 'off'} />
                <span className="small grow ellipsis">{b.name}</span>
                <span className="tiny dim">{b.seen ? ago(b.seen, now) : 'not seen yet'}</span>
              </div>
            );
          })}

          <SectionLabel>Activity</SectionLabel>
          {link.running && (
            <div className="row"><Spinner /><span className="mono small ellipsis grow" title={link.running}>{link.running.split('\n')[0]}</span></div>
          )}
          {last ? (
            <div className="stack tight">
              <div className="row">
                <span className={`tiny ${last.ok ? 'green' : 'red'}`}>{last.ok ? 'Done' : 'Failed'}</span>
                <span className="mono small ellipsis grow" title={`${last.tool} ${last.label}`}>{last.tool} <span className="dim">{last.label.split('\n')[0]}</span></span>
                <span className="tiny dim">{ago(last.at, now)}</span>
              </div>
              <div className="tiny dim">{link.log.length} call{link.log.length === 1 ? '' : 's'} so far{link.log.some((c) => !c.ok) ? `, ${link.log.filter((c) => !c.ok).length} failed or refused` : ''}.</div>
            </div>
          ) : !link.running && <div className="small dim">No calls yet.</div>}

          <div className="row">
            <SectionLabel>Edits</SectionLabel>
            <span className="grow" />
            {link.edits.length > 0 && <span className="tiny"><span className="green">+{added}</span> <span className="red">−{removed}</span></span>}
          </div>
          {link.edits.length === 0 ? <div className="small dim">No files changed yet.</div> : (
            <>
              {link.edits.map((e) => (
                <div key={e.path} className="row">
                  <span className="mono small ellipsis grow" title={e.path}>{e.path}</span>
                  {e.created && <span className="badge">New</span>}
                  <span className="tiny"><span className="green">+{e.added}</span> <span className="red">−{e.removed}</span></span>
                </div>
              ))}
              <button type="button" className="button" onClick={() => { ui.set({ dockOpen: true, dockTab: 'changes' }); close(); }}>Review in Changes</button>
            </>
          )}
        </>
      )}

      <SectionLabel>How to use it</SectionLabel>
      <ol className="small how-to">
        <li>Turn the link on here. Scout only takes calls while it's on, for the folder above.</li>
        <li>
          In claude.ai, ChatGPT or another chatbot, add a custom connector with this address:
          <div className="row address"><span className="mono small ellipsis grow" title={address}>{address}</span><CopyButton text={address} /></div>
        </li>
        <li>Sign in to ScoutCode when the chatbot asks, and allow it.</li>
        <li>Ask the chatbot to work on your project. Its model does the thinking; files and commands run here, and what the mode doesn't allow waits for your OK.</li>
      </ol>
      <div className="tiny dim">Nothing is spent on ScoutCode. For apps on this PC like Claude Desktop or Cursor, and to unlink a chatbot, see <button type="button" className="text-button" onClick={() => { openSettings(); close(); }}>Settings &gt; Chatbots</button>.</div>
    </div>
  );
}
