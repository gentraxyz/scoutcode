// Agent, the home page: up to 10 persistent agents that each work on their own Epsilon computer (which the user never
// sees: agents send files into the chat) and talk to them in a chat. Unlike Code, nothing here shows tool calls: the agent keeps the user posted with its messages, and
// asks with cards (approve or deny an action, sign in to an app, turn on a setting). See store.ts and, on the site,
// src/lib/agent-bot.ts.
import { memo, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as KE, type ReactNode } from 'react';
import { openExternal } from '../api';
import { ui, useUi, useView } from '../store';
import { Markdown } from '../Transcript';
import { ScoutLogo } from '../logo';
import { CodeAgentItem } from './CodeAgent';
import { SidebarFoot } from './ConnectApps';
import { Icon, IconButton, Modal, Spinner, Toggle } from '../widgets';
import { AGENT_COLORS, AGENT_SHAPES, Avatar } from './Avatar';
import {
  addSchedule,
  agentStore,
  answerCard,
  answerSecret,
  createAgent,
  deleteAgent,
  discardUnsent,
  fileUrl,
  isImage,
  loadOlder,
  loadMemory,
  loadSchedules,
  pauseSchedule,
  removeSchedule,
  retrySend,
  saveMemory,
  sendToAgent,
  setOpenRouterKey,
  setPrefs,
  STATUS_TEXT,
  stopAgent,
  updateAgent,
  useAgents,
  type Agent,
  type AgentFile,
  type Card,
  type CommandMode,
  type Look,
  type Memory,
  type Message,
  type NewSchedule,
  type Schedule,
} from './store';
import { SentAt } from './SentAt';
import './agent.css';

const when = (iso: string) => {
  const d = new Date(iso);
  const today = new Date().toDateString() === d.toDateString();
  return today ? d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }) : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
};

/** What it's up to, in a few words. Empty while it's only thinking: that shows as the typing dots, nothing more. */
const statusLine = (a: Agent) => {
  if (a.status === 'sleeping' && a.wakeAt) return `Sleeping until ${when(a.wakeAt)}`;
  if (a.status === 'thinking') return '';
  if ((a.status === 'working' || a.status === 'waiting') && a.activity) return a.activity;
  return STATUS_TEXT[a.status];
};

export function AgentPage() {
  const list = useAgents((s) => s.list);
  const error = useAgents((s) => s.error);
  const selected = useAgents((s) => s.selected);
  const editing = useAgents((s) => s.editing);
  const schedulesFor = useAgents((s) => s.schedulesFor);
  const agent = list?.find((a) => a.id === selected) ?? null;
  const scheduled = list?.find((a) => a.id === schedulesFor) ?? null;
  const { hasKey } = useView('cfg');
  // Agents live on the ScoutCode account: signed out (a guest), there's nothing to load, only a way in.
  if (!hasKey) {
    return (
      <div className="main-body agent-page">
        <section className="agent-main">
          <div className="agent-welcome">
            <Avatar shape="blob" color="#7c5cff" status="waiting" size={72} />
            <h1>Meet your agents</h1>
            <p className="dim">They work on their own computer in the cloud, even when this app is closed. Sign in to ScoutCode to make your first one.</p>
            <button type="button" className="agent-pill-btn" onClick={() => ui.set({ onboarding: true, onboardingMode: 'agent' })}>
              <Icon name="user" size={14} /> Sign in and meet your agent
            </button>
          </div>
        </section>
      </div>
    );
  }
  return (
    <div className="main-body agent-page">
      <section className="agent-main">
        {!list ? (
          <div className="agent-empty">{error ? <p className="dim">{error}</p> : <Spinner size={18} />}</div>
        ) : !agent ? (
          <Welcome />
        ) : (
          <AgentChat key={agent.id} agent={agent} />
        )}
      </section>
      {editing && <Editor id={editing} />}
      {scheduled && <Schedules agent={scheduled} />}
    </div>
  );
}

/** The left column on the Agent pages and in a Code Agent's chat: your agents, then the Code agent last (its Code Agents are on
 *  the right of its page). Signed out, there are no agents, so only the Code agent shows. At its foot: Connect to Apps, the
 *  account menu and the usage button (SidebarFoot), which the top bar doesn't show where this column is. */
export function AgentSidebar() {
  const { hasKey } = useView('cfg');
  return (
    <aside className="agent-list">
      {hasKey ? <AgentList /> : <div className="agent-block"><AgentsHead /><div className="agent-list-items"><CodeAgentNav /></div></div>}
      <SidebarFoot />
    </aside>
  );
}

/** The Agents header: the Scout logo, then the controls passed in. */
function AgentsHead({ children }: { children?: ReactNode }) {
  return (
    <div className="agent-list-head">
      <span className="agent-head-logo"><ScoutLogo size={20} /></span>
      {children}
    </div>
  );
}

function AgentList() {
  const list = useAgents((s) => s.list) ?? [];
  const max = useAgents((s) => s.max);
  const selected = useAgents((s) => s.selected);
  // The last-picked agent stays selected in the store, but it only looks picked while the Agent page is showing: on the
  // Code screen (which shares this list) no agent is highlighted.
  const onAgents = useUi((u) => u.page === 'agent');
  const highlighted = onAgents ? selected : null;
  return (
    <div className="agent-block">
      <AgentsHead>
        <span className="grow" />
        <IconButton icon="plus" tip={list.length >= max ? `You can have up to ${max} agents` : 'New agent'} size={26} disabled={list.length >= max} onClick={() => agentStore.set({ editing: 'new' })} />
      </AgentsHead>
      <div className="agent-list-items">
        {list.map((a) => (
          <button key={a.id} type="button" className={`agent-item ${a.id === highlighted ? 'on' : ''}`} onClick={() => { agentStore.set({ selected: a.id }); ui.set({ page: 'agent' }); }}>
            <Avatar shape={a.shape} color={a.color} status={a.status} kind={a.activityKind} size={32} />
            <span className="agent-item-text">
              <span className="agent-item-name">{a.name}</span>
              <span className={`agent-item-sub st-${a.status}`}>{(a.status === 'idle' || !statusLine(a)) && a.last ? a.last.text || 'Sent a file' : statusLine(a)}</span>
            </span>
            {a.unread > 0 && a.id !== selected && <span className="agent-unread">{a.unread > 9 ? '9+' : a.unread}</span>}
          </button>
        ))}
        <CodeAgentNav />
      </div>
    </div>
  );
}

function Welcome() {
  return (
    <div className="agent-welcome">
      <Avatar shape="circle" color="#e8e8e8" status="idle" size={72} />
      <h1>Meet your agents</h1>
      <p className="dim">Each one works on its own computer in the cloud, even when this app is closed, and sends you what it makes.</p>
      <button type="button" className="agent-pill-btn" onClick={() => agentStore.set({ editing: 'new' })}>
        <Icon name="plus" size={14} /> Create an agent
      </button>
    </div>
  );
}

function AgentChat({ agent }: { agent: Agent }) {
  const chat = useAgents((s) => s.chats[agent.id]);
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  /** Whether the view follows new messages: yes unless the user scrolled up to read. */
  const pinned = useRef(true);
  /** Scrolled up: the message at the top of the view and how far it sits from the view's top, so anything that changes
   *  size above it (older messages, images loading, the spinner) leaves it where it was. The browser's own scroll
   *  anchoring is off (agent.css): it skips at the very top, which is exactly where older messages come in. */
  const anchor = useRef<{ el: Element; top: number } | null>(null);
  const messages = chat?.messages ?? [];
  const busy = agent.status === 'thinking' || agent.status === 'working';
  const lastId = messages.at(-1)?.id ?? 0;
  const lastMine = messages.at(-1)?.role === 'user';
  const remember = () => {
    const el = scroller.current;
    const box = content.current;
    if (!el || !box) return;
    const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
    // Following stops as soon as the user scrolls up (see onWheel) and only resumes back at the very bottom: with an
    // 80px band, every render or resize during the first frames of a smooth scroll up snapped the view back down.
    pinned.current = pinned.current ? gap < 80 : gap < 24;
    anchor.current = null;
    if (pinned.current) return;
    // Binary search for the first message still showing at the top. Measuring them one by one from the top on every
    // scroll event made long chats choppy.
    const top = el.getBoundingClientRect().top;
    const kids = box.children;
    let lo = 0;
    let hi = kids.length - 1;
    let hit = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (kids[mid].getBoundingClientRect().bottom > top) {
        hit = mid;
        hi = mid - 1;
      } else lo = mid + 1;
    }
    if (hit >= 0) anchor.current = { el: kids[hit], top: kids[hit].getBoundingClientRect().top - top };
  };
  /** Puts the view back where it belongs after anything changed size: the bottom when following, else the anchor. */
  const settle = () => {
    const el = scroller.current;
    if (!el) return;
    if (pinned.current) {
      el.scrollTop = el.scrollHeight;
      return;
    }
    const a = anchor.current;
    if (!a || !a.el.isConnected) return;
    const shift = a.el.getBoundingClientRect().top - el.getBoundingClientRect().top - a.top;
    if (Math.abs(shift) >= 1) el.scrollTop += shift;
  };
  // Sending always brings the chat to the bottom.
  useLayoutEffect(() => {
    if (lastMine) pinned.current = true;
  }, [lastId, lastMine]);
  // After every render, before it's painted.
  useLayoutEffect(settle);
  // Images and files arriving, Markdown, the composer or window resizing: none of them re-render this, so watch sizes.
  useEffect(() => {
    const el = scroller.current;
    const box = content.current;
    if (!el || !box) return;
    const ro = new ResizeObserver(() => settle());
    ro.observe(box);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const older = () => {
    const el = scroller.current;
    if (el && el.scrollTop < 300 && chat?.more && !chat.older) void loadOlder(agent.id);
  };
  // A short chat that doesn't fill the view can't be scrolled up: fetch more straight away.
  useEffect(() => {
    const el = scroller.current;
    if (el && el.scrollHeight <= el.clientHeight + 300) older();
  }, [chat?.more, chat?.older, messages.length]);
  const line = statusLine(agent);
  return (
    <>
      <header className="agent-head">
        <Avatar shape={agent.shape} color={agent.color} status={agent.status} kind={agent.activityKind} size={30} title={line || undefined} />
        <div className="agent-head-text">
          <b>{agent.name}</b>
          <span className="tiny dim ellipsis">
            {agent.title || 'Agent'}
            {line && <> · <span className={`st-${agent.status}`}>{line}</span></>}
          </span>
        </div>
        <span className="grow" />
        {(busy || agent.status === 'sleeping' || agent.status === 'waiting') && <IconButton icon="stop" tip="Stop what it’s doing" size={28} onClick={() => void stopAgent(agent.id)} />}
        <span className="agent-sched-btn">
          <IconButton icon="clock" tip="Timers, alarms and recurring tasks" size={28} onClick={() => agentStore.set({ schedulesFor: agent.id })} />
          {(agent.schedules ?? 0) > 0 && <span className="agent-sched-count">{agent.schedules}</span>}
        </span>
        <IconButton icon="pencil" tip="Edit agent" size={28} onClick={() => agentStore.set({ editing: agent.id })} />
      </header>
      <div
        className="agent-scroll"
        ref={scroller}
        onScroll={() => {
          remember();
          older();
        }}
        onWheel={(e) => {
          if (e.deltaY < 0 && pinned.current) {
            pinned.current = false;
            remember();
          }
        }}
      >
        <div className="agent-msgs" ref={content}>
          {!chat && <div className="agent-empty"><Spinner size={16} /></div>}
          {chat?.older && <div className="agent-older"><Spinner size={14} /></div>}
          {chat?.error && !messages.length && <p className="tiny dim center">{chat.error}</p>}
          {chat && !chat.error && !messages.length && !busy && (
            <div className="agent-hero">
              <Avatar shape={agent.shape} color={agent.color} status={agent.status} kind={agent.activityKind} size={96} />
              <b>{agent.name}</b>
              <span className="dim">{agent.title || 'Your agent'}</span>
            </div>
          )}
          {messages.map((m, i) => (
            <MessageView key={m.id} m={m} agentId={agent.id} first={i === 0 || messages[i - 1].role !== m.role} />
          ))}
          {busy && (
            <div className="agent-msg agent typing">
              <div className="agent-typing">
                <span className="dots"><i /><i /><i /></span>
              </div>
            </div>
          )}
        </div>
      </div>
      <AgentComposer agent={agent} />
    </>
  );
}

/** Memoized: the chat re-renders on every status change, and parsing every message's Markdown again each time made scrolling choppy. */
const MessageView = memo(function MessageView({ m, agentId, first }: { m: Message; agentId: string; first: boolean }) {
  if (m.role === 'user') {
    const files = m.data && 'files' in m.data ? m.data.files ?? [] : [];
    const failed = m.local?.state === 'failed';
    return (
      <div className={`agent-msg user ${failed ? 'failed' : ''}`}>
        <div className="agent-bubble-user">
          {m.text}
          {files.length > 0 && <div className="agent-files">{files.map((f) => <span key={f.id} className="agent-file"><Icon name="paperclip" size={11} />{f.name}</span>)}</div>}
        </div>
        {!m.local && <SentAt at={m.created_at} />}
        {failed && (
          <div className="agent-unsent tiny">
            <span title={m.local?.error}>Didn’t send{m.local?.error ? `: ${m.local.error}` : ''}</span>
            <button type="button" onClick={() => retrySend(agentId, m)}>Retry</button>
            <button type="button" onClick={() => discardUnsent(agentId, m)}>Remove</button>
          </div>
        )}
      </div>
    );
  }
  if (m.role === 'card' && m.data && 'kind' in m.data && m.data.kind === 'activity') {
    return (
      <div id={`agent-msg-${m.id}`} className="agent-activity">
        <Icon name={m.data.tool === 'message_agent' || m.data.tool === 'agent_message' ? 'chat' : 'code'} size={12} />
        <span>{m.text}</span>
        <SentAt at={m.created_at} />
      </div>
    );
  }
  if (m.role === 'card') return <div id={`agent-msg-${m.id}`} className="agent-card-wrap"><CardView m={m} agentId={agentId} /></div>;
  const files = m.data && 'files' in m.data ? m.data.files ?? [] : [];
  const images = files.filter(isImage);
  const others = files.filter((f) => !isImage(f));
  // Stacked like a text thread: what it sent first, then its words under it, never side by side.
  return (
    <div className={`agent-msg agent ${first ? 'first' : ''}`}>
      {images.length > 0 && (
        <div className={`agent-gallery n${Math.min(images.length, 4)}`}>
          {images.map((f) => <SentImage key={f.id} agentId={agentId} f={f} />)}
        </div>
      )}
      {others.map((f) => <SentFile key={f.id} agentId={agentId} f={f} />)}
      {m.text && (
        <div className="agent-bubble">
          <Markdown text={m.text} />
        </div>
      )}
      <SentAt at={m.created_at} />
    </div>
  );
});

function useFileUrl(agentId: string, f: AgentFile, on = true) {
  const [url, setUrl] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!on) return;
    let live = true;
    fileUrl(agentId, f).then((u) => live && setUrl(u), (e) => live && setErr(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [agentId, f.id, on]);
  return { url, err };
}

/** Saves a file the agent sent, from its data: URL. */
const saveAs = (url: string, name: string) => {
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
};

type FileKind = 'image' | 'pdf' | 'video' | 'audio' | 'html' | 'markdown' | 'text' | 'other';
const TEXT_EXT = /\.(txt|log|csv|tsv|json|ya?ml|toml|xml|js|mjs|ts|tsx|jsx|py|rs|go|sh|css|sql|ini|cfg|conf|env)$/i;
const kindOf = (f: AgentFile): FileKind => {
  const t = f.type;
  if (isImage(f)) return 'image';
  if (t === 'application/pdf') return 'pdf';
  if (t.startsWith('video/')) return 'video';
  if (t.startsWith('audio/')) return 'audio';
  if (t === 'text/html') return 'html';
  if (t === 'text/markdown' || /\.(md|markdown)$/i.test(f.name)) return 'markdown';
  if (t.startsWith('text/') || /json|xml|javascript|yaml|toml|x-sh|x-python/.test(t) || TEXT_EXT.test(f.name)) return 'text';
  return 'other';
};
const KIND_LABEL: Record<FileKind, string> = { image: 'Image', pdf: 'PDF', video: 'Video', audio: 'Audio', html: 'Web page', markdown: 'Document', text: 'Text', other: 'File' };
const extOf = (f: AgentFile) => (f.name.includes('.') ? f.name.split('.').pop()!.slice(0, 4).toUpperCase() : 'FILE');

/** An image the agent sent: inline, opening in the viewer when clicked. */
function SentImage({ agentId, f }: { agentId: string; f: AgentFile }) {
  const { url, err } = useFileUrl(agentId, f);
  const [open, setOpen] = useState(false);
  if (err) return <SentFile agentId={agentId} f={f} />;
  return (
    <>
      <button type="button" className={`agent-img ${url ? '' : 'loading'}`} onClick={() => url && setOpen(true)} aria-label={`Open ${f.name}`}>
        {url ? <img src={url} alt={f.name} draggable={false} /> : <Spinner />}
      </button>
      {open && <FileViewer agentId={agentId} f={f} onClose={() => setOpen(false)} />}
    </>
  );
}

/** Any other file the agent sent: a card that opens it in the viewer, or saves it when it can't be shown. */
function SentFile({ agentId, f }: { agentId: string; f: AgentFile }) {
  const [open, setOpen] = useState(false);
  const [want, setWant] = useState(false);
  const { url, err } = useFileUrl(agentId, f, want);
  useEffect(() => {
    if (want && url) {
      saveAs(url, f.name);
      setWant(false);
    }
  }, [want, url]);
  const kind = kindOf(f);
  return (
    <>
      <div className={`agent-filecard k-${kind}`} title={err ?? f.name}>
        <button type="button" className="agent-filecard-main" onClick={() => (kind === 'other' ? setWant(true) : setOpen(true))}>
          <span className="agent-filecard-icon">{extOf(f)}</span>
          <span className="agent-filecard-text">
            <span className="agent-filecard-name">{f.name}</span>
            <span className="agent-filecard-sub">{err ? 'Unavailable' : `${KIND_LABEL[kind]} · ${size(f.size)}`}</span>
          </span>
        </button>
        <button type="button" className="agent-filecard-dl" aria-label={`Download ${f.name}`} title="Download" onClick={() => setWant(true)}>
          {want && !url && !err ? <Spinner /> : <Icon name="download" size={15} />}
        </button>
      </div>
      {open && <FileViewer agentId={agentId} f={f} onClose={() => setOpen(false)} />}
    </>
  );
}

/** A file, big, over the app: images, PDFs, video, audio, web pages and text. Esc or a click outside closes it. */
function FileViewer({ agentId, f, onClose }: { agentId: string; f: AgentFile; onClose: () => void }) {
  const { url, err } = useFileUrl(agentId, f);
  const kind = kindOf(f);
  const [blob, setBlob] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onClose]);
  useEffect(() => {
    if (!url || kind === 'image' || kind === 'other') return;
    let made: string | null = null;
    let live = true;
    void fetch(url)
      .then((r) => r.blob())
      .then(async (b) => {
        if (!live) return;
        if (kind === 'text' || kind === 'markdown') setText(await b.text());
        else setBlob((made = URL.createObjectURL(b)));
      });
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [url, kind]);
  const wait = <Spinner size={18} />;
  const body = err ? (
    <p className="agent-viewer-msg">{err}</p>
  ) : !url ? (
    wait
  ) : kind === 'image' ? (
    <img className="agent-viewer-img" src={url} alt={f.name} />
  ) : kind === 'pdf' || kind === 'html' ? (
    blob ? <iframe className="agent-viewer-frame" src={blob} title={f.name} sandbox={kind === 'html' ? '' : undefined} /> : wait
  ) : kind === 'video' ? (
    blob ? <video className="agent-viewer-media" src={blob} controls autoPlay /> : wait
  ) : kind === 'audio' ? (
    blob ? <audio src={blob} controls autoPlay /> : wait
  ) : kind === 'markdown' ? (
    text === null ? wait : <div className="agent-viewer-doc"><Markdown text={text} /></div>
  ) : kind === 'text' ? (
    text === null ? wait : <pre className="agent-viewer-text">{text.length > 400_000 ? `${text.slice(0, 400_000)}\n…` : text}</pre>
  ) : (
    <div className="agent-viewer-msg">
      <span className="agent-filecard-icon big">{extOf(f)}</span>
      <p>This file can’t be shown here. Download it to open it.</p>
    </div>
  );
  return (
    <div className="agent-viewer" role="dialog" aria-label={f.name} onClick={onClose}>
      <div className="agent-viewer-bar" onClick={(e) => e.stopPropagation()}>
        <span className="agent-viewer-name ellipsis">{f.name}</span>
        <span className="agent-viewer-size">{size(f.size)}</span>
        <span className="grow" />
        <button type="button" className="agent-viewer-btn" disabled={!url} onClick={() => url && saveAs(url, f.name)}>
          <Icon name="download" size={13} /> Download
        </button>
        <button type="button" className="agent-viewer-x" aria-label="Close" onClick={onClose}>
          <Icon name="close" size={14} />
        </button>
      </div>
      <div className={`agent-viewer-body k-${kind}`}>
        <div className="agent-viewer-stage" onClick={(e) => e.stopPropagation()}>{body}</div>
      </div>
    </div>
  );
}

function CardView({ m, agentId }: { m: Message; agentId: string }) {
  const card = (m.data ?? { kind: 'notice' }) as Card;
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setErr(null);
    try {
      await f();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const answered = 'answer' in card ? card.answer : undefined;
  if (card.kind === 'permission') return card.setting === 'localAccess' ? <PermissionCard setting={card.setting} text={m.text} /> : null;
  if (card.kind === 'alarm') return <AlarmCard m={m} card={card} />;
  if (card.kind === 'secret') return <SecretCard m={m} card={card} agentId={agentId} />;
  if (card.kind === 'command' || (card.kind === 'approval' && card.risky)) {
    const isCmd = card.kind === 'command';
    const where = isCmd && card.where === 'pc' ? 'this PC' : 'its computer';
    return (
      <div className={`agent-card kind-${isCmd ? 'local' : 'approval'} ${card.risky ? 'risky' : ''} ${answered ? 'answered' : ''}`}>
        <div className="agent-card-head">
          <Icon name={card.risky ? 'shield' : 'terminal'} size={14} />
          <span>{card.risky ? 'Needs your OK' : m.text}</span>
        </div>
        {card.risky && card.reason && <div className="agent-card-reason">{card.reason}</div>}
        {isCmd ? <pre className="agent-card-cmd">{card.command}</pre> : <div className="agent-card-detail">{card.action}{card.details ? `\n${card.details}` : ''}</div>}
        {isCmd && !answered && <div className="tiny dim">Runs on {where}.{card.setup ? ' Always allow lets commands run without asking; you’ll still be asked about ones it marks risky.' : ''}</div>}
        {answered ? (
          <div className={`agent-card-answer ${answered}`}>{answered === 'approve' ? (card.kind === 'command' && card.always ? 'Allowed, and commands run without asking from now on' : 'Approved') : 'Denied'}</div>
        ) : (
          <div className="agent-card-actions">
            <button type="button" className="button primary" disabled={busy} onClick={() => run(() => answerCard(agentId, m.id, 'approve'))}>{busy ? <Spinner /> : isCmd && !card.risky ? (card.setup ? 'Allow once' : 'Run it') : 'Approve'}</button>
            {isCmd && !card.risky && <button type="button" className="button" disabled={busy} onClick={() => run(() => answerCard(agentId, m.id, 'always'))}>Always allow</button>}
            <button type="button" className="button" disabled={busy} onClick={() => run(() => answerCard(agentId, m.id, 'deny'))}>Deny</button>
          </div>
        )}
        {err && <p className="tiny agent-err">{err}</p>}
      </div>
    );
  }
  return (
    <div className={`agent-card kind-${card.kind} ${answered ? 'answered' : ''}`}>
      <div className="agent-card-head">
        <Icon name={card.kind === 'approval' ? 'shield' : card.kind === 'local' ? 'terminal' : card.kind === 'auth' ? 'key' : card.kind === 'open' ? 'globe' : 'bell'} size={14} />
        <span>{m.text}</span>
      </div>
      {card.kind === 'approval' && card.details && <div className="agent-card-detail">{card.details}</div>}
      {card.kind === 'local' && <pre className="agent-card-cmd">{card.command}</pre>}
      {(card.kind === 'approval' || card.kind === 'local') &&
        (answered ? (
          <div className={`agent-card-answer ${answered}`}>{answered === 'approve' ? (card.kind === 'local' ? 'Ran on your PC' : 'Approved') : 'Denied'}</div>
        ) : (
          <div className="agent-card-actions">
            <button type="button" className="button primary" disabled={busy} onClick={() => run(() => answerCard(agentId, m.id, 'approve'))}>{busy ? <Spinner /> : card.kind === 'local' ? 'Run it' : 'Approve'}</button>
            <button type="button" className="button" disabled={busy} onClick={() => run(() => answerCard(agentId, m.id, 'deny'))}>Deny</button>
          </div>
        ))}
      {card.kind === 'auth' &&
        (answered ? (
          <div className="agent-card-answer approve">Signed in to {card.name}</div>
        ) : (
          <div className="agent-card-actions">
            <button type="button" className="button primary" onClick={() => openExternal(card.url)}>Sign in to {card.name}</button>
          </div>
        ))}
      {card.kind === 'open' && (
        <div className="agent-card-actions">
          <button type="button" className="button primary" onClick={() => openExternal(card.url)}>Open in your browser</button>
          <span className="tiny dim ellipsis">{card.url}</span>
        </div>
      )}
      {err && <p className="tiny agent-err">{err}</p>}
    </div>
  );
}

/** What an app needs to connect, typed in here: the values go to the site's copy of the connection, never the chat. */
function SecretCard({ m, card, agentId }: { m: Message; card: Extract<Card, { kind: 'secret' }>; agentId: string }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ready = card.fields.every((f) => f.optional || values[f.key]?.trim());
  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      await answerSecret(agentId, m.id, values);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={`agent-card kind-auth ${card.answer ? 'answered' : ''}`}>
      <div className="agent-card-head"><Icon name="key" size={14} /><span>{m.text}</span></div>
      {card.answer ? (
        <div className="agent-card-answer approve">Saved. {card.name} is connected.</div>
      ) : (
        <>
          {card.fields.map((f) => (
            <label key={f.key} className="agent-secret-field">
              <span>{f.key === 'token' ? 'Access token' : f.label}{f.optional ? <i className="dim"> (optional)</i> : null}</span>
              {f.hint && <span className="tiny dim">{f.hint}</span>}
              <input type="password" autoComplete="off" spellCheck={false} value={values[f.key] ?? ''} onChange={(e) => setValues({ ...values, [f.key]: e.target.value })} onKeyDown={(e) => e.key === 'Enter' && ready && !busy && void save()} />
            </label>
          ))}
          <div className="agent-card-actions">
            <button type="button" className="button primary" disabled={busy || !ready} onClick={() => void save()}>{busy ? <Spinner /> : 'Save'}</button>
            <span className="tiny dim">Kept with this connection on ScoutCode’s servers. Your agent never sees it.</span>
          </div>
        </>
      )}
      {err && <p className="tiny agent-err">{err}</p>}
    </div>
  );
}

/** Alarm cards that already rang on this PC, so scrolling back or reloading doesn't ring them again. */
const rung = new Set<number>();
const RING_FOR_MS = 2 * 60_000;

/** A short three-beep chime, from the app itself (no sound file). */
function ring() {
  try {
    const Ctx = window.AudioContext ?? (window as any).webkitAudioContext;
    const ac: AudioContext = new Ctx();
    for (let i = 0; i < 3; i++) {
      const o = ac.createOscillator();
      const g = ac.createGain();
      const t = ac.currentTime + i * 0.32;
      o.type = 'sine';
      o.frequency.value = i === 2 ? 1046 : 880;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.25, t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, t + 0.26);
      o.connect(g).connect(ac.destination);
      o.start(t);
      o.stop(t + 0.28);
    }
    setTimeout(() => void ac.close(), 1500);
  } catch {
    /* no audio */
  }
}

/** A timer, alarm or recurring task going off. Rings when it arrives while the app is open. */
function AlarmCard({ m, card }: { m: Message; card: Extract<Card, { kind: 'alarm' }> }) {
  useEffect(() => {
    if (rung.has(m.id) || Date.now() - Date.parse(m.created_at) > RING_FOR_MS) return;
    rung.add(m.id);
    ring();
  }, [m.id]);
  const what = card.type === 'timer' ? 'Timer' : card.type === 'alarm' ? 'Alarm' : 'Scheduled';
  return (
    <div className="agent-card kind-alarm">
      <div className="agent-card-head">
        <Icon name={card.type === 'repeat' ? 'repeat' : 'bell'} size={14} />
        <span><b>{what}:</b> {m.text}</span>
        <span className="grow" />
        <span className="tiny dim">{when(m.created_at)}</span>
      </div>
    </div>
  );
}

/** The popup for a tool the user hasn't allowed yet: a card in the chat with the switch right in it. */
function PermissionCard({ setting, text }: { setting: 'localAccess'; text: string }) {
  const on = useAgents((s) => s.prefs[setting]);
  const what = 'Local access';
  const why = 'Lets your agents run commands on this PC and open pages in your browser. Commands ask you first unless you choose Auto-accept in Settings.';
  return (
    <div className="agent-card kind-permission">
      <div className="agent-card-head"><Icon name="lock" size={14} /><span>{text}</span></div>
      <div className="agent-card-row">
        <div>
          <b>{what}</b>
          <div className="tiny dim">{why}</div>
        </div>
        <Toggle on={on} label={what} onChange={(v) => setPrefs({ [setting]: v })} />
      </div>
      {on && <div className="agent-card-answer approve">Turned on. Tell your agent to try again.</div>}
    </div>
  );
}

function AgentComposer({ agent }: { agent: Agent }) {
  const [text, setText] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const input = useRef<HTMLInputElement>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => area.current?.focus(), [agent.id]);
  // Shows in the chat at once; a message that didn't make it is marked there, with Retry.
  const send = () => {
    const t = text.trim();
    if (!t && !files.length) return;
    sendToAgent(agent.id, t, files);
    setText('');
    setFiles([]);
    area.current?.focus();
  };
  const key = (e: KE<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };
  return (
    <div className="agent-composer">
      {files.length > 0 && (
        <div className="agent-files">
          {files.map((f, i) => (
            <span key={`${f.name}${i}`} className="agent-file">
              <Icon name="paperclip" size={11} />{f.name}
              <button type="button" aria-label={`Remove ${f.name}`} onClick={() => setFiles(files.filter((_, j) => j !== i))}><Icon name="close" size={10} /></button>
            </span>
          ))}
        </div>
      )}
      <div className="agent-composer-box">
        <IconButton icon="plus" tip="Attach files" size={30} onClick={() => input.current?.click()} />
        <input ref={input} type="file" multiple hidden onChange={(e) => { setFiles([...files, ...Array.from(e.target.files ?? [])].slice(0, 10)); e.target.value = ''; }} />
        <textarea
          ref={area}
          rows={1}
          value={text}
          placeholder={`Message ${agent.name}…`}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={key}
          onPaste={(e) => {
            const pasted = Array.from(e.clipboardData.files);
            if (pasted.length) {
              e.preventDefault();
              setFiles([...files, ...pasted].slice(0, 10));
            }
          }}
        />
        <button type="button" className="agent-send" aria-label="Send" disabled={!text.trim() && !files.length} onClick={send}>
          <Icon name="arrowUp" size={16} />
        </button>
      </div>
    </div>
  );
}

const size = (n: number) => (n < 1024 ? `${n} B` : n < 1 << 20 ? `${Math.round(n / 1024)} KB` : `${(n / (1 << 20)).toFixed(1)} MB`);

function Editor({ id }: { id: string }) {
  const existing = useAgents((s) => (id === 'new' ? null : s.list?.find((a) => a.id === id) ?? null));
  const [look, setLook] = useState<Look>(existing ? { name: existing.name, title: existing.title, shape: existing.shape, color: existing.color, instructions: existing.instructions } : { name: '', title: '', shape: AGENT_SHAPES[Math.floor(Math.random() * AGENT_SHAPES.length)], color: AGENT_COLORS[Math.floor(Math.random() * 10)], instructions: '' });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const close = () => agentStore.set({ editing: null });
  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      if (existing) await updateAgent(existing.id, look);
      else await createAgent(look);
      close();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const set = (p: Partial<Look>) => setLook({ ...look, ...p });
  return (
    <Modal onClose={close} className="agent-editor">
      <div className="agent-editor-preview">
        <Avatar shape={look.shape} color={look.color} status="idle" size={96} />
        <b>{look.name || 'Your agent'}</b>
        <span className="tiny dim">{look.title || 'Agent'}</span>
      </div>
      <div className="agent-editor-form">
        <label>
          <span>Name</span>
          <input autoFocus value={look.name} maxLength={40} placeholder="e.g. Nova" onChange={(e) => set({ name: e.target.value })} />
        </label>
        <label>
          <span>Title</span>
          <input value={look.title} maxLength={80} placeholder="e.g. Research assistant" onChange={(e) => set({ title: e.target.value })} />
        </label>
        <div className="agent-field">
          <span>Shape</span>
          <div className="agent-shapes">
            {AGENT_SHAPES.map((s) => (
              <button key={s} type="button" className={look.shape === s ? 'on' : ''} title={s} aria-label={s} onClick={() => set({ shape: s })}>
                <Avatar shape={s} color={look.color} status="idle" size={30} />
              </button>
            ))}
          </div>
        </div>
        <div className="agent-field">
          <span>Color</span>
          <div className="agent-colors">
            {AGENT_COLORS.map((c) => (
              <button key={c} type="button" className={look.color === c ? 'on' : ''} style={{ background: c }} aria-label={c} onClick={() => set({ color: c })} />
            ))}
            <label className="agent-color-custom" title="Custom color">
              <input type="color" value={look.color} onChange={(e) => set({ color: e.target.value })} />
              <Icon name="palette" size={14} />
            </label>
          </div>
        </div>
        <label>
          <span>Instructions <i className="dim">(optional)</i></span>
          <textarea rows={4} value={look.instructions} maxLength={8000} placeholder="What it’s for, how it should behave, what it should know about you…" onChange={(e) => set({ instructions: e.target.value })} />
        </label>
        {err && <p className="tiny agent-err">{err}</p>}
        <div className="agent-editor-actions">
          {existing &&
            (confirmDelete ? (
              <button type="button" className="button danger" disabled={busy} onClick={async () => { setBusy(true); try { await deleteAgent(existing.id); close(); } catch (e) { setErr(String(e)); setBusy(false); } }}>
                Delete {existing.name} and its chat
              </button>
            ) : (
              <button type="button" className="button" onClick={() => setConfirmDelete(true)}><Icon name="trash" size={13} /> Delete</button>
            ))}
          <span className="grow" />
          <button type="button" className="button" onClick={close}>Cancel</button>
          <button type="button" className="button primary" disabled={busy || !look.name.trim()} onClick={() => void save()}>{busy ? <Spinner /> : existing ? 'Save' : 'Create'}</button>
        </div>
      </div>
    </Modal>
  );
}

/** Agent settings, on the Agents page of Settings. */
export function AgentSettings() {
  const prefs = useAgents((s) => s.prefs);
  const openrouter = useAgents((s) => s.openrouter);
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const saveKey = async (k: string | null) => {
    setBusy(true);
    setErr(null);
    try {
      await setOpenRouterKey(k);
      setKey('');
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div className="agent-setting">
        <div>
          <b>Local access</b>
          <div className="tiny dim">Your agents can run commands on this PC and open pages in your browser, while this app is open.</div>
        </div>
        <Toggle on={prefs.localAccess} label="Local access" onChange={(v) => setPrefs({ localAccess: v })} />
      </div>
      <ModeSetting
        title="Commands on its computer"
        about="Your agents run commands on their own cloud computer to get work done."
        value={prefs.commandMode ?? null}
        onChange={(v) => setPrefs({ commandMode: v })}
      />
      {prefs.localAccess && (
        <ModeSetting title="Commands on this PC" about="Commands your agents run on this PC through the app." value={prefs.pcMode ?? null} onChange={(v) => setPrefs({ pcMode: v })} />
      )}
      <div className="agent-setting column">
        <div>
          <b>OpenRouter key</b>
          <div className="tiny dim">{openrouter ? 'Your agents use your OpenRouter key.' : 'Optional: use your own OpenRouter credit instead of your Scout credit.'}</div>
        </div>
        <div className="agent-key">
          <input type="password" value={key} placeholder={openrouter ? '••••••••' : 'sk-or-…'} onChange={(e) => setKey(e.target.value)} />
          <button type="button" className="button" disabled={busy || !key.trim()} onClick={() => void saveKey(key.trim())}>Save</button>
          {openrouter && <button type="button" className="button" disabled={busy} onClick={() => void saveKey(null)}>Remove</button>}
        </div>
        {err && <p className="tiny agent-err">{err}</p>}
      </div>
      <MemorySetting />
      <p className="tiny dim">Each agent has its own Epsilon computer, which uses your Epsilon credits only while it runs.</p>
    </>
  );
}

const MODES: { v: CommandMode; label: string; hint: string }[] = [
  { v: 'ask', label: 'Ask each time', hint: 'You approve every command before it runs.' },
  { v: 'auto', label: 'Auto-accept', hint: 'Commands run right away. You’re only asked about ones your agent marks risky, with its reason.' },
];

/** How commands get approved: each one, or only the ones the agent marks risky. Not chosen yet: the first one asks. */
function ModeSetting({ title, about, value, onChange }: { title: string; about: string; value: CommandMode; onChange: (v: CommandMode) => void }) {
  const cur = MODES.find((m) => m.v === value);
  return (
    <div className="agent-setting column">
      <div>
        <b>{title}</b>
        <div className="tiny dim">{about} {cur ? cur.hint : 'Not chosen yet: the first command asks you.'}</div>
      </div>
      <div className="agent-modes" role="radiogroup" aria-label={title}>
        {MODES.map((m) => (
          <button key={m.v} type="button" role="radio" aria-checked={value === m.v} className={value === m.v ? 'on' : ''} onClick={() => void onChange(m.v)}>
            {m.label}
          </button>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- timers, alarms and recurring tasks

const DAY_KEYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const pad2 = (n: number) => String(n).padStart(2, '0');
/** A date input's value for a moment, in this PC's time. */
const localInput = (d: Date) => `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}:${pad2(d.getMinutes())}`;

/** When it goes off next, as the user reads it: "in 5 min", "today 6:00 PM", "Fri 9:00 AM". */
function nextText(iso: string | null): string {
  if (!iso) return 'Done';
  const t = Date.parse(iso);
  const min = Math.round((t - Date.now()) / 60_000);
  if (min < 1) return 'Any moment';
  if (min < 60) return `In ${min} min`;
  const d = new Date(t);
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (d.toDateString() === new Date().toDateString()) return `Today ${time}`;
  if (t - Date.now() < 6 * 86_400_000) return `${d.toLocaleDateString([], { weekday: 'short' })} ${time}`;
  return `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${time}`;
}

/** An agent's timers, alarms and recurring tasks: what's set, pause or cancel any, and set new ones by hand. */
function Schedules({ agent }: { agent: Agent }) {
  const [list, setList] = useState<Schedule[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const close = () => agentStore.set({ schedulesFor: null });
  const load = async () => {
    try {
      setList(await loadSchedules(agent.id));
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };
  // Again whenever the agent changes (it may set or cancel one as you watch), and each minute for the countdowns.
  useEffect(() => void load(), [agent.id, agent.schedules]);
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);
  const act = async (f: () => Promise<unknown>) => {
    setErr(null);
    try {
      await f();
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <Modal onClose={close} className="agent-schedules">
      <h2>{agent.name}’s timers and schedules</h2>
      <p className="tiny dim">Ask {agent.name} in the chat (“remind me at 5 to call Sam”, “every weekday at 9, send me the news”), or set one here.</p>
      {!list ? (
        err ? <p className="tiny agent-err">{err}</p> : <Spinner size={16} />
      ) : list.length === 0 && !adding ? (
        <p className="dim agent-sched-empty">Nothing scheduled.</p>
      ) : (
        <ul className="agent-sched-list">
          {list.map((s) => (
            <li key={s.id} className={s.paused ? 'paused' : ''}>
              <Icon name={s.kind === 'repeat' ? 'repeat' : s.kind === 'timer' ? 'clock' : 'bell'} size={15} />
              <div className="agent-sched-text">
                <b>{s.label}</b>
                <span className="tiny dim">
                  {s.summary} · {s.paused ? 'Paused' : nextText(s.nextAt)}
                  {s.ring ? ' · rings' : ''}
                </span>
                {s.task && <span className="tiny agent-sched-task">{s.task}</span>}
              </div>
              {s.kind === 'repeat' && <IconButton icon={s.paused ? 'play' : 'pause'} tip={s.paused ? 'Resume' : 'Pause'} size={26} onClick={() => void act(() => pauseSchedule(agent.id, s.id, !s.paused))} />}
              <IconButton icon="trash" tip="Cancel" size={26} onClick={() => void act(() => removeSchedule(agent.id, s.id))} />
            </li>
          ))}
        </ul>
      )}
      {list && err && <p className="tiny agent-err">{err}</p>}
      {adding ? (
        <NewScheduleForm
          onCancel={() => setAdding(false)}
          onSave={async (s) => {
            await addSchedule(agent.id, s);
            setAdding(false);
            await load();
          }}
        />
      ) : (
        <div className="agent-editor-actions">
          <span className="grow" />
          <button type="button" className="button" onClick={close}>Close</button>
          <button type="button" className="button primary" onClick={() => setAdding(true)}><Icon name="plus" size={13} /> New</button>
        </div>
      )}
    </Modal>
  );
}

function NewScheduleForm({ onSave, onCancel }: { onSave: (s: NewSchedule) => Promise<void>; onCancel: () => void }) {
  const [type, setType] = useState<NewSchedule['type']>('alarm');
  const [label, setLabel] = useState('');
  const [task, setTask] = useState('');
  const [minutes, setMinutes] = useState(10);
  const [at, setAt] = useState(() => localInput(new Date(Date.now() + 3_600_000)));
  const [every, setEvery] = useState<NonNullable<NewSchedule['every']>>('days');
  const [interval, setInterval_] = useState(1);
  const [time, setTime] = useState('09:00');
  const [days, setDays] = useState<string[]>(['mon', 'tue', 'wed', 'thu', 'fri']);
  const [dom, setDom] = useState(1);
  const [ring, setRing] = useState(true);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const save = async () => {
    setBusy(true);
    setErr(null);
    const s: NewSchedule = { type, label: label.trim(), task: task.trim() || undefined, ring: type === 'repeat' ? ring : true };
    if (type === 'timer') s.minutes = minutes;
    if (type === 'alarm') s.at = at.replace('T', ' ');
    if (type === 'repeat') Object.assign(s, { every, interval, time: every === 'minutes' || every === 'hours' ? undefined : time, days: every === 'weeks' ? days : undefined, day_of_month: every === 'months' ? dom : undefined });
    try {
      await onSave(s);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };
  const ok = label.trim() && (type !== 'repeat' || ring || task.trim()) && (type !== 'repeat' || every !== 'weeks' || days.length);
  return (
    <div className="agent-sched-form">
      <div className="agent-modes" role="radiogroup" aria-label="Kind">
        {(['timer', 'alarm', 'repeat'] as const).map((k) => (
          <button key={k} type="button" role="radio" aria-checked={type === k} className={type === k ? 'on' : ''} onClick={() => setType(k)}>
            {k === 'timer' ? 'Timer' : k === 'alarm' ? 'Alarm' : 'Recurring'}
          </button>
        ))}
      </div>
      <label>
        <span>Label</span>
        <input autoFocus value={label} maxLength={80} placeholder={type === 'repeat' ? 'e.g. Morning news brief' : 'e.g. Call Sam'} onChange={(e) => setLabel(e.target.value)} />
      </label>
      {type === 'timer' && (
        <label>
          <span>Minutes</span>
          <input type="number" min={1} max={86400} value={minutes} onChange={(e) => setMinutes(Number(e.target.value))} />
        </label>
      )}
      {type === 'alarm' && (
        <label>
          <span>When</span>
          <input type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />
        </label>
      )}
      {type === 'repeat' && (
        <>
          <div className="agent-sched-row">
            <label>
              <span>Every</span>
              <input type="number" min={every === 'minutes' ? 5 : 1} max={1000} value={interval} onChange={(e) => setInterval_(Number(e.target.value))} />
            </label>
            <label>
              <span>&nbsp;</span>
              <select value={every} onChange={(e) => setEvery(e.target.value as typeof every)}>
                <option value="minutes">minutes</option>
                <option value="hours">hours</option>
                <option value="days">days</option>
                <option value="weeks">weeks</option>
                <option value="months">months</option>
              </select>
            </label>
            {every !== 'minutes' && every !== 'hours' && (
              <label>
                <span>At</span>
                <input type="time" value={time} onChange={(e) => setTime(e.target.value)} />
              </label>
            )}
            {every === 'months' && (
              <label>
                <span>Day</span>
                <input type="number" min={1} max={31} value={dom} onChange={(e) => setDom(Number(e.target.value))} />
              </label>
            )}
          </div>
          {every === 'weeks' && (
            <div className="agent-days">
              {DAY_KEYS.map((d) => (
                <button key={d} type="button" className={days.includes(d) ? 'on' : ''} onClick={() => setDays(days.includes(d) ? days.filter((x) => x !== d) : [...days, d])}>
                  {d[0].toUpperCase() + d.slice(1, 2)}
                </button>
              ))}
            </div>
          )}
          <div className="agent-setting">
            <div>
              <b>Ring</b>
              <div className="tiny dim">Ring in the chat each time, as well as running the task.</div>
            </div>
            <Toggle on={ring} label="Ring" onChange={setRing} />
          </div>
        </>
      )}
      <label>
        <span>Task for your agent <i className="dim">{type === 'repeat' && !ring ? '' : '(optional)'}</i></span>
        <textarea rows={2} value={task} maxLength={2000} placeholder={type === 'repeat' ? 'What it does each time, e.g. “Send me the top 5 tech headlines”' : 'Something to do when it goes off, or leave empty to just ring'} onChange={(e) => setTask(e.target.value)} />
      </label>
      {err && <p className="tiny agent-err">{err}</p>}
      <div className="agent-editor-actions">
        <span className="grow" />
        <button type="button" className="button" onClick={onCancel}>Cancel</button>
        <button type="button" className="button primary" disabled={busy || !ok} onClick={() => void save()}>{busy ? <Spinner /> : 'Set'}</button>
      </div>
    </div>
  );
}

/** What the agents know about the user: one memory for all of them, kept on ScoutCode's servers. Editable here. */
function MemorySetting() {
  const [mem, setMem] = useState<Memory | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const load = async () => {
    setErr(null);
    try {
      const m = await loadMemory();
      setMem(m);
      setDraft(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };
  useEffect(() => void load(), []);
  const save = async (text: string) => {
    if (!mem) return;
    setBusy(true);
    setErr(null);
    try {
      setMem(await saveMemory(mem.text, text));
      setDraft(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const editing = draft !== null;
  return (
    <div className="agent-setting column">
      <div className="agent-memory-head">
        <div>
          <b>Memory</b>
          <div className="tiny dim">What your agents know about you. They share it, keep it up to date as you talk, and it’s kept on ScoutCode’s servers.</div>
        </div>
        {mem && <span className="tiny dim agent-memory-used">{Math.min(100, Math.round((mem.used / mem.limit) * 100))}% full</span>}
      </div>
      {!mem ? (
        err ? <p className="tiny agent-err">{err}</p> : <Spinner size={14} />
      ) : (
        <>
          <textarea
            className="agent-memory-text"
            rows={8}
            spellCheck={false}
            value={draft ?? mem.text}
            placeholder="Nothing yet. Your agents add to this as they get to know you, or write here yourself (## sections, one fact per line)."
            onChange={(e) => setDraft(e.target.value)}
          />
          <div className="agent-key">
            <span className="grow tiny dim">{mem.updatedAt ? `Last changed ${when(mem.updatedAt)}` : ''}</span>
            {editing && <button type="button" className="button" disabled={busy} onClick={() => setDraft(null)}>Cancel</button>}
            {editing ? (
              <button type="button" className="button primary" disabled={busy} onClick={() => void save(draft)}>{busy ? <Spinner /> : 'Save'}</button>
            ) : (
              <>
                <button type="button" className="button" disabled={busy} onClick={() => void load()}>Reload</button>
                {mem.text && <button type="button" className="button" disabled={busy} onClick={() => void save('')}>Clear</button>}
              </>
            )}
          </div>
          {err && <p className="tiny agent-err">{err}</p>}
          {mem.log.length > 0 && (
            <details className="agent-memory-log">
              <summary className="tiny dim">Recent changes</summary>
              <ul>
                {mem.log.slice(0, 12).map((l, i) => (
                  <li key={i} className="tiny">
                    <span className="dim">{when(l.at)} · {l.by}</span> {l.verb}: {l.text}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
    </div>
  );
}

/** The Code agent's row: picked on its page and in its Code Agents' chats. */
function CodeAgentNav() {
  const on = useUi((u) => u.page === 'code' || u.page === 'chat');
  return <CodeAgentItem on={on} />;
}
