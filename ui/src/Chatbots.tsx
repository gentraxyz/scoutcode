import { useEffect, useRef, useState } from 'react';
import { act, openExternal } from './api';
import { CLAUDE_NOTE, CODEX_NOTE, GROK_NOTE, GrokEntry, SignInEntry } from './Accounts';
import { ClaudeSetup } from './ClaudeSetup';
import { WEB_BOTS, type WebBot } from './webbots';
import { ui, useCore, useView } from './store';
import { CopyButton, Spinner } from './widgets';

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

/** The chatbots Scout can add itself to (ClaudeSetup.tsx), by id; the rest get their steps. */
const AUTO: Record<string, 'claude' | 'gemini'> = { 'claude-web': 'claude', gemini: 'gemini' };

/**
 * The model menu's chatbot pages, by rail tab: each one's WEB_BOTS id. Codex and Claude Code are the models of ChatGPT
 * and Claude, so their pages hold those too.
 */
type Page = { bot: string; connector: string; provider?: 'codex' | 'claude' | 'grok'; name?: string; site?: string; app?: string };
/**
 * Each page has the chatbot's connector, and for ChatGPT, Claude and Grok also its app's models (Codex, Claude Code,
 * Grok Build). When only one of the two is set up, the page asks about the other.
 */
const PAGES: Record<string, Page> = {
  codex: { bot: 'chatgpt', provider: 'codex', name: 'ChatGPT', site: 'chatgpt.com', app: 'Codex', connector: 'Use ChatGPT on chatgpt.com' },
  claude: { bot: 'claude-web', provider: 'claude', name: 'Claude', site: 'claude.ai', app: 'Claude Code', connector: 'Use Claude on claude.ai' },
  gemini: { bot: 'gemini', connector: 'Use Gemini on gemini.google.com' },
  grok: { bot: 'grok-web', provider: 'grok', name: 'Grok', site: 'grok.com', app: 'Grok Build', connector: 'Use Grok on grok.com' },
  chatbots: { bot: 'any', connector: 'Connect another chatbot' },
};
/** The pages that are only a chatbot: they list no models of their own. */
export const CHATBOT_ONLY_PAGES = ['gemini', 'chatbots'];
export const isBotPage = (tab: string): boolean => tab in PAGES;

type Linked = { name: string; seen: number | null };

/** The linked chatbots on a page: the ones named after its chatbot, or for Other, any that has a page of its own elsewhere. */
export function linkedTo(botId: string, bots: Linked[]): Linked[] {
  const named = (name: string, bot: WebBot) => bot.aliases.some((a) => name.toLowerCase().includes(a));
  if (botId === 'any') return bots.filter((b) => !WEB_BOTS.some((w) => w.id !== 'any' && named(b.name, w)));
  const bot = WEB_BOTS.find((w) => w.id === botId)!;
  return bots.filter((b) => named(b.name, bot));
}

/** Whether a page's chatbot has reached ScoutCode, for the rail's dimming. */
export const chatbotConnected = (tab: string, bots: Linked[]): boolean =>
  linkedTo(PAGES[tab].bot, bots).some((b) => b.seen !== null);

/** How to add Scout to a chatbot yourself: its steps, the address to paste, and a way to open it. */
function Steps({ bot, address }: { bot: WebBot; address: string }) {
  return (
    <div className="stack tight">
      <ol className="small how-to">
        {bot.steps.map((s, i) => (
          <li key={s.title}>
            <b className="strong">{s.title}.</b> <span>{s.detail}</span>
            {i === 1 && <div className="row address"><span className="mono small ellipsis grow" title={address}>{address}</span><CopyButton text={address} /></div>}
          </li>
        ))}
        <li>Start a chat and ask it to work on your project.</li>
      </ol>
      {bot.url && <button type="button" className="button compact" onClick={() => openExternal(bot.url!)}>Open {bot.name}</button>}
    </div>
  );
}

/** Folder name only, for a short line: C:\…\scout → scout. */
const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

/**
 * One chatbot's connection over MCP: lets it use Scout's tools here. Kept to one choice, set it up for me or I'll do it
 * myself, with whether it's working on top and the rest under More options.
 */
export function ChatbotsPanel({ close, botId, hideSetup = false }: { close: () => void; botId: string; hideSetup?: boolean }) {
  const link = useView('link');
  const cfg = useView('cfg');
  const modes = useCore((s) => s.statics.modes);
  const now = useNow();
  const [how, setHow] = useState<'auto' | 'manual' | null>(null);
  // With a chatbot connected, the list hides until "Connect another".
  const [adding, setAdding] = useState(false);
  const bot = WEB_BOTS.find((b) => b.id === botId)!;
  const address = `${cfg.baseUrl}/mcp`;
  const connected = linkedTo(botId, link.chatbots).filter((b) => b.seen !== null);
  const added = link.edits.reduce((n, e) => n + e.added, 0);
  const removed = link.edits.reduce((n, e) => n + e.removed, 0);
  const last = link.log[link.log.length - 1];

  // One line on whether chatbots can reach Scout, and if not, the one thing to do about it.
  const blocker = !cfg.hasKey ? 'Sign in to ScoutCode first.' : !link.cwd ? 'Open a project first: chatbots work in it.' : null;
  // The link can be up while this page's chatbot still isn't linked, so the headline is about that chatbot.
  const who = botId === 'any' ? 'another chatbot' : bot.name;
  const linked = connected.length > 0;
  const tone = blocker ? 'off' : linked ? 'on' : link.on ? 'wait' : 'off';
  const headline = blocker ? 'Not ready' : linked ? `Connected to ${who}` : link.online ? `Waiting for ${who}` : link.on ? 'Connecting…' : link.wanted ? 'Not running' : 'Off';
  const detail = blocker ?? (link.online || link.on ? `Working in ${folderName(link.cwd)}` : link.status || 'Turn it on to let chatbots use Scout here.');

  // Changing what chatbots may do restarts the link in the new mode.
  const setMode = (mode: string) => {
    if (link.on) act('toggleLink', { mode });
    act('toggleLink', { mode });
  };

  return (
    <div className="stack chatbots-panel">
      {!hideSetup && <div className="link-card row">
        <Dot tone={tone} />
        <div className="stack tight grow min0">
          <b className="strong small">{headline}</b>
          <span className="tiny dim ellipsis" title={link.cwd}>{detail}</span>
        </div>
        {!blocker && !link.on && <button type="button" className="button compact primary" onClick={() => act('toggleLink', { mode: '' })}>Turn on</button>}
      </div>}

      {connected.length > 0 && !adding ? (
        <div className="stack tight">
          {connected.map((b) => (
            <div key={b.name} className="row">
              <Dot tone={b.seen !== null && now - b.seen < ACTIVE_MS ? 'on' : 'off'} />
              <span className="small grow ellipsis">{b.name} is connected</span>
              <span className="tiny dim">{ago(b.seen!, now)}</span>
            </div>
          ))}
          <div className="tiny dim">Ask it to work on your project. <button type="button" className="text-button" onClick={() => setAdding(true)}>Connect another</button></div>
        </div>
      ) : hideSetup ? null : (
        <div className="stack">
          {AUTO[bot.id] ? (
            <>
              <div className="row chatbot-choice">
                <button type="button" className={`button grow${how !== 'manual' ? ' primary' : ''}`} disabled={!!blocker} onClick={() => setHow('auto')}>Set it up for me</button>
                <button type="button" className={`button grow${how === 'manual' ? ' primary' : ''}`} disabled={!!blocker} onClick={() => setHow('manual')}>I'll do it myself</button>
              </div>
              {how === 'auto' && <div className="link-card"><ClaudeSetup key={bot.id} address={address} disabled={!cfg.hasKey} bare bot={AUTO[bot.id]} /></div>}
              {how === 'manual' && <Steps bot={bot} address={address} />}
            </>
          ) : (
            <Steps bot={bot} address={address} />
          )}
        </div>
      )}

      {(link.running || last) && (
        <div className="stack tight">
          {link.running ? (
            <div className="row"><Spinner /><span className="mono small ellipsis grow" title={link.running}>{link.running.split('\n')[0]}</span></div>
          ) : last && (
            <div className="row">
              <span className={`tiny ${last.ok ? 'green' : 'red'}`}>{last.ok ? 'Done' : 'Failed'}</span>
              <span className="mono small ellipsis grow" title={`${last.tool} ${last.label}`}>{last.tool} <span className="dim">{last.label.split('\n')[0]}</span></span>
              <span className="tiny dim">{ago(last.at, now)}</span>
            </div>
          )}
          {link.edits.length > 0 && (
            <button type="button" className="button compact" onClick={() => { ui.set({ dockOpen: true, dockTab: 'changes' }); close(); }}>
              Review {link.edits.length} changed file{link.edits.length === 1 ? '' : 's'} <span className="green">+{added}</span> <span className="red">−{removed}</span>
            </button>
          )}
        </div>
      )}

      <details className="stack tight chatbot-more">
        <summary className="tiny dim">More options</summary>
        <div className="stat-row">
          <span className="small dim">May do without asking</span>
          <select className="input select" value={link.mode ?? 'default'} disabled={!!blocker} onChange={(e) => setMode(e.target.value)}>
            {modes.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
          </select>
        </div>
        {link.mode === 'yolo' && <div className="tiny red">Yolo lets a chatbot run any command here without asking.</div>}
        <div className="tiny dim">Anything else waits for your OK in Scout, with a notification.</div>
        {link.on && <button type="button" className="text-button tiny" onClick={() => act('toggleLink', { mode: '' })}>Turn the link off</button>}
        <div className="tiny dim">Nothing is spent on ScoutCode. Apps on this PC (Claude Desktop, Cursor) and unlinking: <button type="button" className="text-button" onClick={() => { openSettings(); close(); }}>Settings &gt; Chatbots</button>.</div>
      </details>
    </div>
  );
}

/**
 * A chatbot's page in the model menu. For ChatGPT and Claude it also holds the Codex or Claude Code sign-in and models,
 * so the chatbot and its models are one place.
 */
export function ChatbotPage({ tab, close }: { tab: string; close: () => void }) {
  const cfg = useView('cfg');
  const link = useView('link');
  const { bot: botId, provider, name, site, app, connector } = PAGES[tab];
  const signedIn = provider === 'codex' ? cfg.codex : provider === 'claude' ? cfg.claude : provider === 'grok' ? cfg.grokSignedIn || cfg.grokKey !== null : false;
  const connected = linkedTo(botId, link.chatbots).some((b) => b.seen !== null);
  const note = { codex: CODEX_NOTE, claude: CLAUDE_NOTE, grok: GROK_NOTE }[provider ?? 'codex'];
  // "Not now" hides the question for this visit only; it comes back the next time the page is opened.
  const [answered, setAnswered] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const connectorRef = useRef<HTMLElement>(null);
  // Only one side set up: ask about the other. Neither set up: the sign-in is the only way to see its models.
  const oneSide = !!provider && signedIn !== connected;
  const neither = !!provider && !signedIn && !connected;
  return (
    <div className="stack chatbot-page">
      {neither && (
        <div className="card-note stack tight">
          <b className="strong small">Models from {app}</b>
          <div className="small dim">{note}</div>
          {provider === 'grok' ? <GrokEntry /> : <SignInEntry which={provider!} compact />}
        </div>
      )}
      {oneSide && !answered && (signedIn ? (
        <div className="card-note stack tight">
          <b className="strong small">Do you want to use {name} via {site} for free?</b>
          <div className="row">
            <button type="button" className="button compact primary" onClick={() => { setAnswered(true); connectorRef.current?.scrollIntoView({ block: 'start' }); }}>Yes</button>
            <button type="button" className="button compact" onClick={() => setAnswered(true)}>Not now</button>
          </div>
        </div>
      ) : (
        <div className="card-note stack tight">
          <b className="strong small">Do you want to use {app} models too?</b>
          {signingIn ? (provider === 'grok' ? <GrokEntry /> : <SignInEntry which={provider!} compact />) : (
            <div className="row">
              <button type="button" className="button compact primary" onClick={() => setSigningIn(true)}>Yes</button>
              <button type="button" className="button compact" onClick={() => setAnswered(true)}>Not now</button>
            </div>
          )}
        </div>
      ))}
      <section ref={connectorRef} className="stack tight">
        <b className="strong small">{connector}</b>
        {/* The question comes first: the setup steps show once it's answered. */}
        <ChatbotsPanel key={botId} close={close} botId={botId} hideSetup={oneSide && signedIn && !answered} />
      </section>
    </div>
  );
}
