// Local models with llama.cpp: installing it, getting models, how it runs them, and what small models are given.
// Settings > Providers and onboarding both show this.
import { useState } from 'react';
import { act, gb, invoke, type LocalSettings } from './api';
import { useView } from './store';
import { Spinner, Toggle } from './widgets';

export const LOCAL_NOTE = 'Runs models on this PC with llama.cpp. Free, private and offline: nothing you send leaves your computer. Needs enough memory for the model, and a GPU makes it much faster.';

const CONTEXTS = [4096, 8192, 16384, 32768, 65536];
const THINKING = [{ tokens: 0, name: 'No thinking' }, { tokens: 512, name: '512 tokens' }, { tokens: 1024, name: '1,024 tokens' }, { tokens: 4096, name: '4,096 tokens' }, { tokens: -1, name: 'No limit' }];
const PRESETS: { id: LocalSettings['tools']; name: string; about: string }[] = [
  { id: 'none', name: 'None', about: 'Chat only. The model gets no tools.' },
  { id: 'essential', name: 'Essential', about: 'Read, search, edit files and run commands: 8 tools. Best for small models.' },
  { id: 'standard', name: 'Standard', about: 'Adds web search, background shells, todos and questions: 15 tools.' },
  { id: 'full', name: 'Full', about: 'Every tool, with sub-agents and your MCP servers. For large models with a big context.' },
];

function pct(done: number, total: number): string {
  return total ? `${Math.floor((done / total) * 100)}%` : gb(done);
}

function Row({ title, sub, children }: { title: string; sub?: string; children?: React.ReactNode }) {
  return (
    <div className="setting-row">
      <div className="grow min0"><div className="setting-title">{title}</div>{sub && <div className="small dim pre-wrap">{sub}</div>}</div>
      {children && <div className="row setting-control">{children}</div>}
    </div>
  );
}

/** Installing llama.cpp: the build for this PC's GPU. */
function Engine() {
  const local = useView('local');
  const chosen = local.backends.find((b) => b.id === local.backend);
  const inst = local.installing;
  if (!local.backends.length) return <p className="small red">llama.cpp has no build for this computer.</p>;
  return (
    <>
      <Row title="Runs on" sub={chosen?.about}>
        <select className="input select" value={local.backend ?? ''} disabled={!!inst} onChange={(e) => act('setLocal', { patch: { backend: e.target.value } })}>
          {local.backends.map((b) => <option key={b.id} value={b.id}>{b.name}{b.installed ? '' : ` (${gb(b.size)} download)`}</option>)}
        </select>
      </Row>
      {inst ? (
        <div className="row">
          <Spinner />
          <span className="small grow">Installing llama.cpp {local.build}… {pct(inst.done, inst.total)}</span>
          <button type="button" className="button" onClick={() => act('localCancelInstall')}>Cancel</button>
        </div>
      ) : chosen && !chosen.installed ? (
        <div><button type="button" className="button primary" onClick={() => act('localInstall', { backend: chosen.id })}>Install llama.cpp ({gb(chosen.size)})</button></div>
      ) : chosen && (
        <Row title="llama.cpp" sub={`Build ${local.build}, installed`}>
          <button type="button" className="button" onClick={() => act('localRemoveBackend', { backend: chosen.id })}>Uninstall</button>
        </Row>
      )}
      {local.installError && <div className="small red">{local.installError}</div>}
    </>
  );
}

/** The user's models, and the ones Scout offers to download. */
function Models() {
  const cfg = useView('cfg').local;
  const local = useView('local');
  const [error, setError] = useState<string | null>(null);
  const addFile = () => { setError(null); invoke('local_add_file').catch((e) => setError(String(e))); };
  return (
    <>
      <div className="setting-title">Your models</div>
      {cfg.models.length === 0 && <p className="small dim">None yet. Download one below, or add a GGUF file you already have.</p>}
      {cfg.models.map((m) => (
        <Row key={m.id} title={m.name} sub={`${gb(m.size)} · ${m.path}`}>
          <button type="button" className="button" onClick={() => act('localRemove', { id: m.id })}>Remove</button>
        </Row>
      ))}
      <div><button type="button" className="button" onClick={addFile}>Add a GGUF file…</button></div>
      {error && <div className="small red">{error}</div>}
      <div className="setting-title">Download a model</div>
      {local.curated.filter((c) => !c.have).map((c) => {
        const d = local.downloads[c.id];
        const err = local.downloadErrors[c.id];
        return (
          <div key={c.id} className="stack">
            <Row title={`${c.name} · ${gb(c.size)}`} sub={c.about}>
              {d ? (
                <>
                  <Spinner />
                  <span className="small">{pct(d.done, d.total)}</span>
                  <button type="button" className="button" onClick={() => act('localCancelDownload', { id: c.id })}>Pause</button>
                </>
              ) : <button type="button" className="button" onClick={() => act('localDownload', { id: c.id })}>Download</button>}
            </Row>
            {err && <div className="small red">{err}</div>}
          </div>
        );
      })}
      <p className="small dim">From Hugging Face, checked against their published hashes and saved in your .scoutcode folder. A paused download carries on where it stopped.</p>
    </>
  );
}

/** How llama.cpp runs the model: on the GPU, and with how much context. */
function Running() {
  const cfg = useView('cfg').local;
  const { server } = useView('local');
  const auto = cfg.gpuLayers === null;
  // The model's layer count, once llama.cpp has said; until then a generous upper end.
  const max = server.offloaded?.[1] ?? 100;
  const name = cfg.models.find((m) => m.id === server.model)?.name ?? server.model;
  const status = server.state === 'ready' ? (server.cpuFaster ? `${name} is loaded on the CPU, which Scout measured as faster than this PC's GPU.` : `${name} is loaded${server.offloaded ? `, ${server.offloaded[0]} of ${server.offloaded[1]} layers on the GPU` : ''}.`)
    : server.state === 'loading' ? `Loading ${name}…`
    : server.state === 'error' ? server.error ?? 'llama.cpp stopped.'
    : 'Not running. It starts when you choose a local model.';
  return (
    <>
      <Row title="Status" sub={status}>
        {server.state === 'loading' && <Spinner />}
        {(server.state === 'ready' || server.state === 'loading') && <button type="button" className="button" onClick={() => act('localUnload')}>Unload</button>}
      </Row>
      <Row title="GPU offload" sub={auto ? 'Auto: llama.cpp puts as many layers on the GPU as fit, and runs the rest on the CPU. The first time a model loads, Scout times it on both and keeps the faster.' : `${cfg.gpuLayers} layers on the GPU, the rest on the CPU. Fewer layers use less GPU memory but run slower.`}>
        <span className="small dim">Auto</span>
        <Toggle label="Automatic GPU offload" on={auto} onChange={(on) => act('setLocal', { patch: { gpuLayers: on ? null : Math.min(server.offloaded?.[0] ?? 20, max) } })} />
      </Row>
      {!auto && (
        <div className="row">
          <input type="range" className="grow" min={0} max={max} value={Math.min(cfg.gpuLayers ?? 0, max)} aria-label="GPU layers"
            onChange={(e) => act('setLocal', { patch: { gpuLayers: Number(e.target.value) } })} />
          <span className="small mono">{cfg.gpuLayers}/{server.offloaded ? max : '?'}</span>
        </div>
      )}
      <Row title="Context size" sub="How much of the chat the model sees at once, in tokens. More needs more memory. Scout summarizes the chat before it fills up.">
        <select className="input select" value={cfg.context} onChange={(e) => act('setLocal', { patch: { context: Number(e.target.value) } })}>
          {CONTEXTS.map((c) => <option key={c} value={c}>{c / 1024}K</option>)}
        </select>
      </Row>
      <Row title="Thinking limit" sub="The most tokens a model thinks for before it has to answer. Less is faster; more can help on hard problems.">
        <select className="input select" value={cfg.reasoningBudget} onChange={(e) => act('setLocal', { patch: { reasoningBudget: Number(e.target.value) } })}>
          {!THINKING.some((t) => t.tokens === cfg.reasoningBudget) && <option value={cfg.reasoningBudget}>{cfg.reasoningBudget} tokens</option>}
          {THINKING.map((t) => <option key={t.tokens} value={t.tokens}>{t.name}</option>)}
        </select>
      </Row>
    </>
  );
}

/** What a local model is given: fewer tools and a shorter prompt keep small models on track. */
function SmallModels() {
  const cfg = useView('cfg').local;
  const preset = PRESETS.find((p) => p.id === cfg.tools) ?? PRESETS[1];
  return (
    <>
      <Row title="Tools" sub={preset.about}>
        <select className="input select" value={preset.id} onChange={(e) => act('setLocal', { patch: { tools: e.target.value } })}>
          {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </Row>
      <Row title="One tool call at a time" sub="The model makes a single call per reply and sees its result before the next. Small models get confused by several at once.">
        <Toggle label="One tool call at a time" on={cfg.singleCall} onChange={(on) => act('setLocal', { patch: { singleCall: on } })} />
      </Row>
      <Row title="Short system prompt" sub="Plain rules instead of Scout's full instructions, with your project's instruction files and memory cut down to fit a small context.">
        <Toggle label="Short system prompt" on={cfg.shortPrompt} onChange={(on) => act('setLocal', { patch: { shortPrompt: on } })} />
      </Row>
    </>
  );
}

/** Everything about local models, for Settings and onboarding. */
export function LocalSetup({ compact = false }: { compact?: boolean }) {
  const local = useView('local');
  const installed = local.backends.some((b) => b.id === local.backend && b.installed);
  return (
    <div className="stack">
      <p className="small dim">{LOCAL_NOTE}</p>
      <Engine />
      {installed && <Models />}
      {installed && !compact && (
        <>
          <div className="setting-title">How it runs</div>
          <Running />
          <div className="setting-title">For small models</div>
          <SmallModels />
        </>
      )}
    </div>
  );
}

/** The provider row's status line. */
export function localStatus(cfg: LocalSettings, installed: boolean): string {
  if (!installed) return 'Not installed';
  return cfg.models.length ? `${cfg.models.length} model${cfg.models.length === 1 ? '' : 's'}` : 'No models yet';
}
