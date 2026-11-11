// TEMPORARY: fakes the Tauri core so the UI renders in a plain browser for landing-page screenshots. Delete after use.
const q = new URLSearchParams(location.search);
// The same random numbers every load, so the greeting and backdrop match from frame to frame.
let seed = 7;
Math.random = () => { seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
const scene = q.get('s') || 'hero';

const model = (id, label, extra = {}) => ({
  id, label, description: null, context: 200000, tools: true, vision: true, cost: null, providers: ['scoutcode'], routesTo: [], available: true,
  openrouterId: null, nvidiaId: null, opencodeId: null, kiloId: null, kiloFree: false, kiloTier: null, copilotId: null, devinId: null, kiroId: null,
  codexId: null, claudeId: null, grokId: null, hermesId: null, additional: null, hosted: true, free: true, tier: 3, openrouterTier: null,
  openrouterFree: false, intelligence: 3, ...extra,
});
const models = [
  model('qwen/qwen3.8-coder', 'Qwen 3.8 Coder'),
  model('deepseek/deepseek-v4', 'DeepSeek V4'),
  model('z-ai/glm-5', 'GLM 5'),
  model('moonshotai/kimi-k3', 'Kimi K3'),
  model('minimax/minimax-m3', 'MiniMax M3'),
];

const diffLines = (rows) => ({ rows, more: 0 });
let n = 1;
const user = (text) => ({ id: n++, kind: 'user', text, files: [] });
const say = (text) => ({ id: n++, kind: 'assistant', text, reasoning: '', model: 'Qwen 3.8 Coder', provider: 'scoutcode' });
const tool = (name, label, summary, display = { type: 'text', text: '' }) => ({ id: n++, kind: 'tool', name, label, progress: '', result: { isError: false, summary, display } });

const sessionDiff = {
  type: 'diff', path: 'src/auth/session.ts', created: false, add: 2, del: 1,
  lines: diffLines([
    { kind: 'same', text: 'export async function login(user: User) {', no: 42 },
    { kind: 'del', text: '  saveSession(user);', no: 43 },
    { kind: 'add', text: '  await saveSession(user);', no: 43 },
    { kind: 'add', text: '  await flushCookies();', no: 44 },
    { kind: 'same', text: '  return redirect("/");', no: 45 },
    { kind: 'same', text: '}', no: 46 },
  ]),
};

const patch = { type: 'diff', path: 'tests/login.test.ts', created: false, add: 3, del: 1, lines: diffLines([
  { kind: 'same', text: "it('keeps the session after login', async () => {", no: 12 },
  { kind: 'del', text: '  const res = login(alice);', no: 13 },
  { kind: 'add', text: '  const res = await login(alice);', no: 13 },
  { kind: 'add', text: '  await waitForCookie(res, "sid");', no: 14 },
  { kind: 'add', text: '  expect(res.cookies.sid).toBeDefined();', no: 15 },
  { kind: 'same', text: '});', no: 16 },
]) };
const items = scene === 'tools' ? [
  tool('edit_file', 'src/auth/session.ts', '+2 −1', sessionDiff),
  { id: n++, kind: 'assistant', text: 'Now the test waits for the cookie instead of racing it.', reasoning: '', model: 'GPT-5.5 Codex', provider: 'codex' },
  tool('apply_patch', 'tests/login.test.ts', '+3 −1', patch),
] : [
  user('The login test flakes on CI. Find out why and fix it.'),
  tool('view', 'src/auth/session.ts', '88 lines'),
  tool('grep', 'saveSession', '6 matches in 4 files'),
  tool('view', 'tests/login.test.ts', '54 lines'),
  say('`login()` redirects before the session is written, so a fast test runner can reach the next page with no cookie. It needs to wait for the write, and flush cookies before redirecting.'),
  tool('edit_file', 'src/auth/session.ts', '+2 −1', sessionDiff),
];
if (scene === 'done') {
  items.push(tool('bash', 'npm test -- login --repeat 20', '20 passed'));
  items.push(say('Fixed. `login()` now awaits `saveSession` and flushes cookies before redirecting. The login test passed **20 times in a row**, where it failed about 1 in 6 before.'));
}

const permission = scene === 'hero' || scene === 'perm'
  ? { tool: 'bash', label: 'npm test -- login --repeat 20', preview: null, prefix: 'npm test', reason: null }
  : null;

const view = {
  cfg: {
    loginNeeded: false, team: null, hasKey: false, guest: false, browserSignIn: true, username: 'scout', baseUrl: 'https://scoutcode.dev', openrouterKeys: [],
    nvidiaKey: null, kiloKey: null, kilo: true, copilot: false, copilotUser: null, devin: false, kiro: false, codex: false, claude: false, opencode: true, opencodeSignedIn: false,
    grokSignedIn: false, grokKey: null, hermesSignedIn: false, hermesKey: null, additional: [], notifyFlash: true, notifyToast: true, discordPresence: false, discordProject: false,
    autoUpdate: true, usageAnalytics: false, autoCompactTokens: 90000,
    fallbacks: { chosen: false, all: [], models: [] }, agentRouters: [], experimentalImages: false, imageModel: null,
    explore: { on: true, models: [], tokens: 184000, showFrom: 0 }, games: { on: false, place: 'float', openOnRun: false, pos: null }, music: { on: false, pos: null, volume: 0.5 },
    local: { backend: null, gpuLayers: null, context: 16384, tools: 'standard', singleCall: false, shortPrompt: false, reasoningBudget: 0, models: [] },
  },
  local: {
    build: '', backend: null, backends: [], installing: null, installError: null, hardware: { ramGb: 32, gpuGb: 8, gpuName: null, shared: false }, recommended: null, freeDisk: null,
    curated: [], downloads: {}, downloadErrors: {}, setup: null, setupError: null, server: { state: 'stopped', model: null, error: null, offloaded: null, cpuFaster: false },
  },
  review: { chat: 'c1', running: false, reviews: [] },
  dreaming: { on: false, running: false, model: null, last: 0, log: [], message: null },
  mode: q.get('mode') || 'default',
  model: { id: 'qwen/qwen3.8-coder', provider: 'scoutcode', key: 'qwen/qwen3.8-coder', short: 'Qwen 3.8 Coder' },
  effort: 'medium',
  starred: [],
  modelsLoading: 0,
  additionalErrors: [], kiroModelsError: null, codexModelsError: null, claudeModelsError: null, opencodeModelsError: null, grokModelsError: null, hermesModelsError: null,
  run: scene === 'steer'
    ? { running: true, stopping: false, stopped: false, status: 'Running npm test', anyRunning: true }
    : { running: false, stopping: false, stopped: false, status: '', anyRunning: false },
  todos: [],
  totals: { totals: { promptTokens: 41200, completionTokens: 1830, cost: 0, requests: 7, cachedTokens: 30100 }, context: 41200 },
  permission,
  plan: null,
  questions: null,
  queue: scene === 'steer' ? [{ text: 'Use the existing retry helper instead.', files: 0 }] : [],
  steering: [],
  sessions: [
    { id: 'c1', cwd: 'C:\\code\\api-server', title: 'Fix the flaky login test', ago: 'now', running: scene === 'steer', waiting: !!permission, unread: false },
    { id: 'c2', cwd: 'C:\\code\\api-server', title: 'Add rate limits to uploads', ago: '2h', running: false, waiting: false, unread: false, done: true },
    { id: 'c3', cwd: 'C:\\code\\api-server', title: 'Why is the build slow?', ago: '1d', running: false, waiting: false, unread: false },
    { id: 'c4', cwd: 'C:\\code\\website', title: 'Clean up the footer', ago: '3d', running: false, waiting: false, unread: false },
    { id: 'c5', cwd: 'C:\\code\\website', title: 'Dark mode for the docs', ago: '5d', running: false, waiting: false, unread: false },
  ],
  projects: [{ name: 'api-server', path: 'C:\\code\\api-server' }, { name: 'website', path: 'C:\\code\\website' }],
  chat: { id: 'c1', cwd: 'C:\\code\\api-server', project: 'api-server', title: 'Fix the flaky login test', epoch: 1 },
  filesEpoch: 1,
  focusEpoch: 0,
  login: {
    busy: false, url: null, error: null, kiloCheck: null, copilot: null, copilotError: null, devin: null, devinError: null, kiro: null, kiroError: null, codex: null, codexError: null,
    claude: null, claudeError: null, opencode: null, opencodeError: null, grok: null, grokError: null, hermes: null, hermesError: null,
  },
  skills: [],
  link: { on: false, online: false, cwd: '', status: '', mode: null, wanted: false, since: null, running: null, log: [], edits: [], chatbots: [], permission: null },
  scoutMote: { on: false, running: false, status: '', online: false, name: '' },
  usage: { info: null, busy: false, message: null },
  quotas: [],
  voice: { state: 'idle' },
  updater: { enabled: true, version: '0.1.30', commit: '', status: { state: 'upToDate' } },
  whatsNew: [],
  mcp: { error: null, servers: [] },
};

const statics = {
  modes: [
    { id: 'default', label: 'Ask first', about: 'Asks before edits and commands' },
    { id: 'acceptEdits', label: 'Accept edits', about: 'Edits freely, asks before commands' },
    { id: 'auto', label: 'Auto', about: 'Holds only what the safety check flags' },
    { id: 'plan', label: 'Plan', about: 'Read-only until you approve a plan' },
    { id: 'yolo', label: 'Yolo', about: 'Never asks' },
  ],
  efforts: ['low', 'medium', 'high'],
  additionalProviders: [], kiloWarning: '',
  plans: { copilot: '', devin: '', kiro: '', codex: '', claude: '', opencode: '', grok: '', hermes: '' },
  languages: [['en', 'English']], exe: 'Scout.exe',
};

const changes = {
  repo: 'C:\\code\\api-server',
  files: [
    { path: 'src/auth/session.ts', from: null, status: 'M', counts: [2, 1] },
    { path: 'tests/login.test.ts', from: null, status: 'M', counts: [18, 0] },
    { path: 'src/auth/cookies.ts', from: null, status: 'A', counts: [24, 0] },
    { path: 'package.json', from: null, status: 'M', counts: [1, 1] },
  ],
};
const tree = {
  'C:\\code\\api-server': [['src', 1], ['tests', 1], ['package.json', 0], ['README.md', 0], ['tsconfig.json', 0]],
  'C:\\code\\api-server\\src': [['auth', 1], ['routes', 1], ['server.ts', 0]],
};

let cb = 0;
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
  transformCallback: () => ++cb,
  unregisterCallback: () => {},
  convertFileSrc: (p) => p,
  invoke: async (cmd, args) => {
    switch (cmd) {
      case 'snapshot': return { seq: 1, view, models, items, ...statics };
      case 'changes_load': return changes;
      case 'changes_diff': return sessionDiff.lines;
      case 'dir_read': return (tree[args.dir] || []).map(([name, dir]) => ({ name, path: `${args.dir}\\${name}`, dir: !!dir }));
      case 'search_chats': case 'music_list': case 'mention_search': case 'skill_search': case 'image_models': return [];
      case 'memory_load': return [];
      default: return null;
    }
  },
};
for (const label of (q.get('click') || '').split(',').filter(Boolean)) {
  const t0 = Date.now();
  const tryClick = () => {
    const el = [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === label || b.textContent.trim() === label);
    if (el) el.click(); else if (Date.now() - t0 < 4000) setTimeout(tryClick, 100);
  };
  setTimeout(tryClick, 300);
}
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };

// ---- Video steps: ?step=N walks through one session, from the home screen to the fix. ----
if (q.has('step')) {
  const step = Number(q.get('step'));
  const run = (status) => ({ running: true, stopping: false, stopped: false, status, anyRunning: true });
  const live = (name, label) => ({ id: n++, kind: 'tool', name, label, progress: '', result: null });
  const done = (name, label, summary, display) => tool(name, label, summary, display);
  const prompt = 'The login test flakes on CI. Find out why and fix it.';
  const reply = '`login()` redirects before the session is written, so a fast test runner can reach the next page with no cookie. It needs to wait for the write, and flush cookies before redirecting.';
  const partial = Number(q.get('chars') || 0);
  const fixed = 'Fixed. `login()` now awaits `saveSession` and flushes cookies before redirecting. The login test passed **20 times in a row**, where it failed about 1 in 6 before.';
  const seq = [
    /* 0 home */ [],
    /* 1 */ [user(prompt)],
    /* 2 */ [user(prompt), live('view', 'src/auth/session.ts')],
    /* 3 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), live('grep', 'saveSession')],
    /* 4 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), live('view', 'tests/login.test.ts')],
    /* 5 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), done('view', 'tests/login.test.ts', '54 lines'), say(reply.slice(0, partial || reply.length))],
    /* 6 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), done('view', 'tests/login.test.ts', '54 lines'), say(reply), live('edit_file', 'src/auth/session.ts')],
    /* 7 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), done('view', 'tests/login.test.ts', '54 lines'), say(reply), done('edit_file', 'src/auth/session.ts', '+2 −1', sessionDiff)],
    /* 8 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), done('view', 'tests/login.test.ts', '54 lines'), say(reply), done('edit_file', 'src/auth/session.ts', '+2 −1', sessionDiff), live('bash', 'npm test -- login --repeat 20')],
    /* 9 */ [user(prompt), done('view', 'src/auth/session.ts', '88 lines'), done('grep', 'saveSession', '6 matches in 4 files'), done('view', 'tests/login.test.ts', '54 lines'), say(reply), done('edit_file', 'src/auth/session.ts', '+2 −1', sessionDiff), done('bash', 'npm test -- login --repeat 20', '20 passed'), say(fixed.slice(0, partial || fixed.length))],
  ];
  items.length = 0;
  items.push(...seq[step]);
  const status = ['', 'Thinking', 'Reading src/auth/session.ts', 'Searching for saveSession', 'Reading tests/login.test.ts', 'Writing', 'Editing src/auth/session.ts', 'Waiting for you', 'Running npm test', step === 9 && partial ? 'Writing' : ''][step];
  view.run = step === 0 || (step === 9 && !partial) ? { running: false, stopping: false, stopped: false, status: '', anyRunning: false } : run(status);
  view.permission = step === 7 ? { tool: 'bash', label: 'npm test -- login --repeat 20', preview: null, prefix: 'npm test', reason: null } : null;
  view.totals.totals.requests = step;
  if (step === 0) {
    view.chat = { id: null, cwd: 'C:\code\api-server', project: 'api-server', title: '', epoch: 1 };
    view.sessions = view.sessions.slice(1);
  } else {
    view.sessions[0].running = view.run.running && step !== 7;
    view.sessions[0].waiting = step === 7;
    if (step === 9 && !partial) view.sessions[0].unread = false;
  }
}

// ?type=… puts text in the chat box, as if typed.
window.addEventListener('load', () => {
  const text = q.get('type');
  const t0 = Date.now();
  const fill = () => {
    const ta = document.querySelector('textarea');
    if (!ta) return Date.now() - t0 < 4000 && setTimeout(fill, 100);
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(ta, text);
    ta.dispatchEvent(new Event('input', { bubbles: true }));
  };
  if (text !== null) setTimeout(fill, 200);
  // ?cur=A or ?cur=A>B@0.5: a mouse pointer on (or between) buttons named A and B, or "start" for a resting spot.
  const cur = q.get('cur');
  if (!cur) return;
  const place = () => {
    const [path, f = '1'] = cur.split('@');
    const [a, b] = path.split('>');
    const at = (label) => {
      if (label === 'start') return [innerWidth * 0.62, innerHeight * 0.55];
      if (label === 'end') return [innerWidth * 0.8, innerHeight * 0.62];
      const el = [...document.querySelectorAll('button')].find((x) => x.getAttribute('aria-label') === label || x.textContent.trim() === label);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return [r.left + r.width / 2, r.top + r.height / 2];
    };
    const p = at(a), p2 = b ? at(b) : p;
    if (!p || !p2) return;
    const k = Number(f), e = k < 0.5 ? 2 * k * k : 1 - (-2 * k + 2) ** 2 / 2;
    const x = p[0] + (p2[0] - p[0]) * e, y = p[1] + (p2[1] - p[1]) * e;
    let d = document.getElementById('fake-cursor');
    if (d) d.remove();
    d = document.createElement('div');
    d.id = 'fake-cursor';
    d.innerHTML = '<svg width="22" height="22" viewBox="0 0 24 24"><path d="M5 3l14 8-6 2-3 6z" fill="#fff" stroke="#000" stroke-width="1.4" stroke-linejoin="round"/></svg>';
    d.style.cssText = `position:fixed;left:${x - 5}px;top:${y - 3}px;z-index:99999;pointer-events:none;filter:drop-shadow(0 1px 2px rgba(0,0,0,.5))`;
    if (q.has('press')) d.firstChild.style.transform = 'scale(.85)';
    document.body.appendChild(d);
  };
  setInterval(place, 100);
});
