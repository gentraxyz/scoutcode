// Local models with llama.cpp. Most people only ever see QuickSetup: Scout picks the model that suits this PC, and one
// button installs llama.cpp, downloads it and moves the chat to it. Getting more models (Scout's list, or any GGUF on
// Hugging Face) is in the model menu and Settings; how llama.cpp runs is under Advanced in Settings.
import { useState } from 'react';
import { act, gb, invoke, type Fit, type HfFile, type HfRepo, type LocalSettings, type LocalView } from './api';
import { useView } from './store';
import { Icon, Spinner, Toggle } from './widgets';

export const LOCAL_NOTE = 'Runs models on this PC with llama.cpp. Free, private and offline: nothing you send leaves your computer. Needs enough memory for the model, and a GPU makes it much faster.';

const CONTEXTS = [4096, 8192, 16384, 32768, 65536];
const THINKING = [{ tokens: 0, name: 'No thinking' }, { tokens: 512, name: '512 tokens' }, { tokens: 1024, name: '1,024 tokens' }, { tokens: 4096, name: '4,096 tokens' }, { tokens: -1, name: 'No limit' }];
const PRESETS: { id: LocalSettings['tools']; name: string; about: string }[] = [
  { id: 'none', name: 'None', about: 'Chat only. The model gets no tools.' },
  { id: 'essential', name: 'Essential', about: 'Read, search, edit files and run commands: 8 tools. Best for small models.' },
  { id: 'standard', name: 'Standard', about: 'Adds web search, background shells, todos and questions: 15 tools.' },
  { id: 'full', name: 'Full', about: 'Every tool, with sub-agents and your MCP servers. For large models with a big context.' },
];
const FIT_LABEL: Record<Fit, string> = { good: 'Runs well', slow: 'Slow on this PC', big: 'Too big for this PC' };
const FIT_ORDER: Record<Fit, number> = { good: 0, slow: 1, big: 2 };

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

function Bar({ done, total }: { done: number; total: number }) {
  return <div className="local-bar" role="progressbar" aria-valuenow={total ? Math.round((done / total) * 100) : 0} aria-valuemin={0} aria-valuemax={100}><i style={{ width: total ? `${(done / total) * 100}%` : '0%' }} /></div>;
}

function FitPill({ fit, best }: { fit: Fit; best?: boolean }) {
  return <span className={`fit-pill ${best ? 'best' : fit}`}>{best ? 'Best for this PC' : FIT_LABEL[fit]}</span>;
}

/** "this PC (NVIDIA GeForce RTX 3060, 12 GB)", or with no GPU, its memory. */
function thisPc(hw: LocalView['hardware']): string {
  if (hw.shared) return `this Mac (${hw.ramGb} GB of memory)`;
  if (hw.gpuGb > 0) return `this PC (${hw.gpuName ? `${hw.gpuName}, ` : ''}${hw.gpuGb} GB)`;
  return hw.ramGb ? `this PC (no GPU, ${hw.ramGb} GB of memory)` : 'this PC';
}

function engineInstalled(local: LocalView): boolean {
  return local.backends.some((b) => b.id === local.backend && b.installed);
}

/** The size of llama.cpp's download when it isn't installed yet, else 0. */
function engineSize(local: LocalView): number {
  return engineInstalled(local) ? 0 : local.backends.find((b) => b.id === local.backend)?.size ?? 0;
}

/** Quick setup's progress: one bar over llama.cpp and the model. */
function SetupProgress({ s }: { s: NonNullable<LocalView['setup']> }) {
  return (
    <div className="local-quick stack">
      <div className="row">
        <Spinner />
        <b className="strong grow ellipsis">{s.phase === 'engine' ? `Getting ready for ${s.name}…` : `Downloading ${s.name}…`}</b>
        <span className="small mono">{pct(s.done, s.total)}</span>
        <button type="button" className="button compact" onClick={() => act('localCancelSetup')}>Pause</button>
      </div>
      <Bar done={s.done} total={s.total} />
      <div className="tiny dim">{gb(s.done)} of {gb(s.total)}. You can carry on with something else: it keeps going, and is ready to pick in the model menu when it's done.</div>
    </div>
  );
}

/** The one-button setup: the model Scout picked for this PC, and Set up. */
function QuickSetup() {
  const local = useView('local');
  const models = useView('cfg').local.models;
  const rec = local.curated.find((c) => c.id === local.recommended);
  if (local.setup) return <SetupProgress s={local.setup} />;
  if (!local.backends.length) return <p className="small red">llama.cpp has no build for this computer, so local models can't run on it.</p>;
  if (models.length > 0) {
    return (
      <div className="linked-line">
        <Icon name="check" size={16} />
        <div className="stack tight">
          <b className="strong">{models.length === 1 ? `${models[0].name} is ready` : `${models.length} models are ready`}</b>
          <span className="small dim">Runs on this PC. Pick it in the model menu under Local models.</span>
        </div>
      </div>
    );
  }
  if (!rec) {
    return <p className="small dim">{local.setupError ?? "This PC doesn't have enough memory for the models Scout offers. A free cloud provider will work better here."}</p>;
  }
  const size = rec.size + engineSize(local);
  return (
    <div className="local-quick stack">
      <div className="tiny dim">Scout picked this for {thisPc(local.hardware)}</div>
      <div className="row">
        <div className="grow min0 stack tight">
          <b className="strong big">{rec.name}</b>
          <span className="small dim">{rec.about}</span>
        </div>
      </div>
      <div className="row">
        <button type="button" className="button primary" onClick={() => act('localSetup', { model: null })}>
          {local.setupError ? 'Try again' : 'Set up'}
        </button>
        <span className="small dim">{gb(size)} download, once</span>
      </div>
      {local.setupError && <div className="small red">{local.setupError}</div>}
    </div>
  );
}

/** Scout's own models, best for this PC first. `viaSetup` moves the chat to the one picked, as onboarding wants. */
function ModelList({ viaSetup = false }: { viaSetup?: boolean }) {
  const local = useView('local');
  const busy = !!local.setup && !engineInstalled(local);
  const list = local.curated
    .filter((c) => !c.have)
    .sort((a, b) => (a.id === local.recommended ? -1 : b.id === local.recommended ? 1 : FIT_ORDER[a.fit] - FIT_ORDER[b.fit] || a.size - b.size));
  if (!list.length) return <p className="small dim">You have all of Scout's models.</p>;
  return (
    <>
      {list.map((c) => {
        const inSetup = local.setup?.model === c.id ? local.setup : null;
        const d = local.downloads[c.id] ?? inSetup;
        const err = local.downloadErrors[c.id];
        const get = () => (viaSetup ? act('localSetup', { model: c.id }) : act('localDownload', { id: c.id }));
        return (
          <div key={c.id} className={`stack${c.fit === 'big' ? ' local-dim' : ''}`}>
            <Row title={`${c.name} · ${gb(c.size)}`} sub={c.about}>
              <FitPill fit={c.fit} best={c.id === local.recommended} />
              {d ? (
                <>
                  <Spinner />
                  <span className="small">{pct(d.done, d.total)}</span>
                  <button type="button" className="button" onClick={() => (inSetup ? act('localCancelSetup') : act('localCancelDownload', { id: c.id }))}>Pause</button>
                </>
              ) : <button type="button" className="button" disabled={busy} onClick={get}>Download</button>}
            </Row>
            {err && <div className="small red">{err}</div>}
          </div>
        );
      })}
    </>
  );
}

/** The file to offer first: a Q4_K_M that fits, else the largest that runs well, else the smallest. */
function defaultFile(files: HfFile[]): string | null {
  const ok = files.filter((f) => !f.split);
  const good = ok.filter((f) => f.fit === 'good');
  return (good.find((f) => f.quant === 'Q4_K_M') ?? good[good.length - 1] ?? ok[0])?.path ?? null;
}

/** Any GGUF on Hugging Face, from a link or "owner/name". */
function HuggingFace() {
  const [input, setInput] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [repo, setRepo] = useState<HfRepo | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const find = () => {
    if (!input.trim() || loading) return;
    setLoading(true); setError(null); setRepo(null);
    invoke<HfRepo>('local_hf_files', { input })
      .then((r) => { setRepo(r); setPicked(r.file ?? defaultFile(r.files)); })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };
  const file = repo?.files.find((f) => f.path === picked);
  return (
    <details className="local-more">
      <summary className="small">Any model from Hugging Face</summary>
      <div className="stack">
        <div className="small dim">Paste a link to a GGUF model, like huggingface.co/unsloth/Qwen3.5-4B-GGUF.</div>
        <div className="row">
          <input className="input grow" placeholder="huggingface.co/owner/model-GGUF" value={input} onChange={(e) => setInput(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && find()} />
          <button type="button" className="button" disabled={!input.trim() || loading} onClick={find}>{loading ? <Spinner size={12} /> : 'Find files'}</button>
        </div>
        {error && <div className="small red">{error}</div>}
        {repo && (
          <>
            <div className="row">
              <select className="input select grow" aria-label="File" value={picked ?? ''} onChange={(e) => setPicked(e.target.value)}>
                {repo.files.map((f) => (
                  <option key={f.path} value={f.path} disabled={f.split}>
                    {f.quant ?? f.path} · {gb(f.size)} · {f.split ? 'split into parts, not supported yet' : FIT_LABEL[f.fit]}
                  </option>
                ))}
              </select>
              <button type="button" className="button primary" disabled={!file}
                onClick={() => file && act('localDownloadHf', { repo: repo.repo, path: file.path, sha256: file.sha256, size: file.size })}>
                Download
              </button>
            </div>
            {file && <div className="tiny dim mono ellipsis" title={file.path}>{repo.repo}/{file.path}</div>}
            <div className="tiny dim">Scout hasn't tried this model. To edit files it has to support tool calls; a model made for chat may only talk.</div>
          </>
        )}
      </div>
    </details>
  );
}

/** The user's models, and downloads of models that aren't in Scout's list. */
function YourModels() {
  const cfg = useView('cfg').local;
  const local = useView('local');
  const curated = new Set(local.curated.map((c) => c.id));
  const other = Object.entries(local.downloads).filter(([id]) => !curated.has(id));
  const otherErrors = Object.entries(local.downloadErrors).filter(([id]) => !curated.has(id));
  if (!cfg.models.length && !other.length && !otherErrors.length) return null;
  return (
    <>
      <div className="setting-title">Your models</div>
      {cfg.models.map((m) => (
        <Row key={m.id} title={m.name} sub={`${gb(m.size)} · ${m.path}`}>
          <button type="button" className="button" onClick={() => act('localRemove', { id: m.id })}>Remove</button>
        </Row>
      ))}
      {other.map(([id, d]) => (
        <Row key={id} title={d.name} sub={`Downloading, ${gb(d.done)} of ${gb(d.total)}`}>
          <Spinner />
          <span className="small">{pct(d.done, d.total)}</span>
          <button type="button" className="button" onClick={() => act('localCancelDownload', { id })}>Pause</button>
        </Row>
      ))}
      {otherErrors.map(([id, e]) => <div key={id} className="small red">{e}</div>)}
    </>
  );
}

/** Scout's list, Hugging Face, and a file the user already has. `setup` shows quick setup's progress, where no
 *  QuickSetup card already does. */
function GetMore({ setup }: { setup: boolean }) {
  const [error, setError] = useState<string | null>(null);
  const addFile = () => { setError(null); invoke('local_add_file').catch((e) => setError(String(e))); };
  const local = useView('local');
  return (
    <>
      <div className="setting-title">Get more models</div>
      {setup && local.setup && <SetupProgress s={local.setup} />}
      {setup && local.setupError && !local.setup && <div className="small red">{local.setupError}</div>}
      <ModelList />
      <HuggingFace />
      <div className="row">
        <button type="button" className="button" onClick={addFile}>Add a GGUF file…</button>
        <span className="tiny dim">A model file already on this PC. It stays where it is.</span>
      </div>
      {error && <div className="small red">{error}</div>}
      <p className="tiny dim">Downloads come from Hugging Face, are checked against their published hashes and are saved in your .scoutcode folder. A paused download carries on where it stopped.</p>
    </>
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
        <select className="input select" value={local.backend ?? ''} disabled={!!inst || !!local.setup} onChange={(e) => act('setLocal', { patch: { backend: e.target.value } })}>
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
        <div><button type="button" className="button" disabled={!!local.setup} onClick={() => act('localInstall', { backend: chosen.id })}>Install llama.cpp ({gb(chosen.size)})</button></div>
      ) : chosen && (
        <Row title="llama.cpp" sub={`Build ${local.build}, installed`}>
          <button type="button" className="button" onClick={() => act('localRemoveBackend', { backend: chosen.id })}>Uninstall</button>
        </Row>
      )}
      {local.installError && <div className="small red">{local.installError}</div>}
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

/**
 * Local models, for onboarding, the model menu and Settings.
 * - onboarding: quick setup, with Scout's other models folded away.
 * - menu: quick setup until there's a model, then getting more.
 * - settings: the user's models, getting more, and llama.cpp's own settings under Advanced.
 */
export function LocalSetup({ mode }: { mode: 'onboarding' | 'menu' | 'settings' }) {
  const cfg = useView('cfg').local;
  const local = useView('local');
  const have = cfg.models.length > 0;
  if (mode === 'onboarding' || (mode === 'menu' && !have)) {
    return (
      <div className="stack">
        <p className="small dim">{LOCAL_NOTE}</p>
        <QuickSetup />
        {!have && (
          <details className="local-more">
            <summary className="small">Choose a different model</summary>
            <div className="stack"><ModelList viaSetup /></div>
          </details>
        )}
      </div>
    );
  }
  if (mode === 'menu') return <div className="stack"><GetMore setup /></div>;
  return (
    <div className="stack">
      <p className="small dim">{LOCAL_NOTE}</p>
      {have ? <YourModels /> : <QuickSetup />}
      <GetMore setup={have} />
      <details className="local-more">
        <summary className="small">Advanced</summary>
        <div className="stack">
          <div className="setting-title">llama.cpp</div>
          <Engine />
          {engineInstalled(local) && (
            <>
              <div className="setting-title">How it runs</div>
              <Running />
              <div className="setting-title">For small models</div>
              <SmallModels />
            </>
          )}
        </div>
      </details>
    </div>
  );
}

/** The provider row's status line. */
export function localStatus(cfg: LocalSettings, local: LocalView): string {
  if (local.setup) return `Setting up… ${pct(local.setup.done, local.setup.total)}`;
  if (cfg.models.length) return `${cfg.models.length} model${cfg.models.length === 1 ? '' : 's'}`;
  return engineInstalled(local) ? 'No models yet' : 'Not set up';
}
