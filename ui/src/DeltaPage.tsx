import { useMemo } from 'react';
import { act, invoke } from './api';
import { core, ui, useView } from './store';
import { Delta } from './delta/Delta';
import type { AgentTarget, Host } from './delta/sync';

const norm = (p: string) => p.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();

/** ScoutCode Delta in the desktop app: local workspaces in ~/.scoutcode/delta, cloud ones through the account's key. */
export function DeltaPage() {
  const { username } = useView('cfg');
  const host = useMemo<Host>(() => ({
    kind: 'desktop',
    me: username,
    cloud: (method, path, body) => invoke('delta_cloud', { method, path, body: body ?? null }),
    local: {
      list: () => invoke('delta_local_list'),
      read: (id) => invoke('delta_local_read', { id }),
      write: (id, text) => invoke('delta_local_write', { id, text }),
      remove: (id) => invoke('delta_local_delete', { id }),
    },
    agent: {
      async targets(): Promise<AgentTarget[]> {
        const { projects, chat } = core.get().view;
        const here = projects.filter((p) => norm(p.path) === norm(chat.cwd));
        const rest = projects.filter((p) => norm(p.path) !== norm(chat.cwd));
        return [...here, ...rest].map((p) => ({ pc: null, pcName: 'This PC', cwd: p.path, project: p.name, online: true }));
      },
      async start(target, prompt) {
        const { view, items } = core.get();
        // A new chat in that project, unless the open one is empty and already there.
        if (items.length > 0 || norm(view.chat.cwd) !== norm(target.cwd)) await invoke('act', { action: { type: 'changeProject', dir: target.cwd } });
        const taken = await invoke<boolean>('send', { text: prompt, files: [], steer: false });
        if (!taken) throw new Error("Scout didn't take the message. Is it set up with a model?");
        // The new chat gets its id once the first message is saved.
        for (let n = 0; n < 20 && !core.get().view.chat.id; n++) await new Promise((r) => setTimeout(r, 100));
        return { chatId: core.get().view.chat.id };
      },
      open(run) {
        if (run.chatId) act('openChat', { id: run.chatId });
        ui.set({ page: 'chat' });
      },
    },
    showChat: () => ui.set({ page: 'chat' }),
  }), [username]);
  return (
    <div className="delta-host">
      <Delta key={username ?? ''} host={host} onExit={() => ui.set({ page: 'chat' })} />
    </div>
  );
}
