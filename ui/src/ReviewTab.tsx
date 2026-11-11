import { useEffect, useState } from 'react';
import { act, invoke, type ModelRef, type Review, type ReviewFinding } from './api';
import { logo } from './Accounts';
import { ModelPicker, useModelName } from './ModelMenu';
import { useCore, useView } from './store';
import { fileTarget, openFileAt } from './Transcript';
import { Icon, IconButton, SectionLabel, Spinner, Toggle } from './widgets';

type Scope = 'uncommitted' | 'branch' | 'lastCommit' | 'paths';
type Strictness = 'serious' | 'balanced' | 'thorough';
type FocusId = 'bugs' | 'security' | 'performance' | 'tests' | 'readability';
type ReviewSettings = {
  scope: Scope;
  base: string;
  paths: string;
  focus: Record<FocusId, boolean>;
  strictness: Strictness;
  /** Null for the free models explore uses. */
  reviewer: ModelRef | null;
  extra: string;
};

const FOCUS: [FocusId, string, string][] = [
  ['bugs', 'Bugs and correctness', 'logic errors, edge cases, broken error handling, race conditions'],
  ['security', 'Security', 'injection, secrets, unsafe input handling, permissions'],
  ['performance', 'Performance', 'needless work, slow loops, extra requests, memory'],
  ['tests', 'Tests', 'changed behaviour without tests, tests that can’t fail'],
  ['readability', 'Readability', 'naming, dead code, duplication, simpler ways to write it'],
];

const SCOPES: [Scope, string][] = [
  ['uncommitted', 'Uncommitted changes'],
  ['branch', 'Branch vs base'],
  ['lastCommit', 'Last commit'],
  ['paths', 'Some files'],
];

const STRICTNESS: [Strictness, string, string][] = [
  ['serious', 'Serious only', 'Only problems it’s confident will cause real failures.'],
  ['balanced', 'Balanced', 'Real problems plus clear improvements worth making.'],
  ['thorough', 'Thorough', 'Everything it notices, small and uncertain points included.'],
];

const DEFAULTS: ReviewSettings = {
  scope: 'uncommitted',
  base: 'main',
  paths: '',
  focus: { bugs: true, security: true, performance: false, tests: false, readability: false },
  strictness: 'balanced',
  reviewer: null,
  extra: '',
};

/** Providers the review agent can call itself. Claude Code, Codex and the other apps run their own agent. */
const RUNS_HERE = new Set(['scoutcode', 'openrouter', 'nvidia', 'kilo', 'local']);
const SEVERITIES = ['critical', 'high', 'medium', 'low'] as const;

/** Each chat keeps its own settings; a chat without any starts from the last ones used anywhere. */
const LAST = 'scout:codeReview';

function read(key: string): ReviewSettings | null {
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? 'null');
    if (saved && typeof saved === 'object') return { ...DEFAULTS, ...saved, focus: { ...DEFAULTS.focus, ...(saved.focus ?? {}) } };
  } catch {}
  return null;
}
const load = (chat: string) => read(`${LAST}:${chat}`) ?? { ...(read(LAST) ?? DEFAULTS), extra: '', paths: '' };
function save(chat: string, s: ReviewSettings) {
  try {
    localStorage.setItem(`${LAST}:${chat}`, JSON.stringify(s));
    localStorage.setItem(LAST, JSON.stringify(s));
  } catch {}
}

function scopeLabel(s: ReviewSettings): string {
  if (s.scope === 'branch') return `Branch vs ${s.base.trim() || 'main'}`;
  if (s.scope === 'paths') return s.paths.split(/[\n,]+/).map((p) => p.trim()).filter(Boolean).join(', ');
  return SCOPES.find(([id]) => id === s.scope)![1];
}

/** What the review agent is asked to do. */
function reviewTask(s: ReviewSettings): string {
  const base = s.base.trim() || 'main';
  const what = {
    uncommitted: 'the uncommitted changes in this project (staged, unstaged and new untracked files). Start with `git status` and `git diff HEAD`, and read new files in full.',
    branch: `the changes on the current branch against \`${base}\`. Start with \`git diff ${base}...HEAD\` and \`git log --oneline ${base}..HEAD\`.`,
    lastCommit: 'the last commit. Start with `git show HEAD`.',
    paths: `these files and folders as they are now: ${s.paths.split(/[\n,]+/).map((p) => p.trim()).filter(Boolean).map((p) => `\`${p}\``).join(', ')}.`,
  }[s.scope];
  const focus = FOCUS.filter(([id]) => s.focus[id]).map(([, title, detail]) => `- ${title}: ${detail}`);
  const strict = {
    serious: 'Report only issues you are confident cause real failures, security holes or data loss. Leave out style and minor points.',
    balanced: 'Report real problems and clear improvements worth making. Leave out nitpicks.',
    thorough: 'Report everything you notice, including small points; rate uncertain ones low and say you are unsure in the detail.',
  }[s.strictness];
  const lines = [`Review ${what}`, '', `Look for:\n${focus.join('\n')}`, '', strict];
  if (s.extra.trim()) lines.push('', `From the user: ${s.extra.trim()}`);
  return lines.join('\n');
}

/** The message that hands findings to the chat's agent to fix. */
function fixMessage(findings: ReviewFinding[], model: string): string {
  const one = (f: ReviewFinding) => [`**[${f.severity}] ${f.title}** at \`${f.file}${f.line ? `:${f.line}` : ''}\``, f.detail, f.fix && `Suggested fix: ${f.fix}`].filter(Boolean).join('\n');
  const head = findings.length === 1 ? `A code review${model ? ` by ${model}` : ''} found this problem. Check it's real, then fix it. If it isn't, say why.` : `A code review${model ? ` by ${model}` : ''} found these ${findings.length} problems. Check each is real, then fix them. Say which you fixed and which you didn't, and why.`;
  return `${head}\n\n${findings.map(one).join('\n\n')}`;
}

function Choice<T extends string>({ value, options, onChange }: { value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="review-choices">
      {options.map(([id, label]) => (
        <button key={id} type="button" className={`review-choice ${value === id ? 'selected' : ''}`} onClick={() => onChange(id)}>{label}</button>
      ))}
    </div>
  );
}

function Setup({ s, set, onRun, error }: { s: ReviewSettings; set: (p: Partial<ReviewSettings>) => void; onRun: () => void; error: string }) {
  const { additionalProviders } = useCore((st) => st.statics);
  const reviewerName = useModelName(s.reviewer);
  const [skipped, setSkipped] = useState(false);
  const pick = (key: string, provider: string) => {
    const ok = RUNS_HERE.has(provider) || additionalProviders.some((a) => a.id === provider);
    setSkipped(!ok);
    if (ok) set({ reviewer: { key, provider } });
  };
  const ready = Object.values(s.focus).some(Boolean) && (s.scope !== 'paths' || !!s.paths.trim());
  return (
    <div className="stack review-setup">
      <SectionLabel>What to review</SectionLabel>
      <Choice value={s.scope} options={SCOPES} onChange={(scope) => set({ scope })} />
      {s.scope === 'branch' && (
        <label className="review-field"><span className="small dim">Base branch</span>
          <input className="input" value={s.base} placeholder="main" onChange={(e) => set({ base: e.target.value })} />
        </label>
      )}
      {s.scope === 'paths' && (
        <label className="review-field"><span className="small dim">Files or folders, one per line or separated by commas</span>
          <textarea className="input" rows={3} value={s.paths} placeholder={'src/api.ts\nsrc/lib/'} onChange={(e) => set({ paths: e.target.value })} />
        </label>
      )}

      <SectionLabel>Reviewer</SectionLabel>
      <div className="review-reviewer">
        <div className="row tight grow min0 setting-title">
          {s.reviewer && logo(s.reviewer.provider) && <img className="logo row-logo" src={logo(s.reviewer.provider)} width={13} height={13} alt="" />}
          <span className="ellipsis">{s.reviewer ? reviewerName : 'Free models (Qwen 3.8 27B first)'}</span>
        </div>
        <div className="row tight">
          {s.reviewer && <button type="button" className="button ghost compact" onClick={() => set({ reviewer: null })}>Use free</button>}
          <ModelPicker label={s.reviewer ? 'Change' : 'Pick a model'} onPick={pick} />
        </div>
      </div>
      <div className="tiny dim">{skipped ? 'That model only runs inside another app’s agent (like Claude Code or Codex), so the reviewer can’t use it.' : 'Its own agent, apart from this chat: it reads the code and runs read-only git commands, and never changes files.'}</div>

      <SectionLabel>Look for</SectionLabel>
      <div className="stack">
        {FOCUS.map(([id, title, detail]) => (
          <div key={id} className="review-row">
            <div className="grow min0"><div className="setting-title">{title}</div><div className="tiny dim">{detail}</div></div>
            <Toggle label={title} on={s.focus[id]} onChange={(on) => set({ focus: { ...s.focus, [id]: on } })} />
          </div>
        ))}
      </div>

      <SectionLabel>How strict</SectionLabel>
      <Choice value={s.strictness} options={STRICTNESS.map(([id, label]) => [id, label])} onChange={(strictness) => set({ strictness })} />
      <div className="tiny dim">{STRICTNESS.find(([id]) => id === s.strictness)?.[2]}</div>

      <label className="review-field"><span className="small dim">Anything it should know (optional)</span>
        <textarea className="input" rows={2} value={s.extra} placeholder="e.g. This adds rate limiting to login; check the Redis keys expire." onChange={(e) => set({ extra: e.target.value })} />
      </label>

      {error && <div className="small red">{error}</div>}
      <div className="row review-actions">
        <button type="button" className="button ghost" onClick={() => set({ ...DEFAULTS, reviewer: s.reviewer })}>Reset</button>
        <span className="grow" />
        <button type="button" className="button primary" disabled={!ready} onClick={onRun}><Icon name="shield" size={14} />Run review</button>
      </div>
    </div>
  );
}

function FindingCard({ f, review, cwd }: { f: ReviewFinding; review: Review; cwd: string }) {
  const [open, setOpen] = useState(f.severity === 'critical' || f.severity === 'high');
  const where = `${f.file}${f.line ? `:${f.line}` : ''}`;
  const target = fileTarget(where, cwd);
  const mark = (status: ReviewFinding['status']) => act('reviewFinding', { review: review.id, finding: f.id, status });
  const fix = async () => {
    const taken = await invoke<boolean>('send', { text: fixMessage([f], review.model), files: [], steer: false });
    if (taken) mark('sent');
  };
  return (
    <div className={`review-finding sev-${f.severity} ${f.status !== 'open' ? 'settled' : ''}`}>
      <div className="review-finding-head" onClick={() => setOpen(!open)}>
        <span className={`review-sev sev-${f.severity}`}>{f.severity}</span>
        <span className="grow min0 review-finding-title">{f.title}</span>
        {f.status === 'sent' && <span className="tiny dim">sent</span>}
        {f.status === 'dismissed' && <span className="tiny dim">dismissed</span>}
        <Icon name={open ? 'chevronUp' : 'chevronDown'} size={12} />
      </div>
      <button type="button" className="link file-link review-where" onClick={() => target && openFileAt(target)} title={`Open ${where}`}>{where}</button>
      {open && (
        <>
          {f.detail && <div className="small review-detail">{f.detail}</div>}
          {f.fix && <div className="small review-fix"><b>Fix:</b> {f.fix}</div>}
          <div className="row tight review-finding-actions">
            {f.status === 'open' ? (
              <>
                <button type="button" className="button compact" onClick={fix} title="Send it to this chat's agent to fix"><Icon name="arrowRight" size={12} />Fix in chat</button>
                <button type="button" className="button ghost compact" onClick={() => mark('dismissed')}>Dismiss</button>
              </>
            ) : (
              <button type="button" className="button ghost compact" onClick={() => mark('open')}>Reopen</button>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function ReviewResult({ review, cwd, running }: { review: Review; cwd: string; running: boolean }) {
  const isRunning = review.state === 'running';
  const open = review.findings.filter((f) => f.status === 'open');
  const sorted = [...review.findings].sort((a, b) => SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity) || Number(a.status !== 'open') - Number(b.status !== 'open'));
  const counts = SEVERITIES.map((sev) => [sev, review.findings.filter((f) => f.severity === sev).length] as const).filter(([, n]) => n > 0);
  const lastSteps = review.steps.slice(-4);
  const sendAll = async () => {
    const taken = await invoke<boolean>('send', { text: fixMessage(open, review.model), files: [], steer: false });
    if (taken) open.forEach((f) => act('reviewFinding', { review: review.id, finding: f.id, status: 'sent' }));
  };
  return (
    <div className="stack review-result">
      <div className="review-result-head">
        <div className="grow min0">
          <div className="setting-title ellipsis">{review.label}</div>
          <div className="tiny dim">{review.model || 'starting'}{review.steps.length ? ` · ${review.steps.length} step${review.steps.length === 1 ? '' : 's'}` : ''}{review.tokens ? ` · ${Math.round(review.tokens / 1000)}K tokens` : ''}</div>
        </div>
        {isRunning
          ? <button type="button" className="button ghost compact" onClick={() => act('reviewStop')}>Stop</button>
          : <IconButton icon="close" tip="Remove this review" size={24} onClick={() => act('reviewRemove', { review: review.id })} />}
      </div>

      {isRunning && (
        <div className="review-steps">
          <div className="row tight small"><Spinner />Reviewing{review.findings.length ? ` · ${review.findings.length} found so far` : '…'}</div>
          {lastSteps.map((s, i) => (
            <div key={review.steps.length - lastSteps.length + i} className={`tiny dim ellipsis review-step ${s.failed ? 'red' : ''}`}>{s.done ? '✓' : '…'} {s.tool} {s.label}</div>
          ))}
        </div>
      )}
      {review.state === 'failed' && <div className="small red">The review stopped: {review.error}</div>}
      {review.state === 'stopped' && <div className="small dim">Stopped before it finished.</div>}
      {review.verdict && <div className="small review-verdict">{review.verdict}</div>}

      {counts.length > 0 && (
        <div className="row tight review-counts">
          {counts.map(([sev, n]) => <span key={sev} className={`review-sev sev-${sev}`}>{n} {sev}</span>)}
          <span className="grow" />
          {open.length > 0 && !isRunning && <button type="button" className="button compact" onClick={sendAll} title={running ? 'Queued for when this chat finishes' : 'Send every open finding to this chat’s agent'}><Icon name="arrowRight" size={12} />Fix {open.length} in chat</button>}
        </div>
      )}
      {review.state === 'done' && review.findings.length === 0 && <div className="small dim">No problems found.</div>}
      {sorted.map((f) => <FindingCard key={f.id} f={f} review={review} cwd={cwd} />)}
    </div>
  );
}

/** Code Review, a tab in the chat's side panel. A separate review agent looks over the code, and its findings stay here. */
export function ReviewTab() {
  const chat = useView('chat');
  const { running: chatRunning } = useView('run');
  const view = useView('review');
  const chatId = view?.chat ?? chat.id ?? '';
  const [s, setS] = useState<ReviewSettings>(() => load(chatId));
  const [error, setError] = useState('');
  const reviews = view?.reviews ?? [];
  const [picked, setPicked] = useState<number | null>(null);
  const [setupOpen, setSetupOpen] = useState(reviews.length === 0);
  useEffect(() => {
    setS(load(chatId));
    setPicked(null);
    setError('');
    setSetupOpen((view?.reviews ?? []).length === 0);
  }, [chatId]); // eslint-disable-line react-hooks/exhaustive-deps
  const set = (patch: Partial<ReviewSettings>) => setS((prev) => { const next = { ...prev, ...patch }; save(chatId, next); return next; });
  const shown = reviews.find((r) => r.id === picked) ?? reviews[0];

  const run = async () => {
    setError('');
    try {
      await invoke('act', { action: { type: 'reviewStart', label: scopeLabel(s), task: reviewTask(s), models: s.reviewer ? [s.reviewer] : [] } });
      setSetupOpen(false);
      setPicked(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="review-tab">
      <div className="review-inner stack">
        <div className="row tight">
          <p className="small dim grow">A second opinion on <b>{chat.project}</b> from a separate reviewer. Findings stay here, for this chat.</p>
        </div>

        {reviews.length > 0 && (
          <button type="button" className="review-toggle row tight" onClick={() => setSetupOpen(!setupOpen)}>
            <Icon name={setupOpen ? 'chevronDown' : 'chevronRight'} size={12} /><span className="grow small">New review</span>
          </button>
        )}
        {(setupOpen || reviews.length === 0) && !view?.running && <Setup s={s} set={set} onRun={run} error={error} />}
        {view?.running && setupOpen && <div className="small dim">A review is running. Stop it or wait for it to start another.</div>}

        {shown && (
          <>
            {reviews.length > 1 && (
              <div className="row tight review-history">
                <select className="input select grow" value={shown.id} onChange={(e) => setPicked(Number(e.target.value))}>
                  {reviews.map((r) => <option key={r.id} value={r.id}>{new Date(r.id).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })} · {r.label} · {r.findings.length} found</option>)}
                </select>
                <button type="button" className="button ghost compact" disabled={view?.running} onClick={() => act('reviewRemove', {})}>Clear all</button>
              </div>
            )}
            <ReviewResult review={shown} cwd={chat.cwd} running={chatRunning} />
          </>
        )}
      </div>
    </div>
  );
}
