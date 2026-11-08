import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

/**
 * Scout's own cursor while it adds the Claude connector (claude_native.rs), so the user's cursor stays theirs. It's a
 * small click-through window, `setup-cursor`, that Scout moves around; its top-left corner is the tip.
 */
export function SetupCursor() {
  const [down, setDown] = useState(false);

  useEffect(() => {
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    const off = listen<boolean>('setup-cursor-press', (e) => setDown(e.payload));
    return () => { off.then((f) => f()); };
  }, []);

  return (
    <div className={`setup-cursor${down ? ' down' : ''}`}>
      <svg width="22" height="24" viewBox="0 0 22 24"><path d="M1 1l18 10.2-7.8 1.7L7.4 21z" fill="#fff" stroke="#111" strokeWidth="1.4" strokeLinejoin="round" /></svg>
      <span>Scout</span>
    </div>
  );
}
