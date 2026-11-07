/**
 * Delta Docs: pages of rich text, like Notion or Google Docs, kept apart from issues. Pages nest under other pages and
 * show as a tree in the navigation. The editor is the browser's own contenteditable with Delta's commands on top: a
 * "/" menu, Markdown shortcuts as you type (# , - , 1. , [] , > , ```), a toolbar over selected text, and pictures
 * pasted, dropped or picked. What it writes is cleaned (images.ts) before it's saved and before it's shown.
 */
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type MouseEvent, type ReactNode, type RefObject } from 'react';
import { childPages, newId, pageText, pageTitle, type Doc, type Page } from './model';
import { Empty, useDelta, type DeltaCtx } from './Delta';
import { addImages, cleanHtml, imageFiles, imageSrc, pickImages, textToHtml } from './images';
import { ago, Avatar, Icon, Menu, MenuItem, openContextMenu, showImage, type CtxItem } from './ui';

// ---------- pages ----------

function ancestors(doc: Doc, id: string): Page[] {
  const out: Page[] = [];
  for (let at = doc.page[id]?.parentId ?? null, n = 0; at && doc.page[at] && n < 200; at = doc.page[at].parentId, n++) out.unshift(doc.page[at]);
  return out;
}

const isInside = (doc: Doc, id: string, maybeAncestor: string) => ancestors(doc, id).some((p) => p.id === maybeAncestor);

/** Makes a page at the end of its parent's sub-pages and opens it. */
export function newPage(c: DeltaCtx, parentId: string | null = null, data: Partial<Page> = {}): string | null {
  const last = childPages(c.s.doc, parentId).pop();
  const id = newId();
  if (!c.s.dispatch({ type: 'create', kind: 'page', id, data: { parentId, sortOrder: (last?.sortOrder ?? 0) + 1, ...data } })) return null;
  c.go({ view: 'doc', id });
  return id;
}

function movePage(c: DeltaCtx, id: string, target: string, zone: 'before' | 'after' | 'inside') {
  const { doc } = c.s;
  if (id === target || isInside(doc, target, id)) return;
  if (zone === 'inside') {
    const last = childPages(doc, target).filter((p) => p.id !== id).pop();
    c.s.dispatch({ type: 'update', kind: 'page', id, patch: { parentId: target, sortOrder: (last?.sortOrder ?? 0) + 1 } });
    return;
  }
  const parentId = doc.page[target].parentId ?? null;
  const sibs = childPages(doc, parentId).filter((p) => p.id !== id);
  const at = sibs.findIndex((p) => p.id === target);
  const a = zone === 'before' ? sibs[at - 1] : sibs[at];
  const b = zone === 'before' ? sibs[at] : sibs[at + 1];
  const order = a && b ? (a.sortOrder + b.sortOrder) / 2 : a ? a.sortOrder + 1 : b ? b.sortOrder - 1 : 1;
  c.s.dispatch({ type: 'update', kind: 'page', id, patch: { parentId, sortOrder: order } });
}

function deletePage(c: DeltaCtx, p: Page) {
  const kids = Object.values(c.s.doc.page).filter((x) => isInside(c.s.doc, x.id, p.id)).length;
  if (!confirm(`Delete "${pageTitle(p)}"${kids ? ` and its ${kids} sub-page${kids === 1 ? '' : 's'}` : ''}? This can't be undone.`)) return;
  const parent = p.parentId;
  c.s.dispatch({ type: 'delete', kind: 'page', id: p.id });
  if (c.route.view === 'doc' && (c.route.id === p.id || !c.s.doc.page[c.route.id])) c.go(parent && c.s.doc.page[parent] ? { view: 'doc', id: parent } : { view: 'docs' });
}

function duplicatePage(c: DeltaCtx, p: Page) {
  newPage(c, p.parentId, { title: p.title ? `${p.title} (copy)` : '', icon: p.icon, body: p.body, full: p.full, sortOrder: p.sortOrder + 0.5 });
}

export function pageMenu(c: DeltaCtx, p: Page): CtxItem[] {
  return [
    { label: 'Open', icon: <Icon name="doc" size={14} />, onClick: () => c.go({ view: 'doc', id: p.id }) },
    ...(c.openTab ? [{ label: 'Open in new tab', icon: <Icon name="plus" size={14} />, onClick: () => c.openTab?.({ view: 'doc', id: p.id }) }] : []),
    { label: 'Add a sub-page', icon: <Icon name="plus" size={14} />, onClick: () => newPage(c, p.id) },
    { label: 'Duplicate', icon: <Icon name="copy" size={14} />, onClick: () => duplicatePage(c, p) },
    ...(p.parentId ? [{ label: 'Move to the top level', icon: <Icon name="back" size={14} />, onClick: () => movePage(c, p.id, rootOf(c.s.doc, p).id, 'after') }] : []),
    { label: 'Copy as Markdown', icon: <Icon name="copy" size={14} />, onClick: () => navigator.clipboard?.writeText(toMarkdown(c.s.doc, p)) },
    'sep',
    { label: 'Delete', icon: <Icon name="trash" size={14} />, danger: true, onClick: () => deletePage(c, p) },
  ];
}

const rootOf = (doc: Doc, p: Page) => ancestors(doc, p.id)[0] ?? p;

function PageIcon({ p, size = 15 }: { p: Page; size?: number }) {
  return p.icon ? <span className="d-page-emoji" style={{ fontSize: size - 1, width: size, height: size }}>{p.icon}</span> : <Icon name="doc" size={size} />;
}

// ---------- the tree in the navigation ----------

const OPEN_KEY = 'delta.docs.open';

export function DocTree() {
  const c = useDelta();
  const { s, route } = c;
  const [open, setOpen] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem(OPEN_KEY) ?? '[]'));
    } catch {
      return new Set();
    }
  });
  const [drop, setDrop] = useState<{ id: string; zone: 'before' | 'after' | 'inside' } | null>(null);
  const toggle = (id: string, to?: boolean) => {
    const n = new Set(open);
    if (to ?? !n.has(id)) n.add(id);
    else n.delete(id);
    setOpen(n);
    try {
      localStorage.setItem(OPEN_KEY, JSON.stringify([...n]));
    } catch {}
  };
  // The open page's parents open with it.
  const here = route.view === 'doc' ? route.id : null;
  useEffect(() => {
    if (!here) return;
    const up = ancestors(s.doc, here).map((p) => p.id).filter((id) => !open.has(id));
    if (up.length) setOpen(new Set([...open, ...up]));
  }, [here]);

  const rows = (parent: string | null, depth: number): ReactNode[] =>
    childPages(s.doc, parent).flatMap((p) => {
      const kids = childPages(s.doc, p.id).length;
      const isOpen = open.has(p.id);
      const z = drop?.id === p.id ? drop.zone : null;
      const row = (
        <div
          key={p.id}
          role="button"
          tabIndex={0}
          className={`d-nav-item d-tree-row ${here === p.id ? 'on' : ''} ${z ? `drop-${z}` : ''}`}
          style={{ paddingLeft: 6 + depth * 14 }}
          draggable
          onClick={(e) => {
            if (c.openTab && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              c.openTab({ view: 'doc', id: p.id });
            } else {
              c.go({ view: 'doc', id: p.id });
            }
          }}
          onAuxClick={(e) => {
            if (e.button === 1 && c.openTab) {
              e.preventDefault();
              c.openTab({ view: 'doc', id: p.id });
            }
          }}
          onKeyDown={(e) => { if (e.key === 'Enter') c.go({ view: 'doc', id: p.id }); }}
          onContextMenu={(e) => openContextMenu(e, pageMenu(c, p))}
          onDragStart={(e) => { e.dataTransfer.setData('text/delta-page', p.id); e.dataTransfer.effectAllowed = 'move'; }}
          onDragOver={(e: DragEvent) => {
            if (!e.dataTransfer.types.includes('text/delta-page')) return;
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            const y = (e.clientY - r.top) / r.height;
            const zone = y < 0.28 ? 'before' : y > 0.72 ? 'after' : 'inside';
            if (drop?.id !== p.id || drop.zone !== zone) setDrop({ id: p.id, zone });
          }}
          onDragLeave={() => setDrop((d) => (d?.id === p.id ? null : d))}
          onDrop={(e) => {
            e.preventDefault();
            const id = e.dataTransfer.getData('text/delta-page');
            setDrop(null);
            if (!id || !z) return;
            movePage(c, id, p.id, z);
            if (z === 'inside') toggle(p.id, true);
          }}
          onDragEnd={() => setDrop(null)}
        >
          <span className={`d-tree-twist ${kids ? '' : 'none'}`} onClick={(e) => { e.stopPropagation(); toggle(p.id); }}>
            <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          </span>
          <PageIcon p={p} size={14} />
          <span className="d-grow d-ellipsis">{pageTitle(p)}</span>
          <span className="d-tree-acts">
            <button type="button" className="d-icon-btn tiny" title="More" onClick={(e) => { e.stopPropagation(); openContextMenu(e, pageMenu(c, p)); }}><Icon name="more" size={13} /></button>
            <button type="button" className="d-icon-btn tiny" title="Add a sub-page" onClick={(e) => { e.stopPropagation(); toggle(p.id, true); newPage(c, p.id); }}><Icon name="plus" size={13} /></button>
          </span>
        </div>
      );
      return [row, ...(isOpen ? rows(p.id, depth + 1) : [])];
    });

  const top = childPages(s.doc, null);
  return (
    <>
      <div className="d-section-label d-nav-label d-row">
        <button type="button" className="d-grow d-nav-label-btn" onClick={() => c.go({ view: 'docs' })}>Docs</button>
        <button type="button" className="d-icon-btn tiny" title="New doc" onClick={() => newPage(c)}><Icon name="plus" size={13} /></button>
      </div>
      {rows(null, 0)}
      {!top.length && <button type="button" className="d-nav-item d-dim" onClick={() => newPage(c)}><Icon name="plus" size={15} /><span className="d-grow">New doc</span></button>}
    </>
  );
}

// ---------- all docs ----------

export function DocsView() {
  const c = useDelta();
  const { s, v } = c;
  const [q, setQ] = useState('');
  const pages = useMemo(() => {
    const t = q.trim().toLowerCase();
    return Object.values(s.doc.page)
      .filter((p) => !t || pageTitle(p).toLowerCase().includes(t) || pageText(p.body).toLowerCase().includes(t))
      .sort((a, b) => b.updatedAt - a.updatedAt);
  }, [v, q]);
  const any = Object.keys(s.doc.page).length > 0;
  return (
    <div className="d-page">
      <header className="d-head">
        <h1>Docs</h1>
        <span className="d-grow" />
        {any && <input className="d-input small d-docs-search" placeholder="Search docs…" value={q} onChange={(e) => setQ(e.currentTarget.value)} />}
        <button type="button" className="d-btn primary small" onClick={() => newPage(c)}><Icon name="plus" size={13} />New doc</button>
      </header>
      {!any ? (
        <Empty icon="doc" title="No docs yet" text="Write specs, notes, plans and wikis for your team, with headings, lists, to-dos, code and pictures." action={<button type="button" className="d-btn primary" onClick={() => newPage(c)}>New doc</button>} />
      ) : (
        <div className="d-scroll d-grow">
          <div className="d-docs-grid">
            {pages.map((p) => {
              const path = ancestors(s.doc, p.id).map(pageTitle).join(' / ');
              const text = pageText(p.body).replace(/\s+/g, ' ').trim().slice(0, 160);
              return (
                <button
                  type="button"
                  key={p.id}
                  className="d-doc-card"
                  onClick={(e) => {
                    if (c.openTab && (e.ctrlKey || e.metaKey)) {
                      e.preventDefault();
                      c.openTab({ view: 'doc', id: p.id });
                    } else {
                      c.go({ view: 'doc', id: p.id });
                    }
                  }}
                  onAuxClick={(e) => {
                    if (e.button === 1 && c.openTab) {
                      e.preventDefault();
                      c.openTab({ view: 'doc', id: p.id });
                    }
                  }}
                  onContextMenu={(e) => openContextMenu(e, pageMenu(c, p))}
                >
                  <span className="d-doc-card-icon">{p.icon || <Icon name="doc" size={22} />}</span>
                  <b className="d-ellipsis">{pageTitle(p)}</b>
                  {path && <span className="d-tiny d-dim d-ellipsis">{path}</span>}
                  <span className="d-doc-card-text d-small d-muted">{text || <i className="d-dim">Empty</i>}</span>
                  <span className="d-row d-tiny d-dim"><Avatar name={p.updatedBy} size={14} />{p.updatedBy} · {ago(p.updatedAt)}</span>
                </button>
              );
            })}
            {!pages.length && <div className="d-menu-empty">No docs match “{q}”.</div>}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------- one doc ----------

const EMOJI = ['📄', '📝', '📌', '📚', '📖', '🗂️', '📋', '✅', '🎯', '🚀', '💡', '🔥', '⭐', '🧠', '🛠️', '⚙️', '🐛', '🔒', '📈', '📊', '🗓️', '⏱️', '💬', '📣', '🎨', '🧪', '🧩', '🌱', '🏁', '🏗️', '🔍', '❤️', '👋', '🙌', '🤝', '🎉', '☕', '🌍', '🏠', '📦', '💰', '🧾', '🔗', '🧭', '⚡', '🌙', '☀️', '✨'];

function EmojiGrid({ onPick }: { onPick: (e: string) => void }) {
  return <div className="d-emoji-grid">{EMOJI.map((e) => <button type="button" key={e} onClick={() => onPick(e)}>{e}</button>)}</div>;
}

export function DocView({ id }: { id: string }) {
  const c = useDelta();
  const { s, go } = c;
  const p = s.doc.page[id];
  const up = ancestors(s.doc, id);
  const kids = childPages(s.doc, id);
  const set = (patch: Partial<Page>) => s.dispatch({ type: 'update', kind: 'page', id, patch });
  const editor = useRef<EditorApi>(null);
  return (
    <div className="d-page">
      <header className="d-head d-doc-head">
        <nav className="d-crumbs d-row d-min0 d-grow">
          <button type="button" className="d-crumb" onClick={() => go({ view: 'docs' })}>Docs</button>
          {up.map((a) => (
            <Fragment key={a.id}>
              <span className="d-dim">/</span>
              <button type="button" className="d-crumb d-ellipsis" onClick={() => go({ view: 'doc', id: a.id })}>{a.icon && <span>{a.icon}</span>}{pageTitle(a)}</button>
            </Fragment>
          ))}
          <span className="d-dim">/</span>
          <span className="d-crumb here d-ellipsis">{p.icon && <span>{p.icon}</span>}{pageTitle(p)}</span>
        </nav>
        <span className="d-tiny d-dim d-nowrap" title={new Date(p.updatedAt).toLocaleString()}>Edited {ago(p.updatedAt)}{p.updatedBy !== s.me ? ` by ${p.updatedBy}` : ''}</span>
        <Menu align="end" width={230} trigger={(open, toggle) => <button type="button" className={`d-icon-btn ${open ? 'on' : ''}`} onClick={toggle} aria-label="Page options"><Icon name="more" /></button>}>
          {(close) => (
            <>
              <MenuItem selected={p.full} onClick={() => { close(); set({ full: !p.full }); }}><Icon name="expand" size={14} />Full width</MenuItem>
              <MenuItem onClick={() => { close(); newPage(c, id); }}><Icon name="plus" size={14} />Add a sub-page</MenuItem>
              <MenuItem onClick={() => { close(); duplicatePage(c, p); }}><Icon name="copy" size={14} />Duplicate</MenuItem>
              <MenuItem onClick={() => { close(); navigator.clipboard?.writeText(toMarkdown(s.doc, p)); }}><Icon name="copy" size={14} />Copy as Markdown</MenuItem>
              <MenuItem onClick={() => { close(); download(`${pageTitle(p).replace(/[\\/:*?"<>|]+/g, ' ').trim() || 'Untitled'}.md`, toMarkdown(s.doc, p)); }}><Icon name="download" size={14} />Download Markdown</MenuItem>
              <hr />
              <MenuItem danger onClick={() => { close(); deletePage(c, p); }}><Icon name="trash" size={14} />Delete</MenuItem>
            </>
          )}
        </Menu>
      </header>
      <div className="d-scroll d-grow d-doc-scroll" onMouseDown={(e) => {
        // A click below the text puts the caret at its end, as in a word processor.
        if (e.target === e.currentTarget) { e.preventDefault(); editor.current?.focusEnd(); }
      }}>
        <article className={`d-doc ${p.full ? 'full' : ''}`}>
          <div className="d-doc-top">
            {p.icon ? (
              <Menu width={292} trigger={(_, toggle) => <button type="button" className="d-doc-icon" onClick={toggle} title="Change icon">{p.icon}</button>}>
                {(close) => (
                  <>
                    <EmojiGrid onPick={(e) => { close(); set({ icon: e }); }} />
                    <hr />
                    <MenuItem onClick={() => { close(); set({ icon: '' }); }}><Icon name="close" size={14} />Remove icon</MenuItem>
                  </>
                )}
              </Menu>
            ) : (
              <div className="d-doc-addons">
                <Menu width={292} trigger={(_, toggle) => <button type="button" className="d-btn ghost small" onClick={toggle}><Icon name="smile" size={14} />Add icon</button>}>
                  {(close) => <EmojiGrid onPick={(e) => { close(); set({ icon: e }); }} />}
                </Menu>
              </div>
            )}
          </div>
          <TitleField page={p} onEnter={() => editor.current?.focusStart()} />
          <PageEditor key={id} page={p} api={editor} />
          {kids.length > 0 && (
            <div className="d-doc-kids">
              <div className="d-section-label">Sub-pages</div>
              {kids.map((k) => (
                <button
                  type="button"
                  key={k.id}
                  className="d-doc-kid"
                  onClick={(e) => {
                    if (c.openTab && (e.ctrlKey || e.metaKey)) {
                      e.preventDefault();
                      c.openTab({ view: 'doc', id: k.id });
                    } else {
                      go({ view: 'doc', id: k.id });
                    }
                  }}
                  onAuxClick={(e) => {
                    if (e.button === 1 && c.openTab) {
                      e.preventDefault();
                      c.openTab({ view: 'doc', id: k.id });
                    }
                  }}
                  onContextMenu={(e) => openContextMenu(e, pageMenu(c, k))}
                >
                  <PageIcon p={k} />
                  <span className="d-ellipsis">{pageTitle(k)}</span>
                </button>
              ))}
            </div>
          )}
        </article>
      </div>
    </div>
  );
}

function download(name: string, text: string) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'text/markdown' }));
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

function TitleField({ page, onEnter }: { page: Page; onEnter: () => void }) {
  const { s } = useDelta();
  const [draft, setDraft] = useState(page.title);
  const ref = useRef<HTMLTextAreaElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const save = (t: string) => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    if (t !== s.doc.page[page.id]?.title) s.dispatch({ type: 'update', kind: 'page', id: page.id, patch: { title: t } });
  };
  // Someone else's rename shows unless this field is being typed in.
  useEffect(() => {
    if (document.activeElement !== ref.current) setDraft(page.title);
  }, [page.title]);
  useEffect(() => () => { if (timer.current) { clearTimeout(timer.current); save(ref.current?.value ?? draft); } }, []);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;

    const adjust = () => {
      if (typeof CSS !== 'undefined' && CSS.supports?.('field-sizing', 'content')) {
        el.style.height = '';
        return;
      }
      el.style.height = 'auto';
      el.style.height = `${el.scrollHeight}px`;
    };

    adjust();

    if (typeof document !== 'undefined' && 'fonts' in document) {
      document.fonts.ready.then(adjust);
    }

    let lastWidth = el.clientWidth;
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver((entries) => {
      for (const entry of entries) {
        const w = entry.contentRect.width;
        if (w && w !== lastWidth) {
          lastWidth = w;
          adjust();
        }
      }
    }) : null;
    ro?.observe(el);

    return () => ro?.disconnect();
  }, [draft]);
  // A new page starts in its title.
  useEffect(() => {
    if (!page.title && !page.body && Date.now() - page.createdAt < 3000) ref.current?.focus();
  }, []);
  return (
    <textarea
      ref={ref}
      rows={1}
      className="d-doc-title"
      placeholder="Untitled"
      value={draft}
      maxLength={300}
      onChange={(e) => {
        const t = e.currentTarget.value.replace(/\n/g, '');
        setDraft(t);
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => save(t), 400);
      }}
      onBlur={() => save(draft)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || (e.key === 'ArrowDown' && e.currentTarget.selectionStart === draft.length)) {
          e.preventDefault();
          save(draft);
          onEnter();
        }
      }}
    />
  );
}

// ---------- the editor ----------

interface EditorApi { focusStart(): void; focusEnd(): void }

type BlockType = 'p' | 'h1' | 'h2' | 'h3' | 'ul' | 'ol' | 'todo' | 'blockquote' | 'aside' | 'pre' | 'hr' | 'image' | 'page';

const COMMANDS: { type: BlockType; label: string; hint: string; icon: ReactNode; keys: string }[] = [
  { type: 'p', label: 'Text', hint: 'Plain writing', icon: <b className="d-cmd-glyph">T</b>, keys: 'text paragraph plain' },
  { type: 'h1', label: 'Heading 1', hint: 'Big section heading', icon: <b className="d-cmd-glyph">H1</b>, keys: 'heading h1 title' },
  { type: 'h2', label: 'Heading 2', hint: 'Medium heading', icon: <b className="d-cmd-glyph">H2</b>, keys: 'heading h2 subtitle' },
  { type: 'h3', label: 'Heading 3', hint: 'Small heading', icon: <b className="d-cmd-glyph">H3</b>, keys: 'heading h3' },
  { type: 'ul', label: 'Bulleted list', hint: 'A simple list', icon: <Icon name="list" size={15} />, keys: 'bullet list ul unordered' },
  { type: 'ol', label: 'Numbered list', hint: 'A list with numbers', icon: <b className="d-cmd-glyph">1.</b>, keys: 'numbered list ol ordered' },
  { type: 'todo', label: 'To-do list', hint: 'Things to tick off', icon: <Icon name="check" size={15} />, keys: 'todo to-do checkbox task check' },
  { type: 'blockquote', label: 'Quote', hint: 'A quotation', icon: <b className="d-cmd-glyph">“</b>, keys: 'quote blockquote' },
  { type: 'aside', label: 'Callout', hint: 'Make something stand out', icon: <b className="d-cmd-glyph">💡</b>, keys: 'callout note info aside' },
  { type: 'pre', label: 'Code', hint: 'A block of code', icon: <b className="d-cmd-glyph">{'</>'}</b>, keys: 'code pre snippet' },
  { type: 'hr', label: 'Divider', hint: 'A line between sections', icon: <b className="d-cmd-glyph">—</b>, keys: 'divider hr line rule separator' },
  { type: 'image', label: 'Image', hint: 'Upload a picture', icon: <Icon name="image" size={15} />, keys: 'image picture photo screenshot upload img' },
  { type: 'page', label: 'Sub-page', hint: 'A page inside this one', icon: <Icon name="doc" size={15} />, keys: 'page subpage sub-page doc' },
];

const BLOCK_TAGS = /^(P|H1|H2|H3|LI|BLOCKQUOTE|PRE|ASIDE)$/;
const PLACEHOLDERS: Record<string, string> = { H1: 'Heading 1', H2: 'Heading 2', H3: 'Heading 3', LI: 'List', BLOCKQUOTE: 'Quote', ASIDE: 'Callout', PRE: 'Code' };

const exec = (cmd: string, arg?: string) => document.execCommand(cmd, false, arg);
const esc = (t: string) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

function PageEditor({ page, api }: { page: Page; api: RefObject<EditorApi | null> }) {
  const c = useDelta();
  const { s } = c;
  const ref = useRef<HTMLDivElement>(null);
  /** The body as last loaded or saved, so our own saves don't reload the editor. */
  const seen = useRef(page.body);
  const saved = useRef('');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [tool, setTool] = useState<{ x: number; y: number; below: boolean } | null>(null);
  const [link, setLink] = useState<{ range: Range; value: string; x: number; y: number; below: boolean } | null>(null);
  const [slash, setSlash] = useState<{ x: number; y: number; q: string; at: number } | null>(null);
  const [, bump] = useState(0);

  const el = () => ref.current!;

  const serialize = () => {
    const e = el();
    if (!e.textContent?.trim() && !e.querySelector('img, hr')) return '';
    return cleanHtml(e.innerHTML);
  };

  const save = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    if (!ref.current) return;
    const html = serialize();
    if (html === saved.current) return;
    saved.current = html;
    seen.current = html;
    s.dispatch({ type: 'update', kind: 'page', id: page.id, patch: { body: html } });
  };
  const schedule = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(save, 600);
  };

  const resolved = useRef(new WeakMap<HTMLImageElement, string>());
  const resolve = () => {
    for (const img of Array.from(el().querySelectorAll<HTMLImageElement>('img[data-img]'))) {
      const id = img.getAttribute('data-img')!;
      if (resolved.current.get(img) === id) continue;
      const src = imageSrc(s.doc, id);
      img.classList.toggle('missing', !src);
      if (src) {
        img.src = src;
        resolved.current.set(img, id);
      }
    }
    // Links to other docs show their page's name as it is now.
    for (const a of Array.from(el().querySelectorAll<HTMLAnchorElement>('a[href^="#page:"]'))) {
      const p = s.doc.page[a.getAttribute('href')!.slice(6)];
      const name = p ? `${p.icon ? `${p.icon} ` : ''}${pageTitle(p)}` : 'Deleted page';
      if (a.textContent !== name) a.textContent = name;
    }
  };

  const load = (body: string, keepCaret = false) => {
    const e = el();
    const at = keepCaret ? caretOffset(e) : null;
    e.innerHTML = cleanHtml(body) || '<p><br></p>';
    saved.current = serialize();
    resolve();
    if (at != null) setCaretOffset(e, at);
  };

  useLayoutEffect(() => {
    try {
      exec('defaultParagraphSeparator', 'p');
    } catch {}
    load(page.body);
    api.current = {
      focusStart: () => {
        el().focus();
        const r = document.createRange();
        r.setStart(el().firstChild ?? el(), 0);
        r.collapse(true);
        const sel = getSelection()!;
        sel.removeAllRanges();
        sel.addRange(r);
      },
      focusEnd: () => {
        const e = el();
        let last = e.lastElementChild as HTMLElement | null;
        // Ending on a picture, a divider, a list or code leaves no plain line to type in: add one.
        if (!last || !/^(P|H1|H2|H3|BLOCKQUOTE|ASIDE)$/.test(last.tagName) || last.querySelector('img')) {
          last = document.createElement('p');
          last.innerHTML = '<br>';
          e.appendChild(last);
        }
        e.focus();
        const r = document.createRange();
        r.selectNodeContents(last);
        r.collapse(false);
        const sel = getSelection()!;
        sel.removeAllRanges();
        sel.addRange(r);
      },
    };
    return () => {
      if (timer.current) save();
    };
  }, []);

  // Others' changes, and pictures that arrive after the text that shows them.
  useEffect(() => {
    if (page.body !== seen.current && !timer.current) {
      seen.current = page.body;
      load(page.body, document.activeElement === ref.current);
    } else resolve();
  }, [c.v]);

  // ----- where the caret is -----

  const block = (): HTMLElement | null => {
    const sel = getSelection();
    if (!sel?.rangeCount) return null;
    let n: Node | null = sel.anchorNode;
    if (!n || !el().contains(n)) return null;
    while (n && n !== el()) {
      if (n.nodeType === 1 && BLOCK_TAGS.test((n as HTMLElement).tagName)) return n as HTMLElement;
      n = n.parentNode;
    }
    return null;
  };
  const textBefore = (b: HTMLElement) => {
    const sel = getSelection()!;
    const r = document.createRange();
    r.setStart(b, 0);
    r.setEnd(sel.anchorNode!, sel.anchorOffset);
    return r.toString();
  };
  const caretRect = (): DOMRect | null => {
    const sel = getSelection();
    if (!sel?.rangeCount) return null;
    const r = sel.getRangeAt(0).cloneRange();
    const rect = r.getClientRects()[0] ?? r.getBoundingClientRect();
    if (rect && (rect.width || rect.height)) return rect;
    return block()?.getBoundingClientRect() ?? null;
  };
  const inside = (tag: string) => {
    const sel = getSelection();
    let n: Node | null = sel?.anchorNode ?? null;
    while (n && n !== ref.current) {
      if (n.nodeType === 1 && (n as HTMLElement).matches(tag)) return n as HTMLElement;
      n = n.parentNode;
    }
    return null;
  };

  // ----- turning a block into another -----

  const turn = async (t: BlockType) => {
    el().focus();
    if (t === 'image') {
      const sel = getSelection();
      const range = sel?.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
      const files = await pickImages();
      if (files.length) insertImages(files, range);
      return;
    }
    if (t === 'page') {
      const id = newId();
      const last = childPages(s.doc, page.id).pop();
      if (s.dispatch({ type: 'create', kind: 'page', id, data: { parentId: page.id, sortOrder: (last?.sortOrder ?? 0) + 1 } })) {
        exec('insertHTML', `<a href="#page:${id}">Untitled</a>&nbsp;`);
        save();
        c.go({ view: 'doc', id });
      }
      return;
    }
    if (t === 'hr') {
      exec('insertHorizontalRule');
      const hr = el().querySelectorAll('hr');
      for (const h of Array.from(hr)) if (!h.nextElementSibling) { const p = document.createElement('p'); p.innerHTML = '<br>'; h.after(p); }
      schedule();
      return;
    }
    const li = inside('li');
    const list = li?.parentElement;
    const isTodo = list?.getAttribute('data-type') === 'todo';
    if (t === 'ul' || t === 'ol' || t === 'todo') {
      const want = t === 'ol' ? 'OL' : 'UL';
      if (!list || list.tagName !== want) exec(t === 'ol' ? 'insertOrderedList' : 'insertUnorderedList');
      const ul = inside(want.toLowerCase());
      if (ul) {
        if (t === 'todo') {
          ul.setAttribute('data-type', 'todo');
          for (const x of Array.from(ul.children)) if (!x.hasAttribute('data-checked')) x.setAttribute('data-checked', 'false');
        } else if (isTodo || ul.getAttribute('data-type')) {
          ul.removeAttribute('data-type');
          for (const x of Array.from(ul.children)) x.removeAttribute('data-checked');
        }
      }
      schedule();
      return;
    }
    // Out of a list first, then into the block.
    if (list) exec(list.tagName === 'OL' ? 'insertOrderedList' : 'insertUnorderedList');
    exec('formatBlock', `<${t}>`);
    if (t === 'aside') inside('aside')?.setAttribute('data-icon', '💡');
    schedule();
  };

  const insertImages = async (files: File[], range: Range | null) => {
    const ids = await addImages(s, files);
    if (!ids.length) return;
    el().focus();
    const sel = getSelection()!;
    if (range && el().contains(range.startContainer)) {
      sel.removeAllRanges();
      sel.addRange(range);
    }
    exec('insertHTML', ids.map((id) => `<img data-img="${id}">`).join(''));
    resolve();
    save();
  };

  // ----- after every change -----

  const tidy = () => {
    const e = el();
    if (!e.firstChild || e.innerHTML === '<br>') {
      e.innerHTML = '<p><br></p>';
      const r = document.createRange();
      r.setStart(e.firstChild!, 0);
      r.collapse(true);
      getSelection()!.removeAllRanges();
      getSelection()!.addRange(r);
    }
    for (const ul of Array.from(e.querySelectorAll('ul[data-type="todo"] ul:not([data-type])'))) ul.setAttribute('data-type', 'todo');
    for (const li of Array.from(e.querySelectorAll('ul[data-type="todo"] > li:not([data-checked])'))) li.setAttribute('data-checked', 'false');
    // A new item made by Enter copies the one before it, tick and all.
    const cur = inside('li');
    if (cur && !cur.textContent && cur.getAttribute('data-checked') === 'true') cur.setAttribute('data-checked', 'false');
    for (const ul of Array.from(e.querySelectorAll('ul:not([data-type="todo"]) > li[data-checked], ol > li[data-checked]'))) ul.removeAttribute('data-checked');
  };

  const placeholder = () => {
    const e = ref.current;
    if (!e) return;
    for (const x of Array.from(e.querySelectorAll('[data-ph]'))) x.removeAttribute('data-ph');
    const only = e.children.length === 1 && e.firstElementChild!.tagName === 'P' && !e.textContent && !e.querySelector('img');
    if (only && document.activeElement !== e) {
      e.firstElementChild!.setAttribute('data-ph', 'Start writing, or press “/” for headings, lists, images and more…');
      return;
    }
    const b = document.activeElement === e ? block() : null;
    if (b && !b.textContent && !b.querySelector('img')) b.setAttribute('data-ph', PLACEHOLDERS[b.tagName] ?? 'Type “/” for commands');
  };

  const findSlash = () => {
    const sel = getSelection();
    const b = block();
    if (!sel?.isCollapsed || !b || b.tagName === 'PRE') return setSlash(null);
    const m = /(?:^|\s)\/([a-z0-9-]{0,20})$/i.exec(textBefore(b));
    if (!m) return setSlash(null);
    const r = caretRect();
    if (!r) return setSlash(null);
    setSlash((old) => ({ x: r.left, y: r.bottom, q: m[1], at: old && old.q === m[1] ? old.at : 0 }));
  };

  // ----- the selection toolbar, and the placeholder that follows the caret -----

  useEffect(() => {
    const onSel = () => {
      placeholder();
      const sel = getSelection();
      if (!sel?.rangeCount || sel.isCollapsed || !ref.current?.contains(sel.anchorNode) || inside('pre')) {
        setTool(null);
        return;
      }
      const r = sel.getRangeAt(0).getBoundingClientRect();
      if (!r.width && !r.height) return setTool(null);
      setTool({ x: r.left + r.width / 2, y: r.top < 70 ? r.bottom : r.top, below: r.top < 70 });
      bump((n) => n + 1);
    };
    document.addEventListener('selectionchange', onSel);
    return () => document.removeEventListener('selectionchange', onSel);
  }, []);

  const matches = useMemo(() => {
    if (!slash) return [];
    const q = slash.q.toLowerCase();
    return COMMANDS.filter((x) => !q || x.label.toLowerCase().includes(q) || x.keys.includes(q));
  }, [slash?.q]);

  const runSlash = (cmd: (typeof COMMANDS)[number] | undefined) => {
    if (!cmd || !slash) return;
    const sel = getSelection()!;
    for (let i = 0; i < slash.q.length + 1; i++) sel.modify('extend', 'backward', 'character');
    exec('delete');
    setSlash(null);
    turn(cmd.type);
  };

  // ----- keys -----

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (slash && matches.length) {
      if (e.key === 'ArrowDown') { e.preventDefault(); setSlash({ ...slash, at: (slash.at + 1) % matches.length }); return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); setSlash({ ...slash, at: (slash.at - 1 + matches.length) % matches.length }); return; }
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); runSlash(matches[slash.at]); return; }
    }
    if (slash && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setSlash(null); return; }
    if (e.key === 'Escape') { e.stopPropagation(); el().blur(); return; }
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === 'k' && !getSelection()?.isCollapsed) {
      e.preventDefault();
      e.stopPropagation();
      startLink();
      return;
    }
    if (mod && e.key.toLowerCase() === 'e') { e.preventDefault(); toggleCode(); return; }
    if (mod && e.shiftKey && /^[0-9]$/.test(e.key)) {
      const t = (['p', 'h1', 'h2', 'h3', 'todo', 'ul', 'ol', 'pre', 'blockquote'] as BlockType[])[Number(e.key)];
      if (t) { e.preventDefault(); turn(t); }
      return;
    }
    const b = block();
    if (!b) return;
    const pre = inside('pre');
    if (pre) {
      if (e.key === 'Tab') { e.preventDefault(); exec('insertText', '  '); return; }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        const sel = getSelection()!;
        const after = document.createRange();
        after.setStart(sel.anchorNode!, sel.anchorOffset);
        after.setEnd(pre, pre.childNodes.length);
        const before = document.createRange();
        before.setStart(pre, 0);
        before.setEnd(sel.anchorNode!, sel.anchorOffset);
        const div = document.createElement('div');
        div.appendChild(before.cloneContents());
        // Enter on an empty last line leaves the code block.
        if (!after.toString() && /\n$/.test(preTextOf(div))) {
          pre.textContent = preTextOf(pre).replace(/\n+$/, '');
          const p = document.createElement('p');
          p.innerHTML = '<br>';
          pre.after(p);
          const r = document.createRange();
          r.setStart(p, 0);
          r.collapse(true);
          sel.removeAllRanges();
          sel.addRange(r);
          schedule();
          return;
        }
        exec('insertLineBreak');
        return;
      }
      return;
    }
    const before = textBefore(b);
    if (e.key === ' ' && b.tagName === 'P' && !b.closest('li')) {
      const shortcut: Record<string, BlockType> = { '#': 'h1', '##': 'h2', '###': 'h3', '-': 'ul', '*': 'ul', '+': 'ul', '1.': 'ol', '[]': 'todo', '[ ]': 'todo', '>': 'blockquote', '```': 'pre', '!': 'aside' };
      const t = shortcut[before];
      if (t) {
        e.preventDefault();
        const sel = getSelection()!;
        for (let i = 0; i < before.length; i++) sel.modify('extend', 'backward', 'character');
        exec('delete');
        turn(t);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && b.tagName === 'P') {
      if (b.textContent === '---' || b.textContent === '***') {
        e.preventDefault();
        b.textContent = '';
        b.innerHTML = '<br>';
        turn('hr');
        return;
      }
      if (/^```\w*$/.test(b.textContent ?? '')) {
        e.preventDefault();
        b.innerHTML = '<br>';
        turn('pre');
        return;
      }
    }
    if (e.key === 'Backspace' && !before && getSelection()?.isCollapsed && /^(H1|H2|H3|BLOCKQUOTE|ASIDE)$/.test(b.tagName) && !b.querySelector('img')) {
      e.preventDefault();
      turn('p');
      return;
    }
    if (e.key === 'Tab' && b.tagName === 'LI') {
      e.preventDefault();
      exec(e.shiftKey ? 'outdent' : 'indent');
      schedule();
    }
  };

  // ----- inline marks -----

  const toggleCode = () => {
    const code = inside('code');
    if (code) {
      code.replaceWith(document.createTextNode(code.textContent ?? ''));
      schedule();
      return;
    }
    const text = getSelection()?.toString();
    if (text) exec('insertHTML', `<code>${esc(text)}</code>`);
  };

  const startLink = () => {
    const sel = getSelection();
    if (!sel?.rangeCount) return;
    const a = inside('a');
    const r = sel.getRangeAt(0).getBoundingClientRect();
    setLink({ range: sel.getRangeAt(0).cloneRange(), value: a?.getAttribute('href') ?? '', x: r.left + r.width / 2, y: r.top < 70 ? r.bottom : r.top, below: r.top < 70 });
  };
  const finishLink = (value: string | null) => {
    const l = link;
    setLink(null);
    if (!l || value === null) return;
    el().focus();
    const sel = getSelection()!;
    sel.removeAllRanges();
    sel.addRange(l.range);
    const v = value.trim();
    if (!v) exec('unlink');
    else exec('createLink', /^(https?:|mailto:|#page:)/i.test(v) ? v : /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v) ? `mailto:${v}` : `https://${v}`);
    schedule();
  };

  const state = (cmd: string) => {
    try {
      return document.queryCommandState(cmd);
    } catch {
      return false;
    }
  };

  // ----- mouse, paste and drop -----

  const onMouseDown = (e: MouseEvent) => {
    const t = e.target as HTMLElement;
    const li = t.closest('li');
    if (li && li === t && li.parentElement?.getAttribute('data-type') === 'todo' && e.clientX - li.getBoundingClientRect().left < 24) {
      e.preventDefault();
      li.setAttribute('data-checked', li.getAttribute('data-checked') === 'true' ? 'false' : 'true');
      save();
    }
  };
  const onClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement;
    const a = t.closest('a');
    if (a) {
      const href = a.getAttribute('href') ?? '';
      const m = /^#page:([a-z0-9]+)$/i.exec(href);
      if (m) {
        e.preventDefault();
        if (s.doc.page[m[1]]) c.go({ view: 'doc', id: m[1] });
        else s.fail('That page was deleted.');
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        window.open(href, '_blank', 'noopener');
      }
      return;
    }
    if (t.tagName === 'IMG' && (t as HTMLImageElement).src) {
      // Selects it, so Delete removes it; a second click shows it big.
      const sel = getSelection()!;
      const r = document.createRange();
      r.selectNode(t);
      if (t.classList.contains('picked')) showImage((t as HTMLImageElement).src);
      for (const x of Array.from(el().querySelectorAll('img.picked'))) x.classList.remove('picked');
      t.classList.add('picked');
      sel.removeAllRanges();
      sel.addRange(r);
    } else for (const x of Array.from(el().querySelectorAll('img.picked'))) x.classList.remove('picked');
  };
  const onPaste = (e: ClipboardEvent) => {
    const files = imageFiles(e.clipboardData);
    if (files.length) {
      e.preventDefault();
      const sel = getSelection();
      insertImages(files, sel?.rangeCount ? sel.getRangeAt(0).cloneRange() : null);
      return;
    }
    const text = e.clipboardData.getData('text/plain');
    if (inside('pre')) {
      e.preventDefault();
      exec('insertText', text);
      return;
    }
    const html = e.clipboardData.getData('text/html');
    if (html) {
      e.preventDefault();
      exec('insertHTML', cleanHtml(html));
      resolve();
    } else if (text.includes('\n')) {
      e.preventDefault();
      exec('insertHTML', textToHtml(text));
    }
  };
  const onDrop = (e: DragEvent) => {
    const files = imageFiles(e.dataTransfer);
    if (!files.length) return;
    e.preventDefault();
    const doc = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
    const r = doc.caretRangeFromPoint?.(e.clientX, e.clientY) ?? null;
    insertImages(files, r);
  };

  return (
    <>
      <div
        ref={ref}
        className="d-editor"
        contentEditable
        suppressContentEditableWarning
        spellCheck
        onInput={() => { tidy(); findSlash(); placeholder(); schedule(); }}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => { if (/^Arrow|Home|End/.test(e.key)) findSlash(); }}
        onMouseDown={onMouseDown}
        onClick={onClick}
        onPaste={onPaste}
        onDrop={onDrop}
        onDragOver={(e) => { if (e.dataTransfer.types.includes('Files')) e.preventDefault(); }}
        onFocus={placeholder}
        onBlur={() => { save(); setSlash(null); setTimeout(placeholder); }}
      />
      {slash && matches.length > 0 && (
        <div className="d-menu d-slash" style={{ left: Math.min(slash.x, window.innerWidth - 280), ...(slash.y + 340 > window.innerHeight ? { bottom: window.innerHeight - slash.y + 26 } : { top: slash.y + 6 }) }} onMouseDown={(e) => e.preventDefault()}>
          <div className="d-menu-label">Blocks</div>
          {matches.map((m, n) => (
            <button type="button" key={m.type} className={`d-menu-item d-slash-item ${n === slash.at ? 'hot' : ''}`} onMouseEnter={() => setSlash({ ...slash, at: n })} onClick={() => runSlash(m)}>
              <span className="d-slash-icon">{m.icon}</span>
              <span className="d-stack d-gap0 d-grow d-min0"><span>{m.label}</span><span className="d-tiny d-dim">{m.hint}</span></span>
            </button>
          ))}
        </div>
      )}
      {(tool || link) && (
        <div className={`d-bubble ${(link ?? tool)!.below ? 'below' : ''}`} style={{ left: (link ?? tool)!.x, top: (link ?? tool)!.y }} onMouseDown={(e) => { if (!(e.target as HTMLElement).closest('input')) e.preventDefault(); }}>
          {link ? (
            <input
              className="d-bubble-input"
              autoFocus
              placeholder="Paste a link, then Enter"
              defaultValue={link.value}
              onKeyDown={(e) => {
                if (e.key === 'Enter') { e.preventDefault(); finishLink(e.currentTarget.value); }
                if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finishLink(null); }
              }}
              onBlur={(e) => finishLink(e.currentTarget.value === link.value ? null : e.currentTarget.value)}
            />
          ) : (
            <>
              <Menu width={180} trigger={(_, toggle) => <button type="button" className="d-bubble-btn wide" onClick={toggle}>{blockName(block())}<Icon name="chevronDown" size={11} /></button>}>
                {(close) => (['p', 'h1', 'h2', 'h3', 'ul', 'ol', 'todo', 'blockquote', 'aside', 'pre'] as BlockType[]).map((t) => {
                  const cmd = COMMANDS.find((x) => x.type === t)!;
                  return <MenuItem key={t} onClick={() => { close(); turn(t); }}><span className="d-slash-icon small">{cmd.icon}</span>{cmd.label}</MenuItem>;
                })}
              </Menu>
              <span className="d-bubble-sep" />
              <button type="button" className={`d-bubble-btn ${state('bold') ? 'on' : ''}`} title="Bold (Ctrl+B)" onClick={() => { exec('bold'); schedule(); bump((n) => n + 1); }}><b>B</b></button>
              <button type="button" className={`d-bubble-btn ${state('italic') ? 'on' : ''}`} title="Italic (Ctrl+I)" onClick={() => { exec('italic'); schedule(); bump((n) => n + 1); }}><i>I</i></button>
              <button type="button" className={`d-bubble-btn ${state('underline') ? 'on' : ''}`} title="Underline (Ctrl+U)" onClick={() => { exec('underline'); schedule(); bump((n) => n + 1); }}><u>U</u></button>
              <button type="button" className={`d-bubble-btn ${state('strikeThrough') ? 'on' : ''}`} title="Strikethrough" onClick={() => { exec('strikeThrough'); schedule(); bump((n) => n + 1); }}><s>S</s></button>
              <button type="button" className={`d-bubble-btn ${inside('code') ? 'on' : ''}`} title="Code (Ctrl+E)" onClick={() => { toggleCode(); bump((n) => n + 1); }}><span className="d-mono">{'<>'}</span></button>
              <button type="button" className={`d-bubble-btn ${inside('a') ? 'on' : ''}`} title="Link (Ctrl+K)" onClick={startLink}><Icon name="link" size={14} /></button>
            </>
          )}
        </div>
      )}
    </>
  );
}

function blockName(b: HTMLElement | null): string {
  if (!b) return 'Text';
  if (b.tagName === 'LI') return b.parentElement?.getAttribute('data-type') === 'todo' ? 'To-do list' : b.parentElement?.tagName === 'OL' ? 'Numbered list' : 'Bulleted list';
  return ({ H1: 'Heading 1', H2: 'Heading 2', H3: 'Heading 3', BLOCKQUOTE: 'Quote', ASIDE: 'Callout', PRE: 'Code' } as Record<string, string>)[b.tagName] ?? 'Text';
}

function preTextOf(el: Node): string {
  let t = '';
  const walk = (n: Node) => {
    for (const c of Array.from(n.childNodes)) {
      if (c.nodeType === 3) t += c.textContent;
      else if ((c as HTMLElement).tagName === 'BR') t += '\n';
      else walk(c);
    }
  };
  walk(el);
  return t;
}

/** The caret as a count of characters from the start, to put it back after the text is reloaded. */
function caretOffset(root: HTMLElement): number | null {
  const sel = getSelection();
  if (!sel?.rangeCount || !root.contains(sel.anchorNode)) return null;
  const r = document.createRange();
  r.setStart(root, 0);
  r.setEnd(sel.anchorNode!, sel.anchorOffset);
  return r.toString().length;
}

function setCaretOffset(root: HTMLElement, at: number) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let left = at;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const len = n.textContent?.length ?? 0;
    if (left <= len) {
      const r = document.createRange();
      r.setStart(n, left);
      r.collapse(true);
      getSelection()!.removeAllRanges();
      getSelection()!.addRange(r);
      return;
    }
    left -= len;
  }
}

// ---------- Markdown ----------

/** A page as Markdown, for copying out of Delta. Pictures stay in as data: URLs so nothing is lost. */
export function toMarkdown(doc: Doc, p: Page): string {
  const tpl = document.createElement('template');
  tpl.innerHTML = cleanHtml(p.body);
  const inline = (n: Node): string => {
    let out = '';
    for (const c of Array.from(n.childNodes)) {
      if (c.nodeType === 3) { out += (c.textContent ?? '').replace(/ /g, ' '); continue; }
      const e = c as HTMLElement;
      const inner = inline(e);
      switch (e.tagName) {
        case 'B': out += inner.trim() ? `**${inner}**` : inner; break;
        case 'I': out += inner.trim() ? `*${inner}*` : inner; break;
        case 'S': out += inner.trim() ? `~~${inner}~~` : inner; break;
        case 'U': out += inner; break;
        case 'CODE': out += `\`${e.textContent}\``; break;
        case 'BR': out += '  \n'; break;
        case 'A': {
          const href = e.getAttribute('href') ?? '';
          const m = /^#page:(.+)$/.exec(href);
          out += m ? `[${inner}](${pageTitle(doc.page[m[1]])})` : `[${inner}](${href})`;
          break;
        }
        case 'IMG': out += `![${doc.image[e.getAttribute('data-img') ?? '']?.name ?? ''}](${imageSrc(doc, e.getAttribute('data-img') ?? '') || e.getAttribute('src') || ''})`; break;
        default: out += inner;
      }
    }
    return out;
  };
  const list = (ul: HTMLElement, depth: number): string[] => {
    const out: string[] = [];
    let n = 1;
    for (const li of Array.from(ul.children)) {
      if (li.tagName === 'UL' || li.tagName === 'OL') { out.push(...list(li as HTMLElement, depth + 1)); continue; }
      const lead = ul.getAttribute('data-type') === 'todo' ? `- [${li.getAttribute('data-checked') === 'true' ? 'x' : ' '}] ` : ul.tagName === 'OL' ? `${n++}. ` : '- ';
      const own = document.createElement('div');
      for (const c of Array.from(li.childNodes)) if (!(c.nodeType === 1 && /^(UL|OL)$/.test((c as HTMLElement).tagName))) own.appendChild(c.cloneNode(true));
      out.push(`${'  '.repeat(depth)}${lead}${inline(own).trim()}`);
      for (const c of Array.from(li.children)) if (/^(UL|OL)$/.test(c.tagName)) out.push(...list(c as HTMLElement, depth + 1));
    }
    return out;
  };
  const blocks: string[] = [`# ${p.icon ? `${p.icon} ` : ''}${pageTitle(p)}`];
  for (const c of Array.from(tpl.content.childNodes)) {
    const e = c as HTMLElement;
    switch (e.tagName) {
      case 'H1': blocks.push(`## ${inline(e)}`); break;
      case 'H2': blocks.push(`### ${inline(e)}`); break;
      case 'H3': blocks.push(`#### ${inline(e)}`); break;
      case 'UL': case 'OL': blocks.push(list(e, 0).join('\n')); break;
      case 'BLOCKQUOTE': case 'ASIDE': blocks.push(inline(e).split('\n').map((l) => `> ${l}`).join('\n')); break;
      case 'PRE': blocks.push(`\`\`\`\n${e.textContent}\n\`\`\``); break;
      case 'HR': blocks.push('---'); break;
      case 'IMG': { const d = document.createElement('div'); d.appendChild(e.cloneNode()); blocks.push(inline(d)); break; }
      default: { const t = inline(e).trim(); if (t) blocks.push(t); }
    }
  }
  return blocks.join('\n\n') + '\n';
}
