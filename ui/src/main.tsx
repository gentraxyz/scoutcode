import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './ErrorBoundary';
import { SetupBar } from './SetupBar';
import { SetupCursor } from './SetupCursor';
import { invoke } from './api';
import { connect } from './store';
import './styles.css';
import './theme';

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
});
