import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { act, openExternal, type AgentStep, type Item, type ModelRef } from './api';
import { logo } from './Accounts';
import { ModelPicker, useModelName } from './ModelMenu';
import { core, focusComposer, ui, useCore, useUi, useView } from './store';
import { CodeBlock, CopyIconButton, Icon, IconButton, MenuItem, Modal, Popover, ShowDisplay, Spinner } from './widgets';
import { ScoutLogo } from './logo';
import { addProject } from './Sidebar';

type ToolItem = Extract<Item, { kind: 'tool' }>;

export function toolTitle(name: string): string {
  const titles: Record<string, string> = {
    view: 'Read', write_file: 'Write', edit_file: 'Edit', multi_edit: 'Edit', apply_patch: 'Patch', list_dir: 'List', glob: 'Find', grep: 'Search', bash: 'Run',
    shell_output: 'Output', kill_shell: 'Stop', web_fetch: 'Fetch', web_search: 'Web search', todo_write: 'Todos', task: 'Task', present_plan: 'Plan', ask_questions: 'Ask', update_memory: 'Memory', generate_image: 'Image',
    browser: 'Browser', browser_navigate: 'Open', browser_snapshot: 'Read page', browser_click: 'Click', browser_type: 'Type', browser_select: 'Pick', browser_press: 'Press', browser_scroll: 'Scroll', browser_wait: 'Wait', browser_eval: 'Page script',
    claude: 'Claude Code', codex: 'Codex', kiro: 'Kiro', antigravity: 'Antigravity', vibe: 'Mistral Vibe', opencode: 'OpenCode', grok: 'Grok Build',
  };
  return titles[name] ?? name.replace(/^mcp__/, '');
}

/** Whether a tool's output starts open: the ones that change files, whose diffs are the point. */
const OPEN_BY_DEFAULT = new Set(['write_file', 'edit_file', 'multi_edit', 'apply_patch', 'todo_write', 'generate_image']);

/** A web address, which opens in the browser; anything else in a link is a file. */
const WEB = /^(https?:|mailto:)/i;
const EXTENSIONS = 'rs|ts|tsx|js|jsx|mjs|cjs|py|go|java|kt|cs|cpp|c|h|hpp|rb|php|swift|json|jsonc|toml|yaml|yml|md|mdx|txt|css|scss|html|astro|vue|svelte|sql|sh|ps1|bat|cfg|ini|lock|xml|gradle';
/** A path as models write them in backticks: with a folder (src/a.rs) or a known extension (main.rs), and :line or :line:col. */
const PATH_LIKE = new RegExp(String.raw`^(?:[A-Za-z]:[\\/]|\.{0,2}[\\/])?(?:(?:[\w@.+-]+[\\/])+[\w@.+-]+\.\w+|[\w@+-]+\.(?:${EXTENSIONS}))(?::\d+(?::\d+)?)?$`);

/** The file and line a link or path points to, resolved against the chat's folder; null for anything else. */
export function fileTarget(raw: string, cwd: string): { path: string; line: number | null } | null {
  let t = raw.trim().replace(/^file:\/\/\/?/i, '');
  try {
    t = decodeURIComponent(t);
  } catch {
    // Kept as written.
  }
  if (!t || t.startsWith('#') || WEB.test(t) || /^[a-z][a-z0-9+.-]*:\/\//i.test(t)) return null;
  let line: number | null = null;
  const hash = /#L(\d+)(?:-L?\d+)?$/.exec(t);
  const colon = /:(\d+)(?::\d+)?(?:-\d+)?$/.exec(t);
  if (hash) {
    line = Number(hash[1]);
    t = t.slice(0, hash.index);
  } else if (colon) {
    line = Number(colon[1]);
    t = t.slice(0, colon.index);
  }
  t = t.replace(/^\.[\\/]/, '');
  if (!t || /^[A-Za-z]:$/.test(t)) return null;
  const absolute = /^[A-Za-z]:[\\/]/.test(t) || t.startsWith('\\\\') || (t.startsWith('/') && !cwd);
  const path = absolute ? t : `${cwd.replace(/[\\/]+$/, '')}\\${t.replace(/^[\\/]+/, '')}`;
  return { path: path.replaceAll('/', '\\'), line };
}

/** Opens a file in the Files tab, at a line when there is one. */
export function openFileAt(target: { path: string; line: number | null }) {
  ui.set({ dockOpen: true, dockTab: 'files', openFile: target.path, openLine: target.line });
}

function FileLink({ target, title, children }: { target: { path: string; line: number | null }; title: string; children: ReactNode }) {
  return (
    <a className="link file-link" href="#" title={`Open ${title}`} onClick={(e) => { e.preventDefault(); openFileAt(target); }}>
      {children}
    </a>
  );
}

/** Keeps file paths in links (react-markdown would drop "main.rs:42" as an unknown protocol), but never script URLs. */
const keepUrl = (url: string) => (/^\s*(javascript|vbscript|data):/i.test(url) ? '' : url);

export function Markdown({ text }: { text: string }) {
  const cwd = useView('chat').cwd;
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={keepUrl}
        components={{
          a: ({ href, children }) => {
            const target = href ? fileTarget(href, cwd) : null;
            if (target) return <FileLink target={target} title={href!}>{children}</FileLink>;
            return <a className="link" href={href} onClick={(e) => { e.preventDefault(); if (href && WEB.test(href)) openExternal(href); }}>{children}</a>;
          },
          // A path in backticks opens too. Code blocks have a language class, or go through `pre` below.
          code: ({ className, children }) => {
            const t = typeof children === 'string' ? children.trim() : null;
            const target = !className && t && PATH_LIKE.test(t) ? fileTarget(t, cwd) : null;
            if (target) return <FileLink target={target} title={t!}><code>{children}</code></FileLink>;
            return <code className={className}>{children}</code>;
          },
          // Replies don't get to fetch things: an image shows as the link it is.
          img: ({ src, alt }) => (typeof src === 'string' ? <a className="link" href={src} onClick={(e) => { e.preventDefault(); openExternal(src); }}>{alt || src}</a> : <>{alt}</>),
          pre: ({ children }) => {
            const code = children as { props?: { children?: unknown } };
            return <CodeBlock code={String(code?.props?.children ?? '').replace(/\n$/, '')} />;
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

function FileChip({ path }: { path: string }) {
  return <span className="file-chip" title={path}><Icon name="file" size={13} /><span className="ellipsis">{path.split(/[\\/]/).pop()}</span></span>;
}

function UserMessage({ item, at, editable, onRetry }: { item: Extract<Item, { kind: 'user' }>; at: number; editable: boolean; onRetry: () => void }) {
  const [draft, setDraft] = useState<string | null>(null);
  if (draft !== null) {
    const send = () => {
      if (draft.trim()) act('resend', { at, text: draft.trim() });
      setDraft(null);
    };
    return (
      <div className="edit-box">
        <textarea autoFocus value={draft} rows={Math.min(12, draft.split('\n').length + 1)} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            setDraft(null);
          } else if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }} />
        <div className="row end">
          <span className="small dim grow">The chat goes back to before this message, and the files Scout changed since are put back.</span>
          <button type="button" className="button ghost" onClick={() => setDraft(null)}>Cancel</button>
          <button type="button" className="button primary" onClick={send}>Send</button>
        </div>
      </div>
    );
  }
  return (
    <div className="user-row">
      <div className="user-actions">
        {item.text && <CopyIconButton text={item.text} size={26} />}
        {editable && (
          <>
            <IconButton icon="refresh" tip="Retry" size={26} onClick={onRetry} />
            <IconButton icon="pencil" tip="Edit and send again" size={26} onClick={() => setDraft(item.text)} />
          </>
        )}
      </div>
      <div className="user-bubble">
        {item.text && <div className="pre-wrap">{item.text}</div>}
        {item.files.length > 0 && <div className="chips">{item.files.map((f) => <FileChip key={f} path={f} />)}</div>}
      </div>
    </div>
  );
}

const REASONING_KEY = 'scout:reasoningOpen';
/** Whether reasoning starts expanded: the last choice made on any reply. */
let reasoningOpen = (() => { try { return localStorage.getItem(REASONING_KEY) === '1'; } catch { return false; } })();

/** What the model thought. While it's still thinking (and hasn't started its answer) the thoughts stream in open, the
 *  newest lines in view; afterwards they fold to a few lines, or stay open if that's how you last left them. */
function Reasoning({ text, live }: { text: string; live: boolean }) {
  const [open, setOpen] = useState(reasoningOpen);
  const box = useRef<HTMLDivElement>(null);
  const toggle = () => {
    const next = !open;
    setOpen(next);
    reasoningOpen = next;
    try { localStorage.setItem(REASONING_KEY, next ? '1' : '0'); } catch {}
  };
  useLayoutEffect(() => {
    if (live && !open && box.current) box.current.scrollTop = box.current.scrollHeight;
  });
  const lines = useMemo(() => text.trim().split('\n').filter((l) => l.trim()).length, [text]);
  return (
    <div className={`reasoning-block ${live ? 'live' : ''} ${open ? 'open' : ''}`}>
      <button type="button" className="reasoning-head" onClick={toggle} aria-expanded={open}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} />
        <b className={live ? 'shimmer' : ''}>{live ? 'Thinking' : 'Reasoning'}</b>
        {!open && !live && lines > 3 && <span className="tiny dim">{lines} lines</span>}
      </button>
      <div ref={box} className={`reasoning pre-wrap ${open ? '' : live ? 'peek-live' : 'peek'}`} onClick={!open && !live ? toggle : undefined}>
        {text.trim()}
      </div>
    </div>
  );
}

function Assistant({ item, at, editable, showModel, onRetry }: { item: Extract<Item, { kind: 'assistant' }>; at: number; editable: boolean; showModel: boolean; onRetry: () => void }) {
  const { running } = useView('run');
  const isLast = useCore((s) => s.items[s.items.length - 1]?.id === item.id);
  const thinking = running && isLast && !item.text;
  const canRetry = useMemo(() => {
    const all = core.get()?.items ?? [];
    for (let i = at - 1; i >= 0; i--) {
      if (all[i]?.kind === 'user') return true;
    }
    return false;
  }, [at]);

  const modelRef = useMemo(() => (item.model ? { key: item.model, provider: item.provider ?? 'scoutcode' } : null), [item.model, item.provider]);
  const modelName = useModelName(modelRef);
  const provLogo = modelRef?.provider ? logo(modelRef.provider) : '';

  return (
    <div className="assistant">
      {item.reasoning.trim() && <Reasoning text={item.reasoning} live={thinking} />}
      <Markdown text={item.text} />
      {(item.text || item.reasoning) && (
        <div className="assistant-foot">
          {showModel && modelName ? (
            <div className="assistant-model tiny dim" title={modelName}>
              {provLogo && <img className="logo" src={provLogo} width={12} height={12} alt="" />}
              <span>{modelName}</span>
            </div>
          ) : <div />}
          {item.text && (
            <div className="assistant-actions">
              {item.tps != null && item.tps > 0 && (
                <span className="assistant-tps tiny dim" title="Average speed of this reply, from its first word to its last (about 4 characters a token)">
                  {item.tps >= 100 ? Math.round(item.tps) : item.tps.toFixed(1)} tok/s
                </span>
              )}
              <CopyIconButton text={item.text} size={24} />
              {editable && canRetry && (
                <IconButton icon="refresh" tip="Retry" size={24} onClick={onRetry} />
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolRow({ item }: { item: ToolItem }) {
  const [opened, setOpened] = useState<boolean | null>(null);
  const r = item.result;
  const steps = item.steps ?? [];
  // A sub-agent can be opened while it works, to watch its steps come in.
  const live = !r && item.name === 'task' && steps.length > 0;
  const canOpen = !!r || live;
  const open = canOpen ? opened ?? (r ? OPEN_BY_DEFAULT.has(item.name) : false) : false;
  const label = item.label || (item.name === 'bash' ? (r ? 'Command' : 'Terminal…') : '');
  const latest = [...steps].reverse().find((s) => s.tool);
  const progress = live ? (latest ? `${toolTitle(latest.tool)} ${latest.label}`.trim() : '') : item.progress;
  return (
    <div className="tool">
      <button type="button" className={`tool-row ${canOpen ? 'can-open' : ''}`} onClick={() => canOpen && setOpened(!open)}>
        {!r ? <span className="tool-status"><Spinner /></span> : <span className={`tool-status ${r.isError ? 'bad' : 'good'}`}><Icon name={r.isError ? 'close' : 'check'} size={12} /></span>}
        <b>{toolTitle(item.name)}</b>
        {label && <span className="mono muted ellipsis tool-label">{label.split('\n')[0].slice(0, 160)}</span>}
        {r?.summary && <span className={`tool-summary ellipsis ${r.isError ? 'red' : 'dim'}`}>· {r.summary}</span>}
        {live && <span className="tool-summary ellipsis dim">· {steps.filter((s) => s.tool).length} steps</span>}
        <span className="grow" />
        {canOpen && <span className="tool-chevron"><Icon name={open ? 'chevronDown' : 'chevronRight'} /></span>}
      </button>
      {!r && !open && progress && <div className="tool-progress mono dim ellipsis">{progress}</div>}
      {open && (
        <div className="tool-output">
          {r?.display.type === 'agent' ? <AgentView steps={r.display.steps} answer={r.display.answer} />
            : r ? <ShowDisplay d={r.display} />
            : <AgentView steps={steps} answer="" />}
        </div>
      )}
    </div>
  );
}

/** What a sub-agent did: each tool call and note in order, then its answer. */
function AgentView({ steps, answer }: { steps: AgentStep[]; answer: string }) {
  const [showAnswer, setShowAnswer] = useState(true);
  return (
    <div className="agent-view">
      {steps.length > 0 && (
        <ol className="agent-steps">
          {steps.map((s, i) => s.tool ? (
            <li key={i} className="agent-step">
              {!s.done ? <span className="tool-status"><Spinner size={10} /></span> : <span className={`tool-status ${s.isError ? 'bad' : 'good'}`}><Icon name={s.isError ? 'close' : 'check'} size={11} /></span>}
              <b>{toolTitle(s.tool)}</b>
              {s.label && <span className="mono muted ellipsis tool-label" title={s.label}>{s.label.split('\n')[0].slice(0, 200)}</span>}
              {s.summary && <span className={`tool-summary ellipsis ${s.isError ? 'red' : 'dim'}`}>· {s.summary}</span>}
            </li>
          ) : (
            <li key={i} className="agent-note pre-wrap">{s.text}</li>
          ))}
        </ol>
      )}
      {steps.length === 0 && !answer && <span className="small dim">No steps yet.</span>}
      {answer && (
        <div className="agent-answer">
          <button type="button" className="reasoning-head" onClick={() => setShowAnswer(!showAnswer)}>
            <Icon name={showAnswer ? 'chevronDown' : 'chevronRight'} />
            <b>Answer</b>
          </button>
          {showAnswer && <div className="agent-answer-body"><Markdown text={answer} /></div>}
        </div>
      )}
    </div>
  );
}

/** A memory Scout saved: one quiet line with what it now knows, rather than a file change. */
function MemoryNote({ item }: { item: ToolItem }) {
  const r = item.result!;
  const full = r.display?.type === 'text' ? r.display.text : '';
  return (
    <div className="memory-note" title={full || undefined}>
      <Icon name="memory" size={13} />
      <span className="memory-verb">{r.summary ?? 'Remembered'}</span>
      {full && <span className="ellipsis">{full.split('\n')[0]}</span>}
    </div>
  );
}

/** The turn stopped because the account can't pay for its model. The choices last until something comes after it. */
function OutOfCredit({ item }: { item: { id: number; text: string } }) {
  const { running } = useView('run');
  const last = useCore((s) => s.items[s.items.length - 1]?.id === item.id);
  const models = useCore((s) => s.models);
  const free = models.filter((m) => m.hosted && m.free && m.tools !== false && m.available !== false).slice(0, 3);
  const retry = (key?: string, provider?: string) => act('retryTurn', { key, provider });
  return (
    <div className="error-box credit-box">
      <Icon name="close" />
      <div className="stack grow">
        <span className="pre-wrap">{item.text}</span>
        {last && !running && (
          <>
            <div className="credit-actions">
              <button type="button" className="button primary" onClick={() => retry()}>Try again</button>
              <ModelPicker label="Choose another model" onPick={retry} />
              <button type="button" className="button" onClick={() => ui.set({ page: 'usage' })}>See usage</button>
            </div>
            {free.length > 0 && (
              <div className="credit-actions">
                <span className="small">Continue on a free model:</span>
                {free.map((m) => <button key={m.id} type="button" className="button" onClick={() => retry(m.id, 'scoutcode')}>{m.label}</button>)}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

/** A model failed and there are no custom agents yet: the error, with an offer to make one that carries a turn on to
 *  the next model when one fails, or to try again on another model now. */
function AgentSuggest({ item }: { item: Extract<Item, { kind: 'agentSuggest' }> }) {
  const { running } = useView('run');
  const routers = useView('cfg').agentRouters;
  const last = useCore((s) => s.items[s.items.length - 1]?.id === item.id);
  const name = useModelName({ key: item.key, provider: item.provider });
  const retry = (key?: string, provider?: string) => act('retryTurn', { key, provider });
  return (
    <div className="error-box credit-box">
      <Icon name="close" />
      <div className="stack grow">
        <span className="pre-wrap">{item.text}</span>
        {last && !running && (
          <>
            {!routers?.length && (
              <span className="small dim">
                Make a custom agent: a route of models, starting with {name}, where the next one carries the turn on whenever one fails or hits a limit.
              </span>
            )}
            <div className="credit-actions">
              {!routers?.length && <button type="button" className="button primary" onClick={() => ui.set({ settings: 'router' })}>Make a custom agent</button>}
              <button type="button" className={`button ${routers?.length ? 'primary' : ''}`} onClick={() => retry()}>Try again</button>
              <ModelPicker label="Choose another model" onPick={retry} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

const Row = memo(function Row({ item, at, editable, hit, showModel = false, onRetry }: { item: Item; at: number; editable: boolean; hit: boolean; showModel?: boolean; onRetry: (at: number) => void }) {
  let body: ReactNode;
  switch (item.kind) {
    case 'user':
      body = <UserMessage item={item} at={at} editable={editable} onRetry={() => onRetry(at)} />;
      break;
    case 'assistant':
      body = <Assistant item={item} at={at} editable={editable} showModel={showModel} onRetry={() => onRetry(at)} />;
      break;
    case 'tool':
      body = item.name === 'update_memory' && item.result && !item.result.isError ? <MemoryNote item={item} /> : <ToolRow item={item} />;
      break;
    case 'notice':
      body = <div className="notice"><span>{item.text}</span></div>;
      break;
    case 'error':
      body = <div className="error-box"><Icon name="close" /><span className="pre-wrap">{item.text}</span></div>;
      break;
    case 'outOfCredit':
      body = <OutOfCredit item={item} />;
      break;
    case 'agentSuggest':
      body = <AgentSuggest item={item} />;
      break;
  }
  return <div className={`item item-${item.kind} ${hit ? 'hit' : ''}`} data-at={at}>{body}</div>;
});

/** A run of two or more tool calls the model has moved on from, under one header. */
function ToolGroup({ items, first, hit, onRetry }: { items: ToolItem[]; first: number; hit: number | null; onRetry: (at: number) => void }) {
  const [open, setOpen] = useState(false);
  const counts = new Map<string, number>();
  for (const i of items) counts.set(toolTitle(i.name), (counts.get(toolTitle(i.name)) ?? 0) + 1);
  const summary = [...counts].map(([t, n]) => `${t} ${n}`).join(', ');
  const failed = items.filter((i) => i.result?.isError).length;
  const inside = hit !== null && hit >= first && hit < first + items.length;
  const shown = open || inside;
  return (
    <>
      <div className="item" data-at={shown ? undefined : first}>
        <button type="button" className="tool-row can-open" onClick={() => setOpen(!open)}>
          <span className="tool-status good"><Icon name="check" size={12} /></span>
          <b>{items.length} tool calls</b>
          <span className="tool-summary dim ellipsis">· {summary}</span>
          <span className="grow" />
          {failed > 0 && <span className="tool-summary red">· {failed} failed</span>}
          <span className="tool-chevron"><Icon name={shown ? 'chevronDown' : 'chevronRight'} /></span>
        </button>
      </div>
      {shown && items.map((item, n) => <Row key={item.id} item={item} at={first + n} editable={false} hit={hit === first + n} onRetry={onRetry} />)}
    </>
  );
}

function itemText(i: Item): string {
  switch (i.kind) {
    case 'user': return `${i.text}\n${i.files.join('\n')}`;
    case 'assistant': return i.text;
    case 'tool': return `${i.name} ${i.label}`;
    default: return i.text;
  }
}

/** Ctrl+F: a search box over the conversation that steps through the messages containing the text. */
function FindBar({ items, onHit }: { items: Item[]; onHit: (at: number | null) => void }) {
  const epoch = useUi((u) => u.findEpoch);
  const [query, setQuery] = useState('');
  const [pos, setPos] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const q = query.trim().toLowerCase();
  const hits = useMemo(() => {
    if (!q) return [];
    const out: number[] = [];
    items.forEach((it, n) => { if (itemText(it).toLowerCase().includes(q)) out.push(n); });
    return out;
  }, [q, items.length]); // eslint-disable-line
  // Start from the newest match, where the reader most likely is.
  useEffect(() => setPos(Math.max(0, hits.length - 1)), [hits]);
  useEffect(() => onHit(hits[pos] ?? null), [hits, pos]); // eslint-disable-line
  useEffect(() => { input.current?.focus(); input.current?.select(); }, [epoch]);
  const close = () => {
    ui.set({ findOpen: false });
    onHit(null);
    focusComposer();
  };
  const step = (by: number) => hits.length && setPos((pos + by + hits.length) % hits.length);
  return (
    <div className="find-bar">
      <Icon name="search" />
      <input ref={input} placeholder="Find in chat" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => {
        if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey ? -1 : 1); }
        if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
      }} />
      <span className="small dim find-count">{!q ? '' : hits.length ? `${pos + 1} of ${hits.length}` : 'No matches'}</span>
      <IconButton icon="chevronUp" tip="Previous (Shift+Enter)" size={24} onClick={() => step(-1)} />
      <IconButton icon="chevronDown" tip="Next (Enter)" size={24} onClick={() => step(1)} />
      <IconButton icon="close" tip="Close (Esc)" size={24} onClick={close} />
    </div>
  );
}

const SPLASH = ['Building in', 'Shipping from', 'Cooking in', 'Hacking on', 'Debugging', 'Vibing in', "What's next in", 'Tinkering with', 'Refactoring', 'Fixing bugs in', 'Polishing', 'Dreaming up features for'];

/** "< Building in project? >" types itself out; clicking the project's name switches project. */
function EmptyState() {
  const chat = useView('chat');
  const projects = useView('projects');
  const { running } = useView('run');
  const phrase = useMemo(() => SPLASH[Math.floor(Math.random() * SPLASH.length)], [chat.epoch]); // eslint-disable-line
  const parts: [string, string][] = [['<', 'accent'], [` ${phrase} `, 'strong'], [chat.project, 'name'], ['?', 'strong'], ['>', 'accent']];
  const total = parts.reduce((n, [t]) => n + t.length, 0);
  const [shown, setShown] = useState(0);
  useEffect(() => {
    setShown(0);
    let n = 0;
    let timer = 0;
    const start = window.setTimeout(() => {
      timer = window.setInterval(() => {
        n += 1;
        setShown(n);
        if (n >= total) clearInterval(timer);
      }, 55);
    }, 300);
    return () => { clearTimeout(start); clearInterval(timer); };
  }, [phrase, total]);
  let left = shown;
  return (
    <div className="empty">
      <ScoutLogo size={48} />
      <div className={`splash ${shown >= total ? 'typed' : ''}`}>
        {/* The full line holds the space, so that nothing moves while it types. */}
        <span className="splash-size" aria-hidden>{parts.map(([t]) => t).join('')}</span>
        <span className="splash-text">
          {parts.map(([t, kind], i) => {
            const part = t.slice(0, Math.max(0, left));
            left -= t.length;
            if (kind !== 'name') return <span key={i} className={kind}>{part}</span>;
            return (
              <Popover key={i} width={240} trigger={(_, toggle) => <button type="button" className="splash-name" disabled={running} title="Switch project" onClick={toggle}>{part}</button>}>
                {(close) => (
                  <>
                    {projects.map((p) => (
                      <MenuItem key={p.path} selected={p.path === chat.cwd} onClick={() => { close(); if (p.path !== chat.cwd) act('changeProject', { dir: p.path }); }}>{p.name}</MenuItem>
                    ))}
                    {projects.length > 0 && <hr />}
                    <MenuItem icon="plus" onClick={() => { close(); addProject(); }}>New project</MenuItem>
                  </>
                )}
              </Popover>
            );
          })}
          <span className="splash-caret" />
        </span>
      </div>
    </div>
  );
}

type Mark = { at: number; id: number; user: boolean; text: string };

/** Stands in for the scrollbar: a tick for each message down the right edge, the one being read lit. Pointing at a tick
 *  shows the start of its message, and clicking goes to it. */
const MessageRail = memo(function MessageRail({ items, scroller, reveal }: { items: Item[]; scroller: RefObject<HTMLDivElement | null>; reveal: (at: number, block: ScrollLogicalPosition) => void }) {
  const marks = useMemo(() => {
    const out: Mark[] = [];
    items.forEach((it, at) => {
      if (it.kind === 'user') out.push({ at, id: it.id, user: true, text: it.text || it.files.map((f) => f.split(/[\\/]/).pop()).join(', ') });
      else if (it.kind === 'assistant' && it.text.trim()) out.push({ at, id: it.id, user: false, text: it.text });
    });
    return out;
  }, [items]);
  const [current, setCurrent] = useState(-1);
  const [peek, setPeek] = useState<{ mark: Mark; top: number } | null>(null);
  const rail = useRef<HTMLDivElement>(null);

  // The message being read: the last one that starts above the middle of what's showing, or the last of all at the bottom.
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    let frame = 0;
    const find = () => {
      frame = 0;
      let now = marks.length - 1;
      if (el.scrollHeight - el.scrollTop - el.clientHeight > 4) {
        const middle = el.getBoundingClientRect().top + el.clientHeight / 2;
        now = -1;
        for (const [n, m] of marks.entries()) {
          const row = el.querySelector(`[data-at="${m.at}"]`);
          if (!row || row.getBoundingClientRect().top > middle) break;
          now = n;
        }
        now = Math.max(now, 0);
      }
      setCurrent(now);
    };
    const queue = () => { frame ||= requestAnimationFrame(find); };
    find();
    el.addEventListener('scroll', queue, { passive: true });
    const observer = new ResizeObserver(queue);
    observer.observe(el);
    return () => {
      el.removeEventListener('scroll', queue);
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [marks, scroller]);

  if (marks.length < 2) return null;
  return (
    <div className="msg-rail" ref={rail} onMouseLeave={() => setPeek(null)}>
      {marks.map((m, n) => (
        <button
          key={m.id}
          type="button"
          className={`msg-tick ${m.user ? 'user' : ''} ${n === current ? 'on' : ''}`}
          aria-label={`Go to ${m.user ? 'your message' : "Scout's reply"}: ${m.text.slice(0, 80)}`}
          onMouseEnter={(e) => setPeek({ mark: m, top: e.currentTarget.offsetTop + e.currentTarget.offsetHeight / 2 })}
          onFocus={(e) => setPeek({ mark: m, top: e.currentTarget.offsetTop + e.currentTarget.offsetHeight / 2 })}
          onBlur={() => setPeek(null)}
          onClick={() => reveal(m.at, 'start')}
        ><i /></button>
      ))}
      {peek && (
        <div className="msg-peek" style={{ top: Math.max(0, Math.min(peek.top, (rail.current?.clientHeight ?? 0))) }}>
          <div className="msg-peek-who small"><b className="strong">{peek.mark.user ? 'You' : 'Scout'}</b></div>
          <div className="msg-peek-text">{peek.mark.text.trim().slice(0, 400)}</div>
        </div>
      )}
    </div>
  );
});

type AssistantItem = Extract<Item, { kind: 'assistant' }>;
const modelOf = (it: AssistantItem): ModelRef | null => (it.model ? { key: it.model, provider: it.provider ?? 'scoutcode' } : null);

/** Which replies name their model (the last one of each finished turn), and where the model changed from the reply
 *  before (the new model, keyed by the index it changed at). */
function modelMarks(items: Item[], running: boolean) {
  const modelAt = new Set<number>();
  const switchAt = new Map<number, ModelRef>();
  let last: ModelRef | null = null;
  let lastInTurn = -1;
  for (let i = 0; i < items.length; i++) {
    const it = items[i];
    if (it.kind === 'user') {
      if (lastInTurn >= 0) modelAt.add(lastInTurn);
      lastInTurn = -1;
      continue;
    }
    if (it.kind !== 'assistant') continue;
    const m = modelOf(it);
    if (!m) continue;
    if (last && (last.key !== m.key || last.provider !== m.provider)) switchAt.set(i, m);
    last = m;
    lastInTurn = i;
  }
  // The turn still being written names its model in the working line instead, until it ends.
  if (lastInTurn >= 0 && !running) modelAt.add(lastInTurn);
  return { modelAt, switchAt };
}

/** A line across the chat where a reply came from a different model than the one before it. */
function ModelSwitch({ to }: { to: ModelRef }) {
  const name = useModelName(to);
  const provLogo = to.provider ? logo(to.provider) : '';
  return (
    <div className="model-switch tiny dim" role="separator">
      <span className="model-switch-line" />
      <span className="model-switch-label">
        {provLogo && <img className="logo" src={provLogo} width={12} height={12} alt="" />}
        Switched to {name}
      </span>
      <span className="model-switch-line" />
    </div>
  );
}

const WORKING_VERBS = [
  'Analyzing', 'Reasoning', 'Evaluating', 'Synthesizing', 'Processing', 'Inspecting', 'Computing', 'Resolving',
  'Assessing', 'Correlating', 'Deliberating', 'Formulating', 'Interpreting', 'Mapping', 'Parsing', 'Structuring',
  'Validating', 'Calibrating', 'Compiling', 'Indexing', 'Examining', 'Deducing', 'Tracing', 'Weighing',
];
const VERB_MS = 3200;
/** How many items a chat draws when opened, and how many more each time the reader scrolls near the top. */
const WINDOW = 150;
const STEP = 100;

/** A word that softly crossfades into the next whenever `text` changes. */
function SoftWord({ text }: { text: string }) {
  return <span key={text} className="working-word shimmer">{text}</span>;
}

function elapsed(ms: number) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
}

/** Under the last message while Scout works, so a slow model still shows it's busy. Laid out like a tool row (the dot
 *  in the tools' icon slot), so it sits in the same place whether or not any tool has run yet: a word that fades into
 *  the next, how long the turn has run, and the model it runs on. */
function WorkingIndicator() {
  const { status } = useView('run');
  const permission = useView('permission');
  const plan = useView('plan');
  const questions = useView('questions');
  const model = useView('model');
  const modelName = useModelName(model.key ? { key: model.key, provider: model.provider } : null);
  const waiting = !!permission || plan !== null || !!questions;
  const [since] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  const [verb, setVerb] = useState(() => Math.floor(Math.random() * WORKING_VERBS.length));
  useEffect(() => {
    const clock = window.setInterval(() => setNow(Date.now()), 1000);
    const words = window.setInterval(() => setVerb((v) => (v + 1 + Math.floor(Math.random() * (WORKING_VERBS.length - 1))) % WORKING_VERBS.length), VERB_MS);
    return () => { clearInterval(clock); clearInterval(words); };
  }, []);
  const word = waiting ? 'Awaiting input' : WORKING_VERBS[verb];
  return (
    <div className="working-indicator" aria-live="polite">
      <SoftWord text={word} />
      <span className="working-meta tiny dim ellipsis">
        {elapsed(now - since)}
        {modelName && <> · {modelName}</>}
        {status && !waiting && <> · {status}</>}
      </span>
    </div>
  );
}

export function Transcript() {
  const items = useCore((s) => s.items);
  const { running } = useView('run');
  const findOpen = useUi((u) => u.findOpen);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [hit, setHit] = useState<number | null>(null);
  const chatEpoch = useView('chat').epoch;
  const [retryTarget, setRetryTarget] = useState<{ at: number; text: string } | null>(null);

  const handleRetry = useCallback((at: number) => {
    const all = core.get()?.items ?? [];
    const target = all[at];
    if (!target) return;
    if (target.kind === 'user') {
      setRetryTarget({ at, text: target.text });
    } else if (target.kind === 'assistant') {
      for (let i = at - 1; i >= 0; i--) {
        if (all[i]?.kind === 'user') {
          const u = all[i] as Extract<Item, { kind: 'user' }>;
          setRetryTarget({ at: i, text: u.text });
          return;
        }
      }
    }
  }, []);

  // Only the latest items are drawn: a chat with thousands of messages and tool calls made the window run out of
  // memory when opened. Older ones are drawn as the reader scrolls up, or when find or the rail goes to one.
  // Keyed to the first item, so another chat (or a rewound or compacted one) starts again from its latest items, even
  // on the very first draw.
  const latest = Math.max(0, items.length - WINDOW);
  const [win, setWin] = useState<{ first: number | undefined; from: number }>(() => ({ first: items[0]?.id, from: latest }));
  const shownFrom = win.first === items[0]?.id ? Math.min(win.from, latest) : latest;
  const shownRef = useRef(shownFrom);
  shownRef.current = shownFrom;
  const setFrom = useCallback((from: number) => setWin({ first: core.get()?.items[0]?.id, from }), []);
  const anchor = useRef<{ height: number; top: number } | null>(null);
  const jump = useRef<{ at: number; block: ScrollLogicalPosition } | null>(null);
  const reveal = useCallback((at: number, block: ScrollLogicalPosition) => {
    const el = scroller.current?.querySelector(`[data-at="${at}"]`);
    if (el) return el.scrollIntoView({ block, behavior: block === 'start' ? 'smooth' : 'auto' });
    if (at >= shownRef.current) return;
    jump.current = { at, block };
    setFrom(Math.max(0, at - 20));
  }, [setFrom]);
  // Drawing older items above keeps what was on screen where it was.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && anchor.current) {
      el.scrollTop = anchor.current.top + (el.scrollHeight - anchor.current.height);
      anchor.current = null;
    }
    if (jump.current) {
      const { at, block } = jump.current;
      const row = el?.querySelector(`[data-at="${at}"]`);
      if (row) {
        jump.current = null;
        row.scrollIntoView({ block });
      }
    }
  }, [shownFrom]);
  const loadOlder = () => {
    const el = scroller.current;
    if (!el || shownFrom === 0 || anchor.current) return;
    anchor.current = { height: el.scrollHeight, top: el.scrollTop };
    setFrom(Math.max(0, shownFrom - STEP));
  };

  useEffect(() => { stick.current = true; }, [chatEpoch]);
  // Stays at the bottom while the reply grows, unless the reader has scrolled up.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current && hit === null) el.scrollTop = el.scrollHeight;
  });
  useEffect(() => {
    if (hit !== null) reveal(hit, 'center');
  }, [hit, reveal]);
  // Items off screen only guess their height until they've been drawn, so the bottom moves after a render too.
  const hasItems = items.length > 0;
  const hitting = useRef(false);
  hitting.current = hit !== null;
  useEffect(() => {
    const el = scroller.current;
    const content = el?.firstElementChild;
    if (!el || !content) return;
    const observer = new ResizeObserver(() => {
      if (stick.current && !hitting.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [hasItems]);

  // The model is named once a turn, under its last reply, and a divider marks where the model changed.
  const { modelAt, switchAt } = useMemo(() => modelMarks(items, running), [items, running]);
  const rows = useMemo(() => {
    const out: ReactNode[] = [];
    if (shownFrom > 0) out.push(<button key="older" type="button" className="button ghost small transcript-older" onClick={loadOlder}>Show earlier ({shownFrom} more)</button>);
    for (let i = shownFrom; i < items.length; ) {
      if (items[i].kind !== 'tool') {
        const sw = switchAt.get(i);
        if (sw) out.push(<ModelSwitch key={`switch-${items[i].id}`} to={sw} />);
        out.push(<Row key={items[i].id} item={items[i]} at={i} editable={!running} hit={hit === i} showModel={modelAt.has(i)} onRetry={handleRetry} />);
        i += 1;
        continue;
      }
      const start = i;
      while (i < items.length && items[i].kind === 'tool') i += 1;
      const run = items.slice(start, i) as ToolItem[];
      // The last run stays unfolded while Scout may still add to it.
      if (run.length < 2 || (i === items.length && running)) run.forEach((it, n) => out.push(<Row key={it.id} item={it} at={start + n} editable={false} hit={hit === start + n} onRetry={handleRetry} />));
      else out.push(<ToolGroup key={run[0].id} items={run} first={start} hit={hit} onRetry={handleRetry} />);
    }
    return out;
  }, [items, running, hit, handleRetry, modelAt, switchAt, shownFrom]); // eslint-disable-line

  if (items.length === 0) return <div className="transcript"><EmptyState /></div>;
  return (
    <div className="transcript-wrap">
      {findOpen && <FindBar items={core.get().items} onHit={setHit} />}
      <div className="transcript" ref={scroller} onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
        if (el.scrollTop < 400) loadOlder();
      }}>
        <div className="column">{rows}{running && <WorkingIndicator />}</div>
      </div>
      <MessageRail items={items} scroller={scroller} reveal={reveal} />
      {retryTarget && (
        <Modal onClose={() => setRetryTarget(null)}>
          <b className="strong big">Retry from this message?</b>
          {retryTarget.text && (
            <div className="ellipsis muted small" style={{ maxWidth: '100%' }}>
              {retryTarget.text.split('\n')[0]}
            </div>
          )}
          <p className="small dim">This will reset files back to where it was before this message and run it again.</p>
          <div className="row end">
            <button type="button" className="button ghost" onClick={() => setRetryTarget(null)}>Cancel</button>
            <button type="button" className="button primary" onClick={() => {
              act('resend', { at: retryTarget.at, text: retryTarget.text });
              setRetryTarget(null);
            }}>Retry</button>
          </div>
        </Modal>
      )}
    </div>
  );
}
