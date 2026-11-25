// Onboarding's first meeting with an agent: it asks, in a chat, what it should be called, what it's for, how it should
// look and what to keep in mind, and the answers make the agent. Its side of the chat is scripted here, so none of it
// costs a model call: the real agent only starts once it's made.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { Icon, Spinner } from '../widgets';
import { AGENT_COLORS, AGENT_SHAPES, Avatar } from './Avatar';
import { createAgent, useAgents, type Look, type Shape, type Status } from './store';
import './agent.css';

type Ask = 'loading' | 'again' | 'name' | 'title' | 'shape' | 'color' | 'notes' | 'saving' | 'retry' | 'done';
type Line = { id: number; from: 'agent' | 'user'; body: ReactNode };

const NAMES = ['Nova', 'Atlas', 'Pixel', 'Juno', 'Echo'];
const TITLES = ['Research assistant', 'Personal assistant', 'Writing partner', 'Planner', 'Coding buddy'];

const reduced = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
/** How long it "types" a line: a beat, a little longer for longer lines. */
const typingFor = (text: string) => (reduced() ? 150 : Math.min(1500, 480 + text.length * 14));

const list = (xs: string[]) => (xs.length < 2 ? xs.join('') : `${xs.slice(0, -1).join(', ')} and ${xs.at(-1)}`);

/** `onReady` fires once there's an agent to go on with (made here, or one they had already), or they say they're done. */
export function MeetAgent({ onReady }: { onReady: () => void }) {
  const agents = useAgents((s) => s.list);
  const max = useAgents((s) => s.max);
  const loadError = useAgents((s) => s.error);
  const [look, setLook] = useState<Look>(() => ({
    name: '',
    title: '',
    shape: AGENT_SHAPES[Math.floor(Math.random() * AGENT_SHAPES.length)],
    color: AGENT_COLORS[Math.floor(Math.random() * 10)],
    instructions: '',
  }));
  const [lines, setLines] = useState<Line[]>([]);
  const [typing, setTyping] = useState(false);
  const [ask, setAsk] = useState<Ask>('loading');
  const [draft, setDraft] = useState('');
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const nextId = useRef(1);
  const scroller = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const started = useRef(false);

  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length, typing, ask]);
  useEffect(() => {
    if (ask === 'name' || ask === 'title' || ask === 'notes') input.current?.focus();
  }, [ask]);

  const push = (from: Line['from'], body: ReactNode) => setLines((ls) => [...ls, { id: nextId.current++, from, body }]);

  /** The agent says each line in turn, "typing" before each, then waits on `then`. */
  const say = (texts: string[], then: Ask, after?: () => void) => {
    setAsk('loading');
    let at = 0;
    texts.forEach((text) => {
      timers.current.push(setTimeout(() => setTyping(true), at));
      at += typingFor(text);
      timers.current.push(setTimeout(() => { setTyping(false); push('agent', text); }, at));
      at += reduced() ? 0 : 220;
    });
    timers.current.push(setTimeout(() => { setAsk(then); after?.(); }, at));
  };

  // Starts once the agents list is in: a new account meets its first agent, one with agents already is asked first.
  useEffect(() => {
    if (started.current || (agents === null && !loadError)) return;
    started.current = true;
    // The list couldn't be fetched: meet a new one anyway, and making it says what's wrong if it still is.
    if (!agents) greet();
    else if (agents.length >= max) {
      say([`Hi again! You already have ${agents.length} agents, the most you can have.`, 'They’re waiting for you on the Agent page.'], 'done', onReady);
    } else if (agents.length > 0) {
      say([`Welcome back! You already have ${list(agents.map((a) => a.name))}.`, 'Want to make another agent?'], 'again');
    } else {
      greet();
    }
  }, [agents, loadError]); // eslint-disable-line react-hooks/exhaustive-deps

  const greet = () => say([
    'Hi! I’m your new agent.',
    'I work on my own computer in the cloud, even when Scout is closed, and send you what I make right here.',
    'First things first: what should I be called?',
  ], 'name');

  const answerName = (raw: string) => {
    const name = raw.trim().slice(0, 40);
    if (!name) return;
    setLook((l) => ({ ...l, name }));
    push('user', name);
    setDraft('');
    say([`${name}. I like it!`, 'What should I mostly help you with?'], 'title');
  };

  const answerTitle = (raw: string) => {
    const title = raw.trim().slice(0, 80);
    if (!title) return;
    setLook((l) => ({ ...l, title }));
    push('user', title);
    setDraft('');
    say([`Got it: ${title.toLowerCase()}.`, 'Now, how should I look? Pick a shape.'], 'shape');
  };

  const answerShape = (shape: Shape) => {
    setLook((l) => ({ ...l, shape }));
    push('user', <Avatar shape={shape} color={look.color} size={34} />);
    say(['Ooh, nice.', 'And what color should I be?'], 'color');
  };

  const answerColor = (color: string) => {
    setLook((l) => ({ ...l, color }));
    push('user', <span className="meet-swatch" style={{ background: color }} />);
    say(['Looking good!', 'Last one: is there anything I should always keep in mind about you? You can skip this.'], 'notes');
  };

  const answerNotes = (raw: string | null) => {
    const instructions = (raw ?? '').trim().slice(0, 8000);
    push('user', instructions || 'Skip');
    setDraft('');
    const final = { ...look, instructions };
    setLook(final);
    say([instructions ? 'Noted. I’ll remember that.' : 'No problem, you can tell me anything later.', 'Give me a second to move into my new computer…'], 'saving', () => void save(final));
  };

  const save = async (final: Look) => {
    try {
      await createAgent(final);
      say(['All set! You’ll find me on the Agent page whenever you need me.'], 'done', onReady);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      say([`Hmm, I couldn’t get set up: ${msg}`], 'retry');
    }
  };

  const submit = () => {
    if (ask === 'name') answerName(draft);
    else if (ask === 'title') answerTitle(draft);
    else if (ask === 'notes') answerNotes(draft);
  };

  const status: Status = ask === 'saving' ? 'working' : typing || ask === 'loading' ? 'thinking' : ask === 'done' ? 'idle' : 'waiting';
  const chips: [string, () => void][] =
    ask === 'name' ? NAMES.map((n) => [n, () => answerName(n)])
    : ask === 'title' ? TITLES.map((n) => [n, () => answerTitle(n)])
    : ask === 'notes' ? [['Skip', () => answerNotes(null)]]
    : ask === 'again' ? [['Make another', () => { push('user', 'Make another'); greet(); }], ['I’m good for now', () => { push('user', 'I’m good for now'); say(['Sounds good! See you on the Agent page.'], 'done', onReady); }]]
    : ask === 'retry' ? [['Try again', () => { push('user', 'Try again'); say(['Trying again…'], 'saving', () => void save(look)); }]]
    : [];
  const typed = ask === 'name' || ask === 'title' || ask === 'notes';

  return (
    <div className="meet">
      <header className="meet-head">
        <Avatar shape={look.shape} color={look.color} status={status} size={64} />
        <div className="stack tight min0">
          <b className="strong big ellipsis">{look.name || 'Your agent'}</b>
          <span className="small dim ellipsis">{ask === 'saving' ? 'Setting up…' : typing ? 'Typing…' : look.title || 'New agent'}</span>
        </div>
      </header>

      <div className="meet-scroll" ref={scroller}>
        <div className="meet-msgs">
          {lines.map((l, i) => (
            <div key={l.id} className={`agent-msg ${l.from} ${l.from === 'agent' && lines[i - 1]?.from !== 'agent' ? 'first' : ''}`}>
              <div className={l.from === 'agent' ? 'agent-bubble' : 'agent-bubble-user'}>{l.body}</div>
            </div>
          ))}
          {typing && (
            <div className="agent-msg agent typing">
              <div className="agent-typing"><span className="dots"><i /><i /><i /></span></div>
            </div>
          )}
          {ask === 'shape' && (
            <div className="meet-pick agent-shapes" role="radiogroup" aria-label="Shape">
              {AGENT_SHAPES.map((s) => (
                <button key={s} type="button" role="radio" aria-checked={look.shape === s} className={look.shape === s ? 'on' : ''} title={s} aria-label={s} onClick={() => answerShape(s)}>
                  <Avatar shape={s} color={look.color} size={40} />
                </button>
              ))}
            </div>
          )}
          {ask === 'color' && (
            <div className="meet-pick agent-colors" role="radiogroup" aria-label="Color">
              {AGENT_COLORS.map((c) => (
                <button key={c} type="button" role="radio" aria-checked={look.color === c} className={look.color === c ? 'on' : ''} style={{ background: c }} aria-label={c} onClick={() => answerColor(c)} />
              ))}
            </div>
          )}
          {ask === 'saving' && !typing && <div className="meet-saving tiny dim"><Spinner size={12} /> Creating {look.name}…</div>}
          {ask === 'done' && <div className="meet-done small"><Icon name="check" size={14} /> Ready to go</div>}
        </div>
      </div>

      {chips.length > 0 && (
        <div className="meet-chips">
          {chips.map(([label, go]) => <button key={label} type="button" className="chip" onClick={go}>{label}</button>)}
        </div>
      )}
      <form className={`meet-composer${typed ? '' : ' off'}`} onSubmit={(e) => { e.preventDefault(); submit(); }}>
        <input
          ref={input}
          className="input grow"
          value={draft}
          disabled={!typed}
          maxLength={ask === 'name' ? 40 : ask === 'title' ? 80 : 8000}
          placeholder={ask === 'name' ? 'Give it a name…' : ask === 'title' ? 'Or type your own…' : ask === 'notes' ? 'e.g. I’m a student, keep things short' : ''}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button type="submit" className="icon-button meet-send" disabled={!typed || !draft.trim()} aria-label="Send">
          <Icon name="arrowUp" size={15} />
        </button>
      </form>
    </div>
  );
}
