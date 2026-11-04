import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { act, openExternal, type Item, type ModelRef } from './api';
import { FallbackList, ModelPicker, useModelName } from './ModelMenu';
import { core, focusComposer, ui, useCore, useUi, useView } from './store';
import { CodeBlock, Icon, IconButton, MenuItem, Popover, SectionLabel, ShowDisplay, Spinner } from './widgets';
import { ScoutLogo } from './logo';

type ToolItem = Extract<Item, { kind: 'tool' }>;

export function toolTitle(name: string): string {
  const titles: Record<string, string> = {
    view: 'Read', write_file: 'Write', edit_file: 'Edit', multi_edit: 'Edit', apply_patch: 'Patch', list_dir: 'List', glob: 'Find', grep: 'Search', bash: 'Run',
    shell_output: 'Output', kill_shell: 'Stop', web_fetch: 'Fetch', web_search: 'Web search', todo_write: 'Todos', task: 'Task', present_plan: 'Plan', ask_questions: 'Ask', update_memory: 'Memory', generate_image: 'Image',
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

function UserMessage({ item, at, editable }: { item: Extract<Item, { kind: 'user' }>; at: number; editable: boolean }) {
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
      {editable && <IconButton icon="pencil" tip="Edit and send again" size={26} className="edit-pencil" onClick={() => setDraft(item.text)} />}
      <div className="user-bubble">
        {item.text && <div className="pre-wrap">{item.text}</div>}
        {item.files.length > 0 && <div className="chips">{item.files.map((f) => <FileChip key={f} path={f} />)}</div>}
      </div>
    </div>
  );
}

function Assistant({ item }: { item: Extract<Item, { kind: 'assistant' }> }) {
  const [show, setShow] = useState(false);
  const latest = useMemo(() => item.reasoning.trim().split('\n').reverse().find((l) => l.trim())?.trim() ?? '', [item.reasoning]);
  return (
    <div className="assistant">
      {item.reasoning && (
        <>
          <button type="button" className="reasoning-head" onClick={() => setShow(!show)}>
            <Icon name={show ? 'chevronDown' : 'chevronRight'} />
            <b>Reasoning</b>
            {!show && <span className="dim ellipsis">{latest}</span>}
          </button>
          {show && <div className="reasoning pre-wrap">{item.reasoning}</div>}
        </>
      )}
      <Markdown text={item.text} />
    </div>
  );
}

function ToolRow({ item }: { item: ToolItem }) {
  const [opened, setOpened] = useState<boolean | null>(null);
  const r = item.result;
  const open = r ? opened ?? OPEN_BY_DEFAULT.has(item.name) : false;
  return (
    <div className="tool">
      <button type="button" className={`tool-row ${r ? 'can-open' : ''}`} onClick={() => r && setOpened(!open)}>
        {!r ? <span className="tool-status"><Spinner /></span> : <span className={`tool-status ${r.isError ? 'bad' : 'good'}`}><Icon name={r.isError ? 'close' : 'check'} size={12} /></span>}
        <b>{toolTitle(item.name)}</b>
        <span className="mono muted ellipsis tool-label">{item.label.split('\n')[0].slice(0, 160)}</span>
        {r?.summary && <span className={`tool-summary ellipsis ${r.isError ? 'red' : 'dim'}`}>· {r.summary}</span>}
        <span className="grow" />
        {r && <span className="tool-chevron"><Icon name={open ? 'chevronDown' : 'chevronRight'} /></span>}
      </button>
      {!r && item.progress && <div className="tool-progress mono dim">{item.progress}</div>}
      {r && open && <div className="tool-output"><ShowDisplay d={r.display} /></div>}
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
              <button type="button" className="button" onClick={() => ui.set({ settings: 'usage' })}>See usage</button>
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

/** The popup the setup card opens: a list for every model and one for the model that failed, which wins over it. */
function FallbackEditor({ failed, name, close }: { failed: ModelRef; name: string; close: () => void }) {
  const { fallbacks } = useView('cfg');
  const same = (m: ModelRef) => m.key === failed.key && m.provider === failed.provider;
  const [all, setAll] = useState<ModelRef[]>(fallbacks.all);
  const [own, setOwn] = useState<ModelRef[]>(fallbacks.models.find(([m]) => same(m))?.[1] ?? []);
  // The failed model can't stand in for itself.
  const usable = (l: ModelRef[]) => l.some((m) => !same(m));
  return (
    <div className="fallback-setup">
      <b>Fallback models</b>
      <span className="small dim">When a model fails, Scout carries the turn on with the next model in its list, then goes back to your model.</span>
      <SectionLabel>Every model</SectionLabel>
      <span className="small dim">Used by any model without a list of its own.</span>
      <FallbackList value={all} onChange={setAll} />
      <SectionLabel>Only {name}</SectionLabel>
      <span className="small dim">Used instead of the list above when {name} fails. Leave it empty to use that one.</span>
      <FallbackList value={own} onChange={setOwn} />
      <div className="credit-actions">
        <button type="button" className="button primary" disabled={!usable(own.length ? own : all)} onClick={() => { close(); act('setupFallbacks', { failed, all, own, off: false }); }}>Save and try again</button>
        <button type="button" className="button" onClick={close}>Cancel</button>
      </div>
      <span className="tiny dim">Change these any time in Settings &gt; Fallbacks.</span>
    </div>
  );
}

/** A model failed for the first time: offers to set up fallbacks, or to keep showing errors. */
function FallbackSetup({ item }: { item: Extract<Item, { kind: 'fallbackSetup' }> }) {
  const { running } = useView('run');
  const { fallbacks } = useView('cfg');
  const last = useCore((s) => s.items[s.items.length - 1]?.id === item.id);
  const failed: ModelRef = { key: item.key, provider: item.provider };
  const name = useModelName(failed);
  return (
    <div className="error-box credit-box">
      <Icon name="close" />
      <div className="stack grow">
        <span className="pre-wrap">{item.text}</span>
        {last && !running && !fallbacks.chosen && (
          <div className="credit-actions">
            <Popover side="top" className="fallback-popover" width={380} trigger={(_, toggle) => <button type="button" className="button primary" onClick={toggle}>Set up fallback models</button>}>
              {(close) => <FallbackEditor failed={failed} name={name} close={close} />}
            </Popover>
            <button type="button" className="button" onClick={() => act('setupFallbacks', { failed, all: [], own: [], off: true })}>No fallbacks, just show errors</button>
          </div>
        )}
      </div>
    </div>
  );
}

const Row = memo(function Row({ item, at, editable, hit }: { item: Item; at: number; editable: boolean; hit: boolean }) {
  let body: ReactNode;
  switch (item.kind) {
    case 'user':
      body = <UserMessage item={item} at={at} editable={editable} />;
      break;
    case 'assistant':
      body = <Assistant item={item} />;
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
    case 'fallbackSetup':
      body = <FallbackSetup item={item} />;
      break;
  }
  return <div className={`item item-${item.kind} ${hit ? 'hit' : ''}`} data-at={at}>{body}</div>;
});

/** A run of two or more tool calls the model has moved on from, under one header. */
function ToolGroup({ items, first, hit }: { items: ToolItem[]; first: number; hit: number | null }) {
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
      {shown && items.map((item, n) => <Row key={item.id} item={item} at={first + n} editable={false} hit={hit === first + n} />)}
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
                {(close) => projects.map((p) => (
                  <MenuItem key={p.path} selected={p.path === chat.cwd} onClick={() => { close(); if (p.path !== chat.cwd) act('changeProject', { dir: p.path }); }}>{p.name}</MenuItem>
                ))}
              </Popover>
            );
          })}
          <span className="splash-caret" />
        </span>
      </div>
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

  useEffect(() => { stick.current = true; }, [chatEpoch]);
  // Stays at the bottom while the reply grows, unless the reader has scrolled up.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current && hit === null) el.scrollTop = el.scrollHeight;
  });
  useEffect(() => {
    if (hit !== null) scroller.current?.querySelector(`[data-at="${hit}"]`)?.scrollIntoView({ block: 'center' });
  }, [hit]);
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

  const rows = useMemo(() => {
    const out: ReactNode[] = [];
    for (let i = 0; i < items.length; ) {
      if (items[i].kind !== 'tool') {
        out.push(<Row key={items[i].id} item={items[i]} at={i} editable={!running} hit={hit === i} />);
        i += 1;
        continue;
      }
      const start = i;
      while (i < items.length && items[i].kind === 'tool') i += 1;
      const run = items.slice(start, i) as ToolItem[];
      // The last run stays unfolded while Scout may still add to it.
      if (run.length < 2 || (i === items.length && running)) run.forEach((it, n) => out.push(<Row key={it.id} item={it} at={start + n} editable={false} hit={hit === start + n} />));
      else out.push(<ToolGroup key={run[0].id} items={run} first={start} hit={hit} />);
    }
    return out;
  }, [items, running, hit]);

  if (items.length === 0) return <div className="transcript"><EmptyState /></div>;
  return (
    <div className="transcript-wrap">
      {findOpen && <FindBar items={core.get().items} onHit={setHit} />}
      <div className="transcript" ref={scroller} onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
      }}>
        <div className="column">{rows}</div>
      </div>
    </div>
  );
}
