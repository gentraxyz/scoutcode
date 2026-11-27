import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './ErrorBoundary';
import { SetupBar } from './SetupBar';
import { SetupCursor } from './SetupCursor';
import { invoke } from './api';
import { connect } from './store';
import { startUiLog, uiLog } from './uiLog';
import { installTooltips } from './Tooltip';
import './styles.css';
import './theme';

// The setup's small windows are this same UI, and their errors belong in the log too.
startUiLog();
installTooltips();

// The right-click menu is the browser's (Back, Reload, Inspect). Text fields keep theirs for copy and paste.
document.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement;
  if (!import.meta.env.DEV && !t.closest('input, textarea, .cm-editor, .xterm')) e.preventDefault();
});

// The Claude setup's bar and cursor over the browser are this same UI in their own small windows, without the app.
if (location.hash === '#setup-bar') createRoot(document.getElementById('root')!).render(<SetupBar />);
else if (location.hash === '#setup-cursor') createRoot(document.getElementById('root')!).render(<SetupCursor />);
else connect().then(() => {
  createRoot(document.getElementById('root')!).render(<ErrorBoundary what="Scout" root><App /></ErrorBoundary>);
  // The window starts hidden; show it once the app has painted, so it never opens as an empty, see-through frame.
  requestAnimationFrame(() => requestAnimationFrame(() => { invoke('window_ready').catch(() => {}); }));
}, (e) => {
  // Scout's core didn't answer: show why instead of a window that never appears.
  uiLog('error', "Scout couldn't start:", e);
  createRoot(document.getElementById('root')!).render(<StartupError error={e} />);
  invoke('window_ready').catch(() => {});
});

function StartupError({ error }: { error: unknown }) {
  const text = `${error instanceof Error ? `${error.name}: ${error.message}\n\n${error.stack ?? ''}` : String(error)}\n\nUser agent: ${navigator.userAgent}`;
  return (
    <div className="error-screen root" role="alert">
      <b className="strong">Scout couldn't start</b>
      <p className="small dim">Copy the details below and send them to us on Discord, so we can fix it.</p>
      <pre className="error-details">{text}</pre>
      <div className="row"><button type="button" className="button primary" onClick={() => location.reload()}>Try again</button></div>
    </div>
  );
}
