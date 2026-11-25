// "Working on …" under the prompt box: the picker that switches between this PC's projects.
import { act, projectDir } from './api';
import { focusComposer, ui, useCore, useView } from './store';
import { Icon, MenuItem, Popover } from './widgets';

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();

/** Where the next chat runs: one of this PC's project folders, and whether it gets its own worktree. */
export function WorkingOn({ disabled }: { disabled?: boolean }) {
  const projects = useView('projects');
  const chat = useView('chat');
  // A new chat in a git repository can go in its own worktree; once it has started in one, the chip says so.
  const started = useCore((s) => s.items.length > 0);
  const worktreeChip = chat.inWorktree || (chat.git && !started);
  return (
    <span className="working-on">
      <Popover width={260} side="top" trigger={(open, toggle) => (
        <button type="button" className={`chip working-chip ${open ? 'open' : ''}`} disabled={disabled} title={`${chat.cwd} · click to switch`} onClick={toggle}>
          <Icon name="folderOpen" size={14} />
          <span className="ellipsis">{chat.project}</span>
        </button>
      )}>
        {(close) => (
          <div className="working-menu">
            {projects.map((p) => (
              <MenuItem key={p.path} icon="folder" selected={norm(p.path) === norm(projectDir(chat.cwd))} onClick={() => {
                close();
                if (norm(p.path) !== norm(projectDir(chat.cwd))) act('changeProject', { dir: p.path });
                focusComposer();
              }}>{p.name}</MenuItem>
            ))}
            <hr />
            <MenuItem icon="folderPlus" onClick={() => { close(); ui.set({ pickingFolder: true }); }}>New project…</MenuItem>
          </div>
        )}
      </Popover>
      {worktreeChip && (
        <button
          type="button"
          className={`chip working-chip worktree-chip ${chat.worktree || chat.inWorktree ? 'on' : ''}`}
          disabled={disabled || chat.inWorktree}
          aria-pressed={!!(chat.worktree || chat.inWorktree)}
          title={chat.inWorktree ? `This chat runs in its own worktree: ${chat.cwd}` : chat.worktree ? 'On: the first message makes a new git worktree and branch for this chat · click to turn off' : 'Run this chat in its own git worktree, on a new branch'}
          onClick={() => act('setWorktree', { on: !chat.worktree })}
        >
          <Icon name="layers" size={14} />
          <span>Worktree</span>
        </button>
      )}
    </span>
  );
}

