import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { act, invoke, type Permission } from './api';
import { LevelPicker, type Level } from './Levels';
import { ModelChip } from './ModelMenu';
import { FreeQuotaChip, meterText, PlanLimits, usePlanLimit, usedShare } from './Quota';
import { Markdown } from './Transcript';
import { focusComposer, ui, useCore, useUi, useView } from './store';
import { CodeBlock, Icon, IconButton, Popover, ShowDisplay, Spinner, TodoList } from './widgets';

const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

function Todos() {
  const todos = useView('todos');
  const { running } = useView('run');
  if (todos.length === 0 || (todos.every((t) => t.status === 'completed') && !running)) return null;
  const done = todos.filter((t) => t.status === 'completed').length;
  return (
    <div className="panel">
      <div className="row">
        <b className="strong">Tasks</b>
        <span className="small dim grow">{done} of {todos.length}</span>
        <span className="meter"><span style={{ width: `${(done * 100) / todos.length}%` }} /></span>
      </div>
      <TodoList todos={todos} />
    </div>
  );
}

/** Asks about one tool call. Both the running turn and a linked chatbot use it, with their own words. */
export function PermissionBody({ p, onAnswer, link }: { p: Permission; onAnswer: (answer: string, feedback?: string) => void; link?: boolean }) {
  const [feedback, setFeedback] = useState('');
  const always = p.prefix ? `Always allow ${p.prefix.replaceAll('`', '')} here` : p.tool === 'bash' ? null : link ? 'Allow all edits while linked' : 'Allow all edits this session';
  return (
    <>
      {p.reason && <div className="red">Held for review. {p.reason}</div>}
      {p.tool === 'bash' ? <CodeBlock code={p.label} /> : <div className="mono code-text">{p.label}</div>}
      {p.preview && <div className="card-scroll"><ShowDisplay d={p.preview} /></div>}
      <div className="row wrap">
        <button type="button" className="button primary" onClick={() => onAnswer('allow')}>Allow</button>
        {always && <button type="button" className="button" onClick={() => onAnswer('always')}>{always}</button>}
        <input className="input feedback" placeholder={link ? 'or tell the chatbot why not' : 'or tell Scout what to do instead'} value={feedback} onChange={(e) => setFeedback(e.target.value)} />
        <button type="button" className="button ghost" onClick={() => onAnswer('deny', feedback.trim())}>{!link && feedback.trim() ? 'Deny and send' : 'Deny'}</button>
      </div>
    </>
  );
}

function PermissionCard() {
  const p = useView('permission');
  const input = useUi((u) => u.input);
  // With text in the prompt, Ctrl+Enter sends it instead.
  useEffect(() => {
    if (!p || input.trim()) return;
    const key = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === 'Enter') {
        e.preventDefault();
        act('answerPermission', { answer: 'allow' });
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [p, input]);
  if (!p) return null;
  const verb = p.tool === 'bash' ? 'Run this command?' : p.tool === 'write_file' ? 'Write this file?' : p.tool === 'update_memory' ? 'Update this memory?' : p.tool.startsWith('mcp__') ? 'Use this MCP tool?' : 'Make this edit?';
  return (
    <div className="card">
      <div className="card-head"><b className="strong">{verb}</b><span className="small dim">Ctrl+Enter to allow</span></div>
      <PermissionBody p={p} onAnswer={(answer, feedback) => act('answerPermission', { answer, feedback })} />
    </div>
  );
}

function PlanCard() {
  const plan = useView('plan');
  const [feedback, setFeedback] = useState('');
  if (plan === null) return null;
  return (
    <div className="card">
      <div className="card-head"><b className="strong">Scout's plan</b><span className="small dim">Review before it starts</span></div>
      <div className="card-scroll tall"><Markdown text={plan} /></div>
      <div className="row wrap">
        <button type="button" className="button primary" onClick={() => act('approvePlan', { mode: 'default' })}>Approve</button>
        <button type="button" className="button" onClick={() => act('approvePlan', { mode: 'acceptEdits' })}>Approve, accept edits</button>
        <input className="input feedback" placeholder="what to change" value={feedback} onChange={(e) => setFeedback(e.target.value)} />
        <button type="button" className="button ghost" onClick={() => { act('revisePlan', { feedback: feedback.trim() }); setFeedback(''); }}>Keep planning</button>
      </div>
    </div>
  );
}

/** The questions from `ask_questions`: options to pick from, or a typed answer, for each question. */
function QuestionsCard() {
  const questions = useView('questions');
  const [picks, setPicks] = useState<number[][]>([]);
  const [other, setOther] = useState<string[]>([]);
  useEffect(() => {
    setPicks((questions ?? []).map(() => []));
    setOther((questions ?? []).map(() => ''));
  }, [questions]);
  const answers = (questions ?? []).map((q, i) => {
    const parts = (picks[i] ?? []).map((j) => q.options[j]?.label).filter(Boolean);
    if (other[i]?.trim()) parts.push(other[i].trim());
    return parts.length ? parts.join(', ') : null;
  });
  const ready = answers.every((a) => a !== null);
  const send = () => ready && act('answerQuestions', { answers });
  useEffect(() => {
    if (!questions) return;
    const key = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === 'Enter' && ready) {
        e.preventDefault();
        send();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  if (!questions) return null;
  return (
    <div className="card">
      <div className="card-head"><b className="strong">{questions.length === 1 ? 'Scout has a question' : 'Scout has some questions'}</b><span className="small dim">Pick an option or type your own</span></div>
      <div className="card-scroll tall stack-wide">
        {questions.map((q, i) => (
          <div key={i} className="stack">
            {q.header && <div className="section-label">{q.header}</div>}
            <div className="strong">{q.question}</div>
            <div className="row wrap">
              {q.options.map((o, j) => {
                const on = picks[i]?.includes(j);
                return (
                  <button key={j} type="button" className={`button ${on ? 'primary' : ''}`} title={o.description || undefined} onClick={() => setPicks(picks.map((p, k) => k !== i ? p : on ? p.filter((x) => x !== j) : q.multiSelect ? [...p, j] : [j]))}>{o.label}</button>
                );
              })}
            </div>
            <input className="input" placeholder="Or type your own answer" value={other[i] ?? ''} onChange={(e) => setOther(other.map((t, k) => (k === i ? e.target.value : t)))} />
          </div>
        ))}
      </div>
      <div className="row">
        <button type="button" className="button" disabled={!ready} onClick={send}>Send answers</button>
        <button type="button" className="button ghost" onClick={() => act('answerQuestions', { answers: null })}>Skip</button>
      </div>
    </div>
  );
}

function QueueCard() {
  const queue = useView('queue');
  const steering = useView('steering');
  const { running } = useView('run');
  if (queue.length === 0 && steering.length === 0) return null;
  return (
    <div className="panel queue">
      {steering.map((t, i) => <QueuedRow key={`s${i}`} text={t} files={0} status="Sending now · Scout reads it after this step" />)}
      {queue.map((q, i) => (
        <QueuedRow key={i} text={q.text} files={q.files} status={running ? 'Queued · sent when Scout is done' : 'Queued'}>
          <button type="button" className="text-button" title={running ? "Send it now: Scout reads it after the step it's on" : undefined} onClick={() => act('sendQueued', { at: i })}>{running ? 'Steer' : 'Send'}</button>
          <IconButton icon="close" size={24} tip="Take it out of the queue (back into the box when that's empty)" onClick={async () => {
            const o = await invoke<{ text: string; files: string[] } | null>('unqueue', { at: i });
            // Back into the box, in case it was removed to be changed.
            if (o && !ui.get().input.trim()) ui.set({ input: o.text, attachments: o.files });
          }} />
        </QueuedRow>
      ))}
    </div>
  );
}

function QueuedRow({ text, files, status, children }: { text: string; files: number; status: string; children?: React.ReactNode }) {
  return (
    <div className="queued">
      <div className="grow min0">
        <div className="ellipsis">{text.split('\n')[0] || `${files} file${files === 1 ? '' : 's'}`}</div>
        <div className="tiny dim">{status}{text && files ? ` · ${files} file${files === 1 ? '' : 's'}` : ''}</div>
      </div>
      {children}
    </div>
  );
}

type Pick = { skill: boolean; name: string; about?: string };

/** What can follow a `/` in the box. Scout's core runs them; `arg` ones take text after the name. */
const COMMANDS: { name: string; about: string; arg?: string }[] = [
  { name: 'plan', about: 'Have Scout write a plan before it starts', arg: 'what to do' },
  { name: 'compact', about: 'Summarize the chat so far to free up context', arg: 'focus (optional)' },
  { name: 'undo', about: "Take back the last turn and the files it changed" },
  { name: 'prune', about: 'Drop old tool output from the context' },
  { name: 'new', about: 'Start a new chat' },
];

function currentWord(text: string, cursor: number) {
  let start = cursor;
  while (start > 0 && !/\s/.test(text[start - 1])) start -= 1;
  return text[start] === '@' && start < cursor ? { start, end: cursor, query: text.slice(start + 1, cursor) } : null;
}

const history: string[] = [];

function InputBox() {
  const input = useUi((u) => u.input);
  const attachments = useUi((u) => u.attachments);
  const focusEpoch = useUi((u) => u.focusEpoch);
  const coreFocus = useView('focusEpoch');
  const { running, stopping } = useView('run');
  const chat = useView('chat');
  const voice = useView('voice');
  const box = useRef<HTMLTextAreaElement>(null);
  const historyPos = useRef<number | null>(null);
  const [mention, setMention] = useState<{ start: number; end: number; query: string } | null>(null);
  const closedAt = useRef<number | null>(null);
  const [picks, setPicks] = useState<Pick[]>([]);
  const [pick, setPick] = useState(0);
  // The / menu shows while the box holds just a command name, until Esc.
  const [slashClosed, setSlashClosed] = useState(false);
  const slashQuery = /^\/\S*$/.test(input) ? input.slice(1).toLowerCase() : null;
  const commands = slashQuery === null || slashClosed ? [] : COMMANDS.filter((c) => c.name.startsWith(slashQuery));
  useEffect(() => { if (slashQuery === null) setSlashClosed(false); setPick(0); }, [slashQuery]);

  // Not while someone is typing in the terminal or the editor.
  useEffect(() => {
    if (!document.activeElement?.closest('.dock')) box.current?.focus();
  }, [focusEpoch, coreFocus]);
  // Attached paths are relative to the folder.
  const cwd = useRef(chat.cwd);
  useEffect(() => {
    if (cwd.current !== chat.cwd) ui.set({ attachments: [] });
    cwd.current = chat.cwd;
  }, [chat.cwd]);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`;
  }, [input]);
  useEffect(() => {
    if (!mention) return setPicks([]);
    let stale = false;
    invoke<Pick[]>('mention_search', { cwd: chat.cwd, query: mention.query }).then((rows) => { if (!stale) { setPicks(rows); setPick(0); } });
    return () => { stale = true; };
  }, [mention?.query, mention !== null, chat.cwd]); // eslint-disable-line

  const updateMention = () => {
    const el = box.current!;
    const found = currentWord(el.value, el.selectionStart);
    if (!found) closedAt.current = null;
    setMention(found && closedAt.current !== found.start ? found : null);
  };

  const choose = (p: Pick) => {
    if (!mention) return;
    const insert = `@${p.name} `;
    const text = input.slice(0, mention.start) + insert + input.slice(mention.end);
    const at = mention.start + insert.length;
    ui.set({ input: text, attachments: !p.skill && !attachments.includes(p.name) ? [...attachments, p.name] : attachments });
    setMention(null);
    requestAnimationFrame(() => box.current?.setSelectionRange(at, at));
  };

  const chooseCommand = (c: (typeof COMMANDS)[number]) => {
    if (!c.arg) return void send(false, `/${c.name}`);
    const text = `/${c.name} `;
    ui.set({ input: text });
    requestAnimationFrame(() => box.current?.setSelectionRange(text.length, text.length));
  };

  const send = async (steer: boolean, typed = input) => {
    const text = typed.trim();
    if (!text && attachments.length === 0) return;
    const taken = await invoke<boolean>('send', { text, files: attachments, steer });
    if (!taken) return;
    if (text && history[history.length - 1] !== text) history.push(text);
    historyPos.current = null;
    ui.set({ input: '', attachments: [] });
    setMention(null);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (commands.length > 0) {
      if (e.key === 'ArrowDown') { e.preventDefault(); return setPick((pick + 1) % commands.length); }
      if (e.key === 'ArrowUp') { e.preventDefault(); return setPick((pick + commands.length - 1) % commands.length); }
      if ((e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.altKey) || (e.key === 'Tab' && !e.shiftKey)) { e.preventDefault(); return chooseCommand(commands[pick] ?? commands[0]); }
      if (e.key === 'Escape') { e.preventDefault(); return setSlashClosed(true); }
    }
    if (mention) {
      if (picks.length > 0) {
        if (e.key === 'ArrowDown') { e.preventDefault(); return setPick((pick + 1) % picks.length); }
        if (e.key === 'ArrowUp') { e.preventDefault(); return setPick((pick + picks.length - 1) % picks.length); }
        if ((e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.altKey) || (e.key === 'Tab' && !e.shiftKey)) { e.preventDefault(); return choose(picks[pick]); }
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        closedAt.current = mention.start;
        return setMention(null);
      }
    }
    // Enter sends (or queues while Scout works), Ctrl+Enter sends into the running turn, Shift+Enter makes a new line.
    if (e.key === 'Enter' && !e.shiftKey && !e.altKey) {
      // With an empty box, Ctrl+Enter is the card's shortcut.
      if (e.ctrlKey && !input.trim() && attachments.length === 0) return;
      e.preventDefault();
      return void send(e.ctrlKey);
    }
    // ↑/↓ walk history when the box is empty or unchanged.
    const atHistory = !input || (historyPos.current !== null && history[historyPos.current] === input);
    if (atHistory && history.length > 0 && !e.shiftKey && !e.ctrlKey) {
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        const p = historyPos.current === null ? history.length - 1 : Math.max(0, historyPos.current - 1);
        historyPos.current = p;
        ui.set({ input: history[p] });
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        const p = historyPos.current;
        if (p !== null && p + 1 < history.length) {
          historyPos.current = p + 1;
          ui.set({ input: history[p + 1] });
        } else {
          historyPos.current = null;
          ui.set({ input: '' });
        }
      }
    }
  };

  // A screenshot or copied image is attached as a file. Text (even with an image) pastes as usual.
  const onPaste = async (e: React.ClipboardEvent) => {
    const item = Array.from(e.clipboardData.items).find((i) => i.type.startsWith('image/'));
    if (!item || e.clipboardData.getData('text/plain')) return;
    const file = item.getAsFile();
    if (!file) return;
    e.preventDefault();
    const ext = { 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp', 'image/bmp': 'bmp' }[file.type] ?? 'png';
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = '';
    for (const b of bytes) binary += String.fromCharCode(b);
    try {
      const path = await invoke<string>('paste_image', { data: btoa(binary), ext });
      const had = ui.get().attachments;
      ui.set({ attachments: [...had, path] });
    } catch {
      // The box keeps what was typed; the image just doesn't attach.
    }
  };

  const ready = !!input.trim() || attachments.length > 0;
  return (
    <div className={`input-box ${running ? 'working' : ''}`}>
      {attachments.length > 0 && (
        <div className="chips">
          {attachments.map((f) => (
            <span key={f} className="file-chip" title={f}>
              <Icon name="file" size={13} /><span className="ellipsis">{f.split(/[\\/]/).pop()}</span>
              <button type="button" aria-label={`Remove ${f}`} onClick={() => ui.set({ attachments: attachments.filter((a) => a !== f) })}><Icon name="close" size={11} /></button>
            </span>
          ))}
        </div>
      )}
      <textarea
        ref={box}
        className="prompt"
        rows={2}
        value={input}
        placeholder={running ? 'Scout is working… type to queue a message' : 'Ask Scout to build, fix or explain something'}
        onChange={(e) => { historyPos.current = null; ui.set({ input: e.target.value }); requestAnimationFrame(updateMention); }}
        onKeyDown={onKey}
        onPaste={onPaste}
        onClick={updateMention}
        onKeyUp={(e) => { if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') updateMention(); }}
        onBlur={() => setTimeout(() => setMention(null), 150)}
      />
      {commands.length > 0 && (
        <div className="mention-menu">
          {commands.map((c, i) => (
            <button key={c.name} type="button" className={`mention-row ${i === pick ? 'selected' : ''}`} onMouseDown={(e) => { e.preventDefault(); chooseCommand(c); }}>
              <span className="strong mono">/{c.name}</span>
              {c.arg && <span className="small faint nowrap">{c.arg}</span>}
              <span className="small dim ellipsis grow">{c.about}</span>
            </button>
          ))}
          <div className="tiny faint pad-x">↑↓ to choose · Enter or Tab to pick · Esc to close</div>
        </div>
      )}
      {mention && picks.length > 0 && (
        <div className="mention-menu">
          {picks.map((p, i) => (
            <button key={`${p.skill}${p.name}`} type="button" className={`mention-row ${i === pick ? 'selected' : ''}`} onMouseDown={(e) => { e.preventDefault(); choose(p); }}>
              <Icon name={p.skill ? 'chat' : 'file'} size={14} />
              <span className={`ellipsis ${p.skill ? 'strong' : ''}`}>{p.skill ? `@${p.name}` : p.name}</span>
              {p.about && <span className="small dim ellipsis grow">{p.about}</span>}
            </button>
          ))}
          <div className="tiny faint pad-x">↑↓ to choose · Enter or Tab to pick · Esc to close</div>
        </div>
      )}
      <div className="row composer-controls">
        <button type="button" className="round attach" title="Attach files (or drop them on the window, or type @)" aria-label="Attach files" onClick={async () => {
          const files = await invoke<string[]>('pick_attachments', { cwd: chat.cwd });
          const had = ui.get().attachments;
          ui.set({ attachments: [...had, ...files.filter((f) => !had.includes(f))] });
          focusComposer();
        }}><Icon name="plus" /></button>
        <span className="grow" />
        {voice.state === 'idle' && <IconButton icon="mic" tip="Dictate (Ctrl+Shift+Space). Transcribed by Deepgram through ScoutCode" onClick={() => act('toggleVoice')} />}
        {voice.state === 'recording' && (
          <button type="button" className="mic-live" title="Listening. Click (or Ctrl+Shift+Space) to stop and transcribe" onClick={() => act('toggleVoice')}>
            <span className="mic-ring" style={{ transform: `scale(${1 + voice.level * 0.6})` }} />
            <Icon name="mic" />
          </button>
        )}
        {voice.state === 'busy' && <span className="mic-busy" title={voice.label}><Spinner size={14} /></span>}
        {(ready || !running) && (
          <button type="button" className={`round send ${ready ? 'ready' : ''}`} disabled={!ready} aria-label="Send" title={running ? 'Queue (Enter) · Ctrl+click or Ctrl+Enter to send it now' : undefined} onClick={(e) => send(e.ctrlKey)}><Icon name="arrowUp" /></button>
        )}
        {running && <button type="button" className="round stop" aria-label="Stop" title={stopping ? 'Stopping' : 'Stop (Esc)'} onClick={() => act('stop')}><span className="stop-square" /></button>}
      </div>
    </div>
  );
}

function UsageRing() {
  const { totals, context } = useView('totals');
  const { autoCompactTokens } = useView('cfg');
  // A plan with limits (Claude, Codex, Grok): how much of the window closest to running out is used. Otherwise the
  // share of the funds this session has spent, grey until the server reports a balance.
  const plan = usePlanLimit();
  const b = totals.balance;
  const frac = plan ? usedShare(plan.top) : b !== undefined && b >= 0 && totals.cost + b > 0 ? Math.min(1, totals.cost / (totals.cost + b)) : 0;
  const low = plan !== null && frac >= 0.9;
  const c = 2 * Math.PI * 6;
  const limit = Math.max(1, autoCompactTokens);
  const rows: [string, string][] = [
    ['Cost', `$${totals.cost.toFixed(4)}`], ['Requests', String(totals.requests)], ['Prompt tokens', `${totals.promptTokens} (${totals.cachedTokens} cached)`],
    ['Completion tokens', String(totals.completionTokens)], ['Context', `${(context / 1000).toFixed(1)}k tokens · ${Math.min(100, (context / limit) * 100).toFixed(0)}% of auto-compact`],
  ];
  return (
    <Popover align="end" side="top" width={280} trigger={(_, toggle) => (
      <button type="button" className="usage-ring" title={plan ? `${plan.top.label}: ${meterText(plan.top)}` : 'Usage this session'} onClick={() => { if (plan) act('quotaRefresh'); toggle(); }}>
        <svg width={16} height={16} viewBox="0 0 16 16">
          <circle cx={8} cy={8} r={6} fill="none" strokeWidth={2.5} className="ring-track" />
          {frac > 0 && <circle cx={8} cy={8} r={6} fill="none" strokeWidth={2.5} className={`ring-fill ${low ? 'low' : ''}`} strokeDasharray={`${frac * c} ${c}`} transform="rotate(-90 8 8)" />}
        </svg>
      </button>
    )}>
      {() => (
        <div className="stack pad">
          {plan && <PlanLimits snapshots={plan.snapshots} />}
          <b className="strong">Usage this session</b>
          {rows.map(([k, v]) => <div key={k} className="stat-row wide-key"><span className="small dim">{k}</span><span className="small">{v}</span></div>)}
        </div>
      )}
    </Popover>
  );
}

/** Permission modes from the most caution to the most freedom, so right on the slider is freer. */
const SECURITY_ORDER = ['plan', 'default', 'acceptEdits', 'auto', 'yolo'];

function securityLevels(modes: { id: string; label: string }[]): Level[] {
  const rank = (id: string) => { const i = SECURITY_ORDER.indexOf(id); return i < 0 ? SECURITY_ORDER.length : i; };
  return [...modes].sort((a, b) => rank(a.id) - rank(b.id)).map((m) => ({ id: m.id, label: m.label }));
}

function Footer() {
  const { running, status } = useView('run');
  const mode = useView('mode');
  const effort = useView('effort');
  const totals = useView('totals').totals;
  const permission = useView('permission');
  const plan = useView('plan');
  const questions = useView('questions');
  const { modes, efforts } = useCore((s) => s.statics);
  const typing = useUi((u) => !!u.input.trim() || u.attachments.length > 0);
  const waiting = permission || plan !== null || questions;
  return (
    <div className="footer">
      <LevelPicker
        title="Permissions"
        tip="Permission mode (Shift+Tab)"
        tone="free"
        less="Safer"
        more="Freer"
        levels={securityLevels(modes)}
        value={mode}
        onChange={(id) => act('setMode', { mode: id })}
      />
      {running ? (
        <>
          <Spinner size={11} />
          <span className="tiny muted ellipsis">{status || (waiting ? 'Waiting for you' : 'Working…')}</span>
          <span className="tiny dim nowrap">{typing ? '· Enter to queue · Ctrl+Enter to send now' : '· Esc to stop'}</span>
        </>
      ) : status && <span className="tiny dim ellipsis">{status}</span>}
      <span className="grow" />
      <ModelChip />
      <LevelPicker
        title="Reasoning Effort"
        tip="Reasoning effort"
        tone="reason"
        less="Faster"
        more="Smarter"
        align="end"
        levels={efforts.map((e) => ({ id: e, label: capital(e) }))}
        value={effort}
        onChange={(id) => act('setEffort', { effort: id })}
      />
      <FreeQuotaChip />
      {totals.balance !== undefined && <span className="tiny dim nowrap">balance ${totals.balance.toFixed(2)}</span>}
      <UsageRing />
    </div>
  );
}

export function Composer() {
  return (
    <div className="composer">
      <div className="composer-column">
        <Todos />
        <PermissionCard />
        <PlanCard />
        <QuestionsCard />
        <QueueCard />
        <InputBox />
        <Footer />
      </div>
    </div>
  );
}

