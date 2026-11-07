const $ = (id) => document.getElementById(id);
const CHECK = '<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 10.5l3.2 3L15 6.5"/></svg>';
const CROSS = '<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M6 6l8 8M14 6l-8 8"/></svg>';

// One of: 'ok', 'bad', 'wait' (a spinner) or 'idle'.
function mark(id, kind) {
  const el = $(id);
  if (el.dataset.kind === kind) return;
  el.dataset.kind = kind;
  el.className = `state ${kind}`;
  el.innerHTML = kind === 'ok' ? CHECK : kind === 'bad' ? CROSS : kind === 'wait' ? '<div class="spin"></div>' : '';
}

const PHASES = {
  none: ['', 'DeepSeek not detected', 'No tab'],
  connecting: ['wait', 'Connecting…', 'Connecting'],
  error: ['bad', 'Something went wrong', 'Error'],
  signin: ['bad', 'Sign in to DeepSeek', 'Sign in'],
  ready: ['ok', 'Ready for Scout', 'Connected'],
  working: ['ok', 'Working on a request', 'Connected'],
};

const STAGES = {
  opening: 'Opening a new chat…',
  sending: 'Sending Scout’s message…',
  waiting: 'Waiting for DeepSeek…',
  streaming: 'DeepSeek is replying…',
};

let tabId = null;

function show(s) {
  s = s || { phase: 'none', tabs: 0 };
  const phase = PHASES[s.phase] ? s.phase : 'none';
  const [kind, summary, pill] = PHASES[phase];
  $('summary').textContent = summary;
  $('pill').className = `pill ${kind}`;
  $('pill').textContent = pill;

  // Scout: only reached once a DeepSeek tab is open, since the tab is what starts the worker.
  if (!s.tabs) { mark('scout', 'idle'); $('scout-text').textContent = 'Waiting for a DeepSeek tab'; }
  else if (s.connected) { mark('scout', 'ok'); $('scout-text').textContent = 'Connected'; }
  else if (s.error) { mark('scout', 'bad'); $('scout-text').textContent = 'Not reachable'; }
  else { mark('scout', 'wait'); $('scout-text').textContent = 'Looking for Scout…'; }

  if (!s.tabs) { mark('tab', 'idle'); $('tab-text').textContent = 'No chat.deepseek.com tab open'; }
  else if (!s.reported) { mark('tab', 'wait'); $('tab-text').textContent = 'Waiting for the page…'; }
  else if (s.signedIn) { mark('tab', 'ok'); $('tab-text').textContent = s.tabs > 1 ? `Signed in · ${s.tabs} tabs open` : 'Signed in'; }
  else { mark('tab', 'bad'); $('tab-text').textContent = 'Not signed in'; }

  $('work').classList.toggle('show', phase === 'working');
  $('work-text').textContent = STAGES[s.stage] || 'Running a request from Scout';

  $('error').classList.toggle('show', !!s.error);
  $('error-text').textContent = s.error ? `${s.error} Retrying…` : '';

  tabId = s.tabId || null;
  const btn = $('open');
  const label = !s.tabs ? 'Open chat.deepseek.com' : phase === 'signin' ? 'Go to the DeepSeek tab' : '';
  btn.textContent = label;
  btn.classList.toggle('show', !!label);
}

$('open').addEventListener('click', async () => {
  try {
    if (tabId) {
      const t = await chrome.tabs.update(tabId, { active: true });
      if (t && t.windowId != null) await chrome.windows.update(t.windowId, { focused: true });
    } else {
      await chrome.tabs.create({ url: 'https://chat.deepseek.com/' });
    }
  } catch {
    await chrome.tabs.create({ url: 'https://chat.deepseek.com/' });
  }
  window.close();
});

chrome.storage.session.get('state').then(({ state }) => show(state));
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'session' && changes.state) show(changes.state.newValue);
});
