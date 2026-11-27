// The Code agent: the built-in agent at the top of the Agents list. It looks like any other Agent's chat, but it runs in
// this app (gui/src/code_agent.rs) and its job is starting Code Agents (the coding chats) for the user. Its message box
// keeps Code's controls: the folder, the model and the permission mode are for the Code Agents it starts, not for it.
// The Code Agents are on the right: pressing one opens its full chat, with a way back here.
import { memo, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as KE } from 'react';
import { act, type CodeAgentMsg } from '../api';
import { ComposerSettings } from '../Composer';
import { ModelChip } from '../ModelMenu';
import { CodeSection } from '../Sidebar';
import { Markdown } from '../Transcript';
import { WorkingOn } from '../WorkingOn';
import { ui, useView } from '../store';
import { Icon, IconButton } from '../widgets';
import { Avatar } from './Avatar';
import { SentAt } from './SentAt';
import './agent.css';

export const CODE_AGENT_COLOR = '#141414';

const when = (ms: number) => {
  const d = new Date(ms);
  const today = new Date().toDateString() === d.toDateString();
  return today ? d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }) : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
};

/** Opens a Code Agent's full chat (the Code view), which has a button back here. */
export function openCodeAgent(id: string) {
  act('openChat', { id });
  ui.set({ page: 'chat' });
}

/** The Code agent's row at the top of the Agents list. */
export function CodeAgentItem({ on }: { on: boolean }) {
  const { busy, status, messages } = useView('codeAgent');
  const sessions = useView('sessions');
  const working = sessions.filter((s) => s.running).length;
  const last = messages.filter((m) => m.role === 'agent' || m.role === 'started').at(-1);
  const sub = busy ? status || 'Thinking…' : working ? `${working} Code Agent${working === 1 ? '' : 's'} working` : last ? (last.role === 'started' ? `Started: ${last.text}` : last.text) : 'Starts Code Agents for you';
  return (
    <button type="button" className={`agent-item code-agent-item ${on ? 'on' : ''}`} onClick={() => ui.set({ page: 'code' })}>
      <Avatar shape="code" color={CODE_AGENT_COLOR} status={busy ? 'thinking' : working ? 'working' : 'idle'} kind="code" size={32} />
      <span className="agent-item-text">
        <span className="agent-item-name">Code</span>
        <span className={`agent-item-sub ${busy || working ? 'st-working' : ''}`}>{sub}</span>
      </span>
    </button>
  );
}

export function CodeAgentPage() {
  const { hasKey } = useView('cfg');
  // The first time it's opened, it says what it does and that the chats already here are its Code Agents.
  useEffect(() => act('codeWelcome'), []);
  return (
    <div className="main-body agent-page code-agent-page">
      <section className="agent-main">
        <CodeAgentChat signedIn={hasKey} />
      </section>
      <aside className="code-agents-panel">
        <CodeSection label="Code Agents" newLabel="New Code Agent" />
      </aside>
    </div>
  );
}

function CodeAgentChat({ signedIn }: { signedIn: boolean }) {
  const { messages, busy, status } = useView('codeAgent');
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const lastId = messages.at(-1)?.id ?? 0;
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lastId, busy]);
  return (
    <>
      <header className="agent-head">
        <Avatar shape="code" color={CODE_AGENT_COLOR} status={busy ? 'thinking' : 'idle'} kind="code" size={30} />
        <div className="agent-head-text">
          <b>Code</b>
          <span className="tiny dim ellipsis">
            Starts Code Agents for you
            {busy && status && <> · <span className="st-working">{status}</span></>}
          </span>
        </div>
        <span className="grow" />
        {busy && <IconButton icon="stop" tip="Stop what it’s doing" size={28} onClick={() => act('codeStop')} />}
      </header>
      <div
        className="agent-scroll"
        ref={scroller}
        onScroll={() => {
          const el = scroller.current;
          if (el) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
        }}
      >
        <div className="agent-msgs">
          {!messages.length && !busy && (
            <div className="agent-hero">
              <Avatar shape="code" color={CODE_AGENT_COLOR} status="idle" kind="code" size={96} />
              <b>Code</b>
              <span className="dim">Tell me what to build or fix. I’ll start Code Agents to do it, in the folder and with the model you pick below.</span>
            </div>
          )}
          {messages.map((m, i) => (
            <CodeMsg key={m.id} m={m} first={i === 0 || messages[i - 1].role !== m.role} />
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
      <CodeComposer signedIn={signedIn} busy={busy} />
    </>
  );
}

const CodeMsg = memo(function CodeMsg({ m, first }: { m: CodeAgentMsg; first: boolean }) {
  if (m.role === 'user') {
    return (
      <div className="agent-msg user">
        <div className="agent-bubble-user">{m.text}</div>
        <SentAt at={m.at} />
      </div>
    );
  }
  if (m.role === 'started' && m.chat) return <StartedWidget m={m} />;
  if (m.role === 'error' || m.role === 'notice') {
    return (
      <div className={`agent-activity ${m.role === 'error' ? 'code-agent-error' : ''}`}>
        <Icon name={m.role === 'error' ? 'close' : 'sparkle'} size={12} />
        <span>{m.text}</span>
        <SentAt at={m.at} />
      </div>
    );
  }
  return (
    <div className={`agent-msg agent ${first ? 'first' : ''}`}>
      <div className="agent-bubble"><Markdown text={m.text} /></div>
      <SentAt at={m.at} />
    </div>
  );
});

/** A Code Agent it started, in its chat: how it's doing, and a press opens it. */
function StartedWidget({ m }: { m: CodeAgentMsg }) {
  const s = useView('sessions').find((x) => x.id === m.chat);
  const state = !s ? 'Started' : s.waiting ? 'Needs your answer' : s.running ? 'Working…' : s.errored ? 'Ended with an error' : 'Done';
  return (
    <button type="button" className={`code-agent-widget ${s?.running ? 'working' : ''} ${s?.waiting ? 'asking' : ''}`} title={when(m.at)} onClick={() => openCodeAgent(m.chat!)}>
      <Icon name="code" size={14} />
      <span className="code-agent-widget-text">
        <span className="ellipsis">{s?.title || m.text}</span>
        <span className="tiny dim">Code Agent · {state}</span>
      </span>
      <Icon name="chevronRight" size={13} />
    </button>
  );
}

function CodeComposer({ signedIn, busy }: { signedIn: boolean; busy: boolean }) {
  const [text, setText] = useState('');
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => area.current?.focus(), []);
  const send = () => {
    const t = text.trim();
    if (!t || !signedIn) return;
    act('codeSend', { text: t });
    setText('');
    area.current?.focus();
  };
  const key = (e: KE<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    }
  };
  return (
    <div className="agent-composer code-agent-composer">
      {!signedIn && (
        <div className="code-agent-signin tiny">
          <span className="grow">Sign in to ScoutCode to talk to the Code agent. Your Code Agents on the right still work without it.</span>
          <button type="button" className="button compact" onClick={() => ui.set({ onboarding: true, onboardingMode: 'agent' })}>Sign in</button>
        </div>
      )}
      <div className="agent-composer-box">
        <textarea
          ref={area}
          rows={1}
          value={text}
          disabled={!signedIn}
          placeholder={busy ? 'Message Code (it hears you once it’s done with this step)…' : 'What should we build?'}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={key}
        />
        <button type="button" className="agent-send" aria-label="Send" disabled={!text.trim() || !signedIn} onClick={send}>
          <Icon name="arrowUp" size={16} />
        </button>
      </div>
      {/* For the Code Agents it starts, not for the Code agent itself. */}
      <div className="code-agent-controls" title="Where and how the Code Agents it starts work">
        <WorkingOn />
        <ModelChip />
        <ComposerSettings />
      </div>
    </div>
  );
}
