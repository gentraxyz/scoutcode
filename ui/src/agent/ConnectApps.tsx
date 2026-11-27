// Connect to Apps: the apps (MCP servers) agents can use, connected by the user rather than by asking an agent. It shows
// all the agents together, or one of them, with what they're connected to orbiting them; below are suggestions and a
// custom server. Apps connected with "All agents" are the account's, which every agent can use; picking one agent
// connects it for that agent only. The site does the connecting (app_* in src/pages/api/account/agents.ts): a server
// that needs signing in opens its sign-in page in the browser, and one that needs a token or keys asks for them here.
// The foot of the Agents sidebar opens it (SidebarFoot), next to the account menu and the usage button.
//
// Each app shows its site's favicon, fetched by the app (provider_icon in src/bridge.rs), shrunk and kept on this PC.
import { useEffect, useState, type CSSProperties } from 'react';
import { invoke, openExternal } from '../api';
import { AccountMenu, UsageGauge } from '../AccountMenu';
import { useUsageShare } from '../Quota';
import { ui, useView } from '../store';
import { Icon, IconButton, Modal, Spinner } from '../widgets';
import { Avatar } from './Avatar';
import { agentStore, appSecret, connectApp, disconnectApp, loadAgents, useAgents, type Agent, type App, type AppTarget, type SecretField } from './store';

type Suggestion = { id: string; name: string; about: string; url: string; site: string };

/** Remote MCP servers whose sign-in works from ScoutCode (they register apps themselves, or GitHub's through the
 *  ScoutCode GitHub App). Ones that only take apps they approved (Vercel, Figma, Canva, Intercom) aren't here. */
const SUGGESTIONS: Suggestion[] = [
  { id: 'github', name: 'GitHub', about: 'Repositories, issues and pull requests', url: 'https://api.githubcopilot.com/mcp/', site: 'https://github.com' },
  { id: 'notion', name: 'Notion', about: 'Pages, docs and databases', url: 'https://mcp.notion.com/mcp', site: 'https://www.notion.so' },
  { id: 'linear', name: 'Linear', about: 'Issues, projects and cycles', url: 'https://mcp.linear.app/mcp', site: 'https://linear.app' },
  { id: 'atlassian', name: 'Atlassian', about: 'Jira and Confluence', url: 'https://mcp.atlassian.com/v1/mcp', site: 'https://www.atlassian.com' },
  { id: 'sentry', name: 'Sentry', about: 'Errors and performance issues', url: 'https://mcp.sentry.dev/mcp', site: 'https://sentry.io' },
  { id: 'stripe', name: 'Stripe', about: 'Payments, customers and invoices', url: 'https://mcp.stripe.com', site: 'https://stripe.com' },
  { id: 'supabase', name: 'Supabase', about: 'Databases, auth and storage', url: 'https://mcp.supabase.com/mcp', site: 'https://supabase.com' },
  { id: 'neon', name: 'Neon', about: 'Serverless Postgres', url: 'https://mcp.neon.tech/mcp', site: 'https://neon.tech' },
  { id: 'cloudflare', name: 'Cloudflare', about: 'Workers, KV, R2 and D1', url: 'https://bindings.mcp.cloudflare.com/mcp', site: 'https://www.cloudflare.com' },
  { id: 'posthog', name: 'PostHog', about: 'Product analytics and feature flags', url: 'https://mcp.posthog.com/mcp', site: 'https://posthog.com' },
  { id: 'monday', name: 'monday.com', about: 'Boards and work items', url: 'https://mcp.monday.com/mcp', site: 'https://monday.com' },
  { id: 'paypal', name: 'PayPal', about: 'Invoices, orders and payments', url: 'https://mcp.paypal.com/mcp', site: 'https://www.paypal.com' },
  { id: 'zapier', name: 'Zapier', about: 'Thousands of apps through your Zaps', url: 'https://mcp.zapier.com/api/mcp/mcp', site: 'https://zapier.com' },
  { id: 'huggingface', name: 'Hugging Face', about: 'Models, datasets and Spaces', url: 'https://huggingface.co/mcp', site: 'https://huggingface.co' },
  { id: 'context7', name: 'Context7', about: 'Up-to-date docs for any library', url: 'https://mcp.context7.com/mcp', site: 'https://context7.com' },
];

/** The site's id for a connection named `name` (slugOf in lib/agent-bot.ts). */
const slugOf = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 24) || 'agent';
const suggestionOf = (id: string) => SUGGESTIONS.find((s) => s.id === id);
const nameOf = (a: Pick<App, 'id' | 'name'>) => suggestionOf(a.id)?.name ?? a.name.charAt(0).toUpperCase() + a.name.slice(1);
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Where an app's favicon is: its suggestion's site, else its server's address. None for one run by a command. */
function siteOf(a: Pick<App, 'id' | 'url'>): string | null {
  const s = suggestionOf(a.id);
  if (s) return s.site;
  try {
    return a.url ? new URL(a.url).origin : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- favicons

const ICONS_KEY = 'apps.icons.v1';
/** Icons found, by site, kept on this PC. A site without one is only remembered until the app restarts. */
let keptIcons: Record<string, string> = (() => {
  try {
    return JSON.parse(localStorage.getItem(ICONS_KEY) ?? '{}') ?? {};
  } catch {
    return {};
  }
})();
const asked = new Map<string, Promise<string | null>>();

/** A picture shrunk to 64 px square, so dozens of them fit in localStorage. */
function shrink(data: string): Promise<string> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement('canvas');
      c.width = c.height = 64;
      const g = c.getContext('2d');
      if (!g) return resolve(data);
      g.imageSmoothingQuality = 'high';
      g.drawImage(img, 0, 0, 64, 64);
      try {
        resolve(c.toDataURL('image/png'));
      } catch {
        resolve(data);
      }
    };
    img.onerror = () => resolve(data);
    img.src = data;
  });
}

function iconOf(site: string): Promise<string | null> {
  if (keptIcons[site]) return Promise.resolve(keptIcons[site]);
  let p = asked.get(site);
  if (!p) {
    p = invoke<string>('provider_icon', { address: site })
      .then(shrink)
      .then((d) => {
        keptIcons = { ...keptIcons, [site]: d };
        try {
          localStorage.setItem(ICONS_KEY, JSON.stringify(keptIcons));
        } catch {
          /* full: it's fetched again next time */
        }
        return d;
      })
      .catch(() => null);
    asked.set(site, p);
  }
  return p;
}

/** An app's icon: its favicon on a light tile, or its initial while there's none. */
function AppIcon({ id, name, site, size = 40 }: { id: string; name: string; site: string | null; size?: number }) {
  const [src, setSrc] = useState<string | null>(site ? keptIcons[site] ?? null : null);
  useEffect(() => {
    if (!site) return setSrc(null);
    let live = true;
    void iconOf(site).then((d) => live && setSrc(d));
    return () => {
      live = false;
    };
  }, [site]);
  return (
    <span className={`app-icon ${src ? 'has-img' : ''}`} style={{ width: size, height: size, borderRadius: Math.round(size * 0.3) } as CSSProperties} aria-hidden>
      {src ? <img src={src} alt="" draggable={false} /> : site ? <b style={{ fontSize: size * 0.42 }}>{name.replace(/[^A-Za-z0-9]/g, '').charAt(0).toUpperCase() || '?'}</b> : <Icon name="terminal" size={Math.round(size * 0.45)} />}
    </span>
  );
}

const iconFor = (a: Pick<App, 'id' | 'name' | 'url'>, size?: number) => <AppIcon id={a.id} name={nameOf(a)} site={siteOf(a)} size={size} />;

// ---------------------------------------------------------------- the foot of the sidebar

/** The foot of the Agents sidebar: "Connect to Apps" above the account menu and the usage button. */
export function SidebarFoot() {
  const { hasKey } = useView('cfg');
  const shared = useAgents((s) => s.apps);
  const list = useAgents((s) => s.list) ?? [];
  const open = useAgents((s) => s.appsFor);
  const { frac, low, tip } = useUsageShare();
  const pct = Math.round(Math.max(0, Math.min(1, frac)) * 100);
  // What's connected anywhere: the account's apps, then each agent's own.
  const seen = new Map<string, App>();
  for (const a of [...shared, ...list.flatMap((x) => x.mcp ?? [])]) if (a.connected && !seen.has(a.id)) seen.set(a.id, a);
  const connected = [...seen.values()];
  return (
    <div className="agent-side-foot">
      {hasKey && (
        <button type="button" className="apps-open" onClick={() => agentStore.set({ appsFor: 'all' })} title="Connect apps and services to your agents">
          <span className="apps-open-text">Connect to Apps</span>
          {connected.length > 0 ? (
            <span className="apps-open-marks">
              {connected.slice(0, 3).map((a) => <span key={a.id}>{iconFor(a, 18)}</span>)}
              {connected.length > 3 && <span className="apps-open-more">+{connected.length - 3}</span>}
            </span>
          ) : (
            <Icon name="plus" size={13} />
          )}
        </button>
      )}
      <div className="agent-side-account">
        <AccountMenu placement="in-side" />
        <button type="button" className="usage-btn" title={tip ? `Usage: ${tip} · ${pct}% used` : `Usage: ${pct}% used`} aria-label="Usage" onClick={() => ui.set({ usageOpen: true })}>
          <UsageGauge frac={frac} low={low} />
        </button>
      </div>
      {open && <ConnectApps />}
    </div>
  );
}

// ---------------------------------------------------------------- the popup

/** What's waiting on the user for an app: signing in in the browser, or values to type in. */
type Pending = { id: string; name: string; kind: 'signin'; url: string } | { id: string; name: string; kind: 'secret'; fields: SecretField[] };

type OrbitApp = App & { shared?: boolean; ghost?: boolean };

/** Shown on an empty orbit, faintly, as a way in. */
const GHOSTS: OrbitApp[] = ['github', 'notion', 'linear', 'sentry'].map((id) => {
  const s = suggestionOf(id)!;
  return { id, name: s.name, url: s.url, connected: false, ghost: true };
});

function ConnectApps() {
  const target = useAgents((s) => s.appsFor) as AppTarget;
  const list = useAgents((s) => s.list) ?? [];
  const shared = useAgents((s) => s.apps);
  const agent = target === 'all' ? null : list.find((a) => a.id === target) ?? null;
  const own = agent ? agent.mcp ?? [] : shared;
  const apps: OrbitApp[] = agent ? [...own, ...shared.filter((s) => !own.some((o) => o.id === s.id)).map((s) => ({ ...s, shared: true }))] : shared;
  const orbit = apps.length ? apps : GHOSTS;
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [entering, setEntering] = useState<string | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [custom, setCustom] = useState(false);
  const close = () => agentStore.set({ appsFor: null });
  const pick = (t: AppTarget) => {
    agentStore.set({ appsFor: t });
    setPending(null);
    setPicked(null);
    setErr(null);
  };
  const arrive = (id: string) => {
    setEntering(id);
    setTimeout(() => setEntering((e) => (e === id ? null : e)), 1600);
  };

  // A sign-in finishes in the browser: check the list until the app shows up connected.
  useEffect(() => {
    if (pending?.kind !== 'signin') return;
    const t = setInterval(() => void loadAgents(), 2500);
    return () => clearInterval(t);
  }, [pending]);
  useEffect(() => {
    if (!pending) return;
    const a = own.find((x) => x.id === pending.id);
    if (a?.connected) {
      setPending(null);
      arrive(a.id);
    }
  }, [own, pending]);

  const connect = async (name: string, body: Omit<Parameters<typeof connectApp>[1], 'name'>) => {
    const id = slugOf(name);
    setBusy(id);
    setErr(null);
    try {
      const r = await connectApp(target, { name, ...body });
      const appId = r.app?.id ?? id;
      if (r.kind === 'connected') arrive(appId);
      else if (r.kind === 'signin') {
        openExternal(r.url);
        setPending({ id: appId, name: suggestionOf(appId)?.name ?? name, kind: 'signin', url: r.url });
      } else setPending({ id: appId, name: suggestionOf(appId)?.name ?? name, kind: 'secret', fields: r.fields });
      setCustom(false);
    } catch (e) {
      setErr(message(e));
    } finally {
      setBusy(null);
    }
  };

  const tap = (a: OrbitApp) => {
    if (a.ghost) return void connect(a.id, { url: a.url });
    if (!a.shared && !a.connected && a.needs?.length) return setPending({ id: a.id, name: nameOf(a), kind: 'secret', fields: a.needs.map((k) => ({ key: k, label: k === 'token' ? 'Access token' : k })) });
    setPicked(picked === a.id ? null : a.id);
  };
  const pickedApp = apps.find((a) => a.id === picked) ?? null;
  const n = orbit.length;

  return (
    <Modal onClose={close} className="apps-modal">
      <div className="apps-head">
        <div className="apps-title">
          <h2>Connect to Apps</h2>
          <span className="small dim">Give {agent ? agent.name : 'your agents'} access to the tools you already use.</span>
        </div>
        <span className="grow" />
        <IconButton icon="close" tip="Close" size={28} onClick={close} />
      </div>
      <div className="apps-targets" role="tablist" aria-label="Which agents">
        <button type="button" role="tab" aria-selected={target === 'all'} className={`apps-target ${target === 'all' ? 'on' : ''}`} onClick={() => pick('all')}>All agents</button>
        {list.map((a) => (
          <button key={a.id} type="button" role="tab" aria-selected={target === a.id} className={`apps-target ${target === a.id ? 'on' : ''}`} onClick={() => pick(a.id)}>
            <Avatar shape={a.shape} color={a.color} status="idle" size={16} />
            {a.name}
          </button>
        ))}
      </div>

      <div className={`apps-stage ${apps.length ? '' : 'is-empty'}`}>
        <div className="apps-ring outer" />
        <div className="apps-ring inner" />
        <Faces agent={agent} list={list} />
        <div className="apps-orbit">
          {orbit.map((a, i) => (
            <div key={a.id} className="apps-slot" style={{ '--a': `${(360 / n) * i - 90}deg` } as CSSProperties}>
              <div className="apps-upright">
                <button
                  type="button"
                  className={['apps-tile', a.ghost && 'ghost', !a.ghost && !a.connected && 'setup', a.shared && 'shared', entering === a.id && 'entering', picked === a.id && 'on'].filter(Boolean).join(' ')}
                  title={a.ghost ? `Connect ${nameOf(a)}` : `${nameOf(a)}${a.shared ? ' · for all agents' : a.connected ? '' : ' · not finished'}`}
                  disabled={busy === a.id}
                  onClick={() => tap(a)}
                >
                  {iconFor(a, 44)}
                  {(busy === a.id || (pending?.id === a.id && pending.kind === 'signin')) && <span className="apps-tile-wait"><Spinner size={11} /></span>}
                  {!a.ghost && !a.connected && pending?.id !== a.id && <span className="apps-tile-dot" />}
                </button>
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="apps-caption">
        {pickedApp ? (
          <div className="apps-picked">
            {iconFor(pickedApp, 26)}
            <div className="apps-picked-text">
              <b>{nameOf(pickedApp)}</b>
              <span className="tiny dim ellipsis">{pickedApp.shared ? 'Connected for all agents' : pickedApp.local ? 'Runs on the agent’s computer' : pickedApp.connected ? pickedApp.url : 'Not finished connecting'}</span>
            </div>
            <span className="grow" />
            {pickedApp.shared ? (
              <button type="button" className="button compact" onClick={() => pick('all')}>Manage in All agents</button>
            ) : (
              <button type="button" className="button compact apps-disconnect" disabled={busy === pickedApp.id} onClick={async () => {
                setBusy(pickedApp.id);
                try {
                  await disconnectApp(target, pickedApp.id);
                  setPicked(null);
                } catch (e) {
                  setErr(message(e));
                } finally {
                  setBusy(null);
                }
              }}>Disconnect</button>
            )}
          </div>
        ) : (
          <span className="small dim">
            {apps.length === 0 ? 'Nothing connected yet. Pick an app and it joins the orbit.' : agent ? `${agent.name} can use ${apps.length} app${apps.length === 1 ? '' : 's'}. Click one to manage it.` : `All your agents can use ${apps.length === 1 ? 'this app' : `these ${apps.length} apps`}. Click one to manage it.`}
          </span>
        )}
      </div>
      {err && <p className="tiny agent-err apps-err">{err}</p>}

      <div className="apps-section-head">
        <b>Suggested</b>
        <span className="grow" />
        <button type="button" className="apps-add" onClick={() => setCustom(true)}><Icon name="plus" size={13} />Add custom MCP</button>
      </div>
      <div className="apps-grid">
        {SUGGESTIONS.map((s) => {
          const mine = own.find((a) => a.id === s.id);
          const viaAll = !mine && agent && shared.some((a) => a.id === s.id && a.connected);
          const state = mine?.connected ? 'on' : viaAll ? 'all' : pending?.id === s.id || busy === s.id ? 'busy' : 'off';
          return (
            <button key={s.id} type="button" className={`apps-suggest ${state}`} disabled={state === 'on' || state === 'all' || state === 'busy'} onClick={() => void connect(s.id, { url: s.url })}>
              <AppIcon id={s.id} name={s.name} site={s.site} size={32} />
              <span className="apps-suggest-text">
                <b>{s.name}</b>
                <span className="tiny dim">{s.about}</span>
              </span>
              <span className="apps-suggest-state">
                {state === 'on' ? <><Icon name="check" size={13} />Connected</> : state === 'all' ? 'All agents' : state === 'busy' ? <Spinner size={12} /> : 'Connect'}
              </span>
            </button>
          );
        })}
      </div>

      {custom && <CustomForm busy={!!busy} onCancel={() => setCustom(false)} onSubmit={(name, body) => connect(name, body)} />}
      {pending && (
        <NeedPopup
          pending={pending}
          onCancel={() => setPending(null)}
          onSaved={(id) => {
            setPending(null);
            arrive(id);
          }}
          target={target}
        />
      )}
    </Modal>
  );
}

/** The agents in the middle of the orbit: one, big, or all of them huddled together. */
function Faces({ agent, list }: { agent: Agent | null; list: Agent[] }) {
  if (agent) return <div className="apps-faces"><Avatar shape={agent.shape} color={agent.color} status={agent.status} size={84} /></div>;
  if (!list.length) return <div className="apps-faces"><Avatar shape="blob" color="#7c5cff" status="idle" size={76} /></div>;
  const size = list.length <= 2 ? 58 : list.length <= 4 ? 46 : 36;
  const r = list.length === 1 ? 0 : list.length <= 3 ? size * 0.6 : size * 0.92;
  return (
    <div className="apps-faces">
      {list.map((a, i) => {
        const t = (2 * Math.PI * i) / list.length - Math.PI / 2;
        return (
          <span key={a.id} className="apps-face" style={{ transform: `translate(${Math.cos(t) * r}px, ${Math.sin(t) * r}px)` }}>
            <Avatar shape={a.shape} color={a.color} status={a.status} size={size} title={a.name} />
          </span>
        );
      })}
    </div>
  );
}

/** Over the popup: finish signing in in the browser, or type in what the app needs. */
function NeedPopup({ pending, target, onCancel, onSaved }: { pending: Pending; target: AppTarget; onCancel: () => void; onSaved: (id: string) => void }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const fields = pending.kind === 'secret' ? pending.fields : [];
  const ready = fields.every((f) => f.optional || values[f.key]?.trim());
  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      const app = await appSecret(target, pending.id, values);
      onSaved(app.id);
    } catch (e) {
      setErr(message(e));
    } finally {
      setBusy(false);
    }
  };
  const site = siteOf({ id: pending.id, url: pending.kind === 'signin' ? pending.url : '' });
  return (
    <div className="apps-need-back" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="apps-need" role="dialog" aria-label={`Connect ${pending.name}`}>
        <div className="apps-need-head">
          <AppIcon id={pending.id} name={pending.name} site={site} size={36} />
          <div className="apps-need-title">
            <b>{pending.kind === 'signin' ? `Sign in to ${pending.name}` : `Connect ${pending.name}`}</b>
            <span className="tiny dim">{pending.kind === 'signin' ? 'A sign-in page opened in your browser' : fields.length === 1 ? 'It needs one thing from you' : 'It needs a few things from you'}</span>
          </div>
        </div>
        {pending.kind === 'signin' ? (
          <>
            <div className="apps-need-wait"><Spinner size={14} /><span className="small dim">Waiting for you to finish signing in…</span></div>
            <div className="apps-need-actions">
              <button type="button" className="apps-link" onClick={() => openExternal(pending.url)}>Open the page again</button>
              <span className="grow" />
              <button type="button" className="button compact" onClick={onCancel}>Close</button>
            </div>
          </>
        ) : (
          <>
            {fields.map((f, i) => (
              <label key={f.key} className="agent-secret-field">
                <span>{f.key === 'token' ? 'Access token' : f.label}{f.optional ? <i className="dim"> (optional)</i> : null}</span>
                {f.hint && <span className="tiny dim">{f.hint}</span>}
                <input autoFocus={i === 0} type="password" autoComplete="off" spellCheck={false} value={values[f.key] ?? ''} onChange={(e) => setValues({ ...values, [f.key]: e.target.value })} onKeyDown={(e) => e.key === 'Enter' && ready && !busy && void save()} />
              </label>
            ))}
            <span className="tiny dim apps-need-note"><Icon name="lock" size={11} />Kept with this connection on ScoutCode’s servers. Your agents never see it.</span>
            {err && <p className="tiny agent-err">{err}</p>}
            <div className="apps-need-actions">
              <span className="grow" />
              <button type="button" className="button compact" onClick={onCancel}>Later</button>
              <button type="button" className="button compact primary" disabled={busy || !ready} onClick={() => void save()}>{busy ? <Spinner /> : 'Connect'}</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Any MCP server: a remote URL (streamable HTTP), or a command that starts one on the agents' computers. */
function CustomForm({ busy, onCancel, onSubmit }: { busy: boolean; onCancel: () => void; onSubmit: (name: string, body: { url?: string; auth?: 'token'; command?: string; env_needed?: string[] }) => void }) {
  const [kind, setKind] = useState<'url' | 'command'>('url');
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [token, setToken] = useState(false);
  const [command, setCommand] = useState('');
  const [env, setEnv] = useState('');
  const ready = !!name.trim() && (kind === 'url' ? /^https:\/\/\S+$/.test(url.trim()) : !!command.trim());
  const submit = () => {
    if (!ready || busy) return;
    if (kind === 'url') onSubmit(name.trim(), { url: url.trim(), ...(token ? { auth: 'token' as const } : {}) });
    else onSubmit(name.trim(), { command: command.trim(), env_needed: env.split(/[\s,]+/).filter(Boolean) });
  };
  const site = kind === 'url' && /^https:\/\/[^/\s]+\.[^/\s]+/.test(url.trim()) ? siteOf({ id: '', url: url.trim() }) : null;
  return (
    <div className="apps-need-back" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="apps-need apps-custom" role="dialog" aria-label="Add a custom MCP server">
        <div className="apps-need-head">
          <AppIcon id="custom" name={name || '?'} site={site} size={36} />
          <div className="apps-need-title">
            <b>Add custom MCP</b>
            <span className="tiny dim">Any MCP server, by address or by the command that starts it</span>
          </div>
        </div>
        <div className="apps-kind">
          <button type="button" className={kind === 'url' ? 'on' : ''} onClick={() => setKind('url')}>Remote URL</button>
          <button type="button" className={kind === 'command' ? 'on' : ''} onClick={() => setKind('command')}>Local command</button>
        </div>
        <label className="agent-secret-field">
          <span>Name</span>
          <input autoFocus value={name} maxLength={30} placeholder="e.g. my-crm" onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && submit()} />
        </label>
        {kind === 'url' ? (
          <>
            <label className="agent-secret-field">
              <span>Server URL</span>
              <input value={url} spellCheck={false} placeholder="https://mcp.example.com/mcp" onChange={(e) => setUrl(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && submit()} />
              <span className="tiny dim">Its streamable HTTP address. If it signs in with OAuth, your browser opens to sign in.</span>
            </label>
            <label className="apps-check">
              <input type="checkbox" checked={token} onChange={(e) => setToken(e.target.checked)} />
              <span className="small">It takes an access token or API key instead of signing in</span>
            </label>
          </>
        ) : (
          <>
            <label className="agent-secret-field">
              <span>Command</span>
              <input value={command} spellCheck={false} placeholder="npx -y @modelcontextprotocol/server-github" onChange={(e) => setCommand(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && submit()} />
              <span className="tiny dim">Starts the server over stdio on the agent’s cloud computer.</span>
            </label>
            <label className="agent-secret-field">
              <span>Secret environment variables <i className="dim">(optional)</i></span>
              <input value={env} spellCheck={false} placeholder="GITHUB_PERSONAL_ACCESS_TOKEN" onChange={(e) => setEnv(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && submit()} />
              <span className="tiny dim">Names only, separated by commas. You type the values in next.</span>
            </label>
          </>
        )}
        <div className="apps-need-actions">
          <span className="grow" />
          <button type="button" className="button compact" onClick={onCancel}>Cancel</button>
          <button type="button" className="button compact primary" disabled={!ready || busy} onClick={submit}>{busy ? <Spinner /> : 'Connect'}</button>
        </div>
      </div>
    </div>
  );
}
