// Scout's own folder picker, for adding a project: places on the left, the folders in one on the right.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from './api';
import { ui, useUi, useView } from './store';
import { Icon, IconButton, Modal, Spinner } from './widgets';

type Place = { name: string; path: string; icon: string };
type Places = { home: string; places: Place[]; drives: Place[] };
type Folder = { name: string; path: string; hidden: boolean; repo: boolean };
type Listing = { path: string; parent: string | null; folders: Folder[]; error: string | null };

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();
const same = (a: string, b: string) => norm(a) === norm(b);
const baseName = (p: string) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
function parentOf(p: string) {
  const t = p.replace(/[\\/]+$/, '');
  const i = Math.max(t.lastIndexOf('\\'), t.lastIndexOf('/'));
  if (i < 0) return null;
  const up = t.slice(0, i);
  return /^[A-Za-z]:$/.test(up) ? `${up}\\` : up || '/';
}

/** The path as clickable pieces: C:\ › Users › me. */
function crumbs(path: string) {
  const sep = path.includes('\\') ? '\\' : '/';
  const parts = path.split(/[\\/]+/).filter(Boolean);
  const out: { name: string; path: string }[] = [];
  let at = path.startsWith('/') ? '/' : '';
  parts.forEach((part, i) => {
    if (i === 0 && /^[A-Za-z]:$/.test(part)) at = `${part}${sep}`;
    else at = at.endsWith(sep) || at === '' ? at + part : `${at}${sep}${part}`;
    out.push({ name: part, path: at });
  });
  if (path.startsWith('/')) out.unshift({ name: '/', path: '/' });
  return out;
}

function PathBar({ path, onGo }: { path: string; onGo: (p: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(path);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setDraft(path), [path]);
  // A long path keeps its end in view.
  useLayoutEffect(() => {
    if (list.current) list.current.scrollLeft = list.current.scrollWidth;
  }, [path, editing]);
  if (editing) {
    return (
      <input
        className="fp-path fp-path-edit mono"
        autoFocus
        spellCheck={false}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onFocus={(e) => e.target.select()}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            setEditing(false);
            if (draft.trim()) onGo(draft.trim().replace(/^"|"$/g, ''));
          } else if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            setEditing(false);
          }
          e.stopPropagation();
        }}
      />
    );
  }
  const parts = crumbs(path);
  return (
    <div ref={list} className="fp-path" title="Click to type a path (Ctrl+L)" onClick={(e) => e.target === e.currentTarget && setEditing(true)} data-path-bar>
      {parts.map((c, i) => (
        <span key={c.path} className="fp-crumb-wrap">
          {i > 0 && <span className="fp-sep"><Icon name="chevronRight" size={11} /></span>}
          <button type="button" className={`fp-crumb ${i === parts.length - 1 ? 'last' : ''}`} onClick={() => onGo(c.path)}>{c.name}</button>
        </span>
      ))}
      <button type="button" className="fp-path-fill" aria-label="Type a path" onClick={() => setEditing(true)} />
    </div>
  );
}

function Picker({ onClose }: { onClose: () => void }) {
  const chat = useView('chat');
  const projects = useView('projects');
  const [places, setPlaces] = useState<Places | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [loading, setLoading] = useState(false);
  const [history, setHistory] = useState<{ back: string[]; forward: string[] }>({ back: [], forward: [] });
  const [selected, setSelected] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [showHidden, setShowHidden] = useState(false);
  const [creating, setCreating] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const seq = useRef(0);

  const load = async (dir: string, select: string | null = null) => {
    const n = ++seq.current;
    setLoading(true);
    const l = await invoke<Listing>('folder_list', { dir }).catch((e) => ({ path: dir, parent: parentOf(dir), folders: [], error: String(e) }) as Listing);
    if (n !== seq.current) return;
    setLoading(false);
    setListing(l);
    setFilter('');
    setCreating(null);
    setError(null);
    setSelected(select && l.folders.some((f) => same(f.path, select)) ? select : null);
  };
  const go = (dir: string, select: string | null = null) => {
    if (listing && !same(listing.path, dir)) setHistory((h) => ({ back: [...h.back, listing.path], forward: [] }));
    load(dir, select);
  };
  const back = () => {
    const prev = history.back.at(-1);
    if (!prev || !listing) return;
    setHistory((h) => ({ back: h.back.slice(0, -1), forward: [listing.path, ...h.forward] }));
    load(prev);
  };
  const forward = () => {
    const next = history.forward[0];
    if (!next || !listing) return;
    setHistory((h) => ({ back: [...h.back, listing.path], forward: h.forward.slice(1) }));
    load(next);
  };
  const up = () => {
    const p = listing && (listing.parent ?? parentOf(listing.path));
    if (p && listing) go(p, listing.path);
  };

  // It opens beside the current project, with that project picked out: projects tend to sit next to each other.
  useEffect(() => {
    invoke<Places>('folder_places').then(setPlaces).catch(() => {});
    const start = chat.cwd ? parentOf(chat.cwd) ?? chat.cwd : null;
    if (start) load(start, chat.cwd);
    else invoke<Places>('folder_places').then((p) => load(p.home));
  }, []);

  const shown = useMemo(() => {
    if (!listing) return [];
    const q = filter.trim().toLowerCase();
    return listing.folders.filter((f) => (showHidden || !f.hidden || same(f.path, selected ?? '')) && (!q || f.name.toLowerCase().includes(q)));
  }, [listing, filter, showHidden, selected]);
  const hiddenCount = listing ? listing.folders.filter((f) => f.hidden).length : 0;

  // Typing narrows the list; the first match is picked so Enter goes straight in.
  useEffect(() => {
    if (filter && shown.length && !shown.some((f) => same(f.path, selected ?? ''))) setSelected(shown[0].path);
  }, [filter, shown]);
  useEffect(() => {
    listRef.current?.querySelector('.fp-row.selected')?.scrollIntoView({ block: 'nearest' });
  }, [selected]);

  const target = selected ?? listing?.path ?? null;
  const choose = async (dir: string | null = target) => {
    if (!dir || busy) return;
    setBusy(true);
    try {
      const fresh = await invoke<string | null>('add_project', { dir });
      onClose();
      if (fresh) ui.set({ renaming: fresh });
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };
  const create = async () => {
    if (!listing || creating === null) return;
    try {
      const made = await invoke<string>('folder_create', { parent: listing.path, name: creating });
      await load(listing.path, made);
    } catch (e) {
      setError(String(e));
    }
  };

  const move = (by: number) => {
    if (!shown.length) return;
    const i = shown.findIndex((f) => same(f.path, selected ?? ''));
    const next = i < 0 ? (by > 0 ? 0 : shown.length - 1) : Math.max(0, Math.min(shown.length - 1, i + by));
    setSelected(shown[next].path);
  };
  const onKey = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest('.fp-path-edit, .fp-new input')) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); move(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); }
    else if (e.key === 'PageDown') { e.preventDefault(); move(10); }
    else if (e.key === 'PageUp') { e.preventDefault(); move(-10); }
    else if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); choose(); }
    else if (e.key === 'Enter') { e.preventDefault(); if (selected) go(selected); }
    else if (e.key === 'Backspace' && !filter) { e.preventDefault(); up(); }
    else if (e.key === 'ArrowLeft' && e.altKey) { e.preventDefault(); back(); }
    else if (e.key === 'ArrowRight' && e.altKey) { e.preventDefault(); forward(); }
    else if (e.key === 'ArrowUp' && e.altKey) { e.preventDefault(); up(); }
    else if (e.key.toLowerCase() === 'l' && e.ctrlKey) { e.preventDefault(); (document.querySelector('[data-path-bar] .fp-path-fill') as HTMLElement | null)?.click(); }
    else if (e.key === 'Escape' && filter) { e.preventDefault(); e.stopPropagation(); setFilter(''); }
  };

  const here = (p: string) => !!listing && same(listing.path, p);
  const parentPath = listing ? listing.parent ?? parentOf(listing.path) : null;

  return (
    <div className="fp" onKeyDown={onKey}>
      <aside className="fp-side">
        <div className="fp-title">Add a project</div>
        {places && (
          <>
            <div className="fp-group">
              {places.places.map((p) => (
                <button key={p.path} type="button" className={`fp-place ${here(p.path) ? 'on' : ''}`} onClick={() => go(p.path)} title={p.path}>
                  <Icon name={p.icon} size={15} /><span className="ellipsis">{p.name}</span>
                </button>
              ))}
            </div>
            {places.drives.length > 0 && (
              <div className="fp-group">
                <div className="fp-label">Drives</div>
                {places.drives.map((p) => (
                  <button key={p.path} type="button" className={`fp-place ${here(p.path) ? 'on' : ''}`} onClick={() => go(p.path)} title={p.path}>
                    <Icon name="drive" size={15} /><span className="ellipsis">{p.name}</span>
                  </button>
                ))}
              </div>
            )}
          </>
        )}
        {projects.length > 0 && (
          <div className="fp-group">
            <div className="fp-label">Projects</div>
            {projects.map((p) => (
              <button key={p.path} type="button" className={`fp-place ${here(p.path) ? 'on' : ''}`} onClick={() => go(parentOf(p.path) ?? p.path, p.path)} title={p.path}>
                <Icon name="folder" size={15} /><span className="ellipsis">{p.name}</span>
              </button>
            ))}
          </div>
        )}
      </aside>

      <section className="fp-main">
        <header className="fp-bar">
          <IconButton icon="back" tip="Back (Alt+←)" size={28} disabled={!history.back.length} onClick={back} />
          <IconButton icon="forward" tip="Forward (Alt+→)" size={28} disabled={!history.forward.length} onClick={forward} />
          <IconButton icon="arrowUp" tip="Up a folder (Backspace)" size={28} disabled={!parentPath} onClick={up} />
          {listing && <PathBar path={listing.path} onGo={(p) => go(p)} />}
          <IconButton icon="close" tip="Cancel (Esc)" size={28} onClick={onClose} />
        </header>

        <div className="fp-tools">
          <label className="fp-filter">
            <Icon name="search" size={14} />
            <input ref={filterRef} autoFocus placeholder="Filter folders" spellCheck={false} value={filter} onChange={(e) => setFilter(e.target.value)} />
            {loading && <Spinner size={11} />}
          </label>
          {hiddenCount > 0 && (
            <button type="button" className={`fp-tool ${showHidden ? 'on' : ''}`} title={showHidden ? 'Hide hidden folders' : 'Show hidden folders'} onClick={() => setShowHidden(!showHidden)}>
              <Icon name="eye" size={14} />{hiddenCount} hidden
            </button>
          )}
          <button type="button" className="fp-tool" title="Make a folder here" disabled={!listing || !!listing.error} onClick={() => setCreating('')}>
            <Icon name="folderPlus" size={14} />New folder
          </button>
        </div>

        <div ref={listRef} className="fp-list" onClick={(e) => e.target === e.currentTarget && setSelected(null)}>
          {creating !== null && (
            <div className="fp-row fp-new">
              <Icon name="folder" size={16} />
              <input
                autoFocus
                className="grow"
                placeholder="Folder name"
                spellCheck={false}
                value={creating}
                onChange={(e) => setCreating(e.target.value)}
                onBlur={() => !creating.trim() && setCreating(null)}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === 'Enter') { e.preventDefault(); create(); }
                  if (e.key === 'Escape') { e.preventDefault(); setCreating(null); filterRef.current?.focus(); }
                }}
              />
            </div>
          )}
          {listing?.error ? (
            <div className="fp-empty">{listing.error}</div>
          ) : listing && !shown.length && creating === null ? (
            <div className="fp-empty">{filter ? `No folders match “${filter}”.` : 'No folders in here. You can add this one.'}</div>
          ) : (
            shown.map((f) => (
              <button
                key={f.path}
                type="button"
                className={`fp-row ${same(f.path, selected ?? '') ? 'selected' : ''} ${f.hidden ? 'hidden' : ''}`}
                onClick={() => setSelected(f.path)}
                onDoubleClick={() => go(f.path)}
                title={f.path}
              >
                <Icon name="folder" size={16} />
                <span className="grow ellipsis">{f.name}</span>
                {f.repo && <span className="fp-badge">git</span>}
                <span className="fp-enter" onClick={(e) => { e.stopPropagation(); go(f.path); }} title="Open this folder"><Icon name="chevronRight" size={13} /></span>
              </button>
            ))
          )}
        </div>

        <footer className="fp-foot">
          <div className="grow fp-choice">
            {error ? <span className="fp-error ellipsis">{error}</span> : target ? (
              <>
                <span className="dim small">Project folder</span>
                <span className="ellipsis mono small" title={target}>{target}</span>
              </>
            ) : null}
          </div>
          <button type="button" className="button" onClick={onClose}>Cancel</button>
          <button type="button" className="button primary" disabled={!target || !!listing?.error || busy} title="Ctrl+Enter" onClick={() => choose()}>
            {busy ? <Spinner size={11} /> : null}Add {target ? `“${baseName(target)}”` : 'folder'}
          </button>
        </footer>
      </section>
    </div>
  );
}

export function FolderPicker() {
  const open = useUi((u) => u.pickingFolder);
  if (!open) return null;
  const close = () => ui.set({ pickingFolder: false });
  return (
    <Modal onClose={close} className="folder-picker">
      <Picker onClose={close} />
    </Modal>
  );
}
