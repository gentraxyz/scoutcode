// The built-in browser, in the right-hand panel. The page itself is a native webview that Rust keeps over the space
// this tab leaves for it (browser.rs); this draws the address bar and says who's using it. The user can browse here
// on their own, and with Settings > Browser on, Scout can too.
import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from './api';
import { ui, useUi, useView } from './store';
import { IconButton } from './widgets';

type BrowserState = { url: string; title: string; loading: boolean; agent: boolean; user: boolean; open: boolean };

const EMPTY: BrowserState = { url: '', title: '', loading: false, agent: false, user: false, open: false };

export function BrowserTab() {
  const host = useRef<HTMLDivElement>(null);
  const [st, setSt] = useState<BrowserState>(EMPTY);
  const [addr, setAddr] = useState('');
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState('');
  const { browserAgent } = useView('cfg');
  // The webview is drawn above the rest of the window, so it steps aside for anything that would cover it.
  const covered = useUi((u) => !!u.settings || !!u.deleting || u.pickingFolder || u.pickingFiles || !!u.uncensored || u.dropping || !!u.renaming || u.newProject !== false);

  useEffect(() => {
    invoke<BrowserState>('browser_status').then(setSt, () => {});
    const off = listen<BrowserState>('browser', (e) => setSt(e.payload));
    return () => { off.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (!editing) setAddr(st.url === 'about:blank' ? '' : st.url);
  }, [st.url, editing]);

  // Keeps the webview over the placeholder: each frame, it sends where that is if it moved.
  useEffect(() => {
    const shown = !covered;
    let last = '';
    let frame = 0;
    const tick = () => {
      const el = host.current;
      if (el) {
        const r = el.getBoundingClientRect();
        const on = shown && r.width > 1 && r.height > 1;
        const key = on ? `${r.x},${r.y},${r.width},${r.height}` : 'hidden';
        if (key !== last) {
          last = key;
          invoke('browser_bounds', { x: r.x, y: r.y, width: r.width, height: r.height, shown: on }).catch(() => {});
        }
      }
      if (shown) frame = requestAnimationFrame(tick);
    };
    tick();
    return () => {
      cancelAnimationFrame(frame);
      invoke('browser_bounds', { x: 0, y: 0, width: 0, height: 0, shown: false }).catch(() => {});
    };
  }, [covered]);

  const go = (to: string) => {
    setError('');
    invoke('browser_go', { to }).catch((e) => setError(String(e)));
  };

  return (
    <div className="dock-page browser-tab">
      <div className="dock-actions">
        <IconButton icon="back" tip="Back" size={26} disabled={!st.open} onClick={() => go('back')} />
        <IconButton icon="forward" tip="Forward" size={26} disabled={!st.open} onClick={() => go('forward')} />
        <IconButton icon="refresh" tip={st.loading ? 'Loading…' : 'Reload'} size={26} disabled={!st.open} onClick={() => go('reload')} />
        <input
          className="input browser-address grow"
          value={addr}
          placeholder="Type an address or search"
          spellCheck={false}
          onFocus={(e) => { setEditing(true); e.currentTarget.select(); }}
          onBlur={() => setEditing(false)}
          onChange={(e) => setAddr(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && addr.trim()) { go(addr.trim()); e.currentTarget.blur(); }
            if (e.key === 'Escape') e.currentTarget.blur();
          }}
        />
      </div>
      {st.user ? (
        <div className="file-banner">
          <span className="grow">You have the browser. Scout waits until you hand it back.</span>
          <button type="button" className="text-button" onClick={() => invoke('browser_take_over', { on: false })}>Hand back to Scout</button>
        </div>
      ) : st.agent ? (
        <div className="file-banner browser-agent">
          <span className="grow">Scout is using the browser.</span>
          <button type="button" className="text-button" onClick={() => invoke('browser_take_over', { on: true })}>Take over</button>
        </div>
      ) : null}
      {error && <div className="file-banner red">{error}</div>}
      <div className="browser-host" ref={host}>
        {!st.open && (
          <div className="placeholder">
            Type an address above to browse here.
            <br />
            {browserAgent ? 'Scout can use this browser too, and you can watch it work.' : (
              <>Scout can use it too once you turn it on in <button type="button" className="text-button" onClick={() => ui.set({ settings: 'browser' })}>Settings &gt; Browser</button>.</>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
