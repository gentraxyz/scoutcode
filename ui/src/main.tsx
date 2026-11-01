import { createRoot } from 'react-dom/client';
import { App } from './App';
import { connect } from './store';
import './styles.css';

// The right-click menu is the browser's (Back, Reload, Inspect). Text fields keep theirs for copy and paste.
document.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement;
  if (!import.meta.env.DEV && !t.closest('input, textarea, .cm-editor, .xterm')) e.preventDefault();
});

connect().then(() => createRoot(document.getElementById('root')!).render(<App />));
