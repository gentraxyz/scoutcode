// The window's own tooltips, in place of the system's: any element with a title shows it in a styled bubble after a short
// pause. While hovered, the title is moved aside (to data-tooltip) so the system's plain tooltip doesn't show as well,
// and it goes back on leave.

const DELAY = 350;
const SELECTOR = '[title],[data-tooltip]';
let tip: HTMLDivElement | null = null;
/** The element under the pointer, and one just clicked, which stays quiet until the pointer moves to another. */
let hovered: HTMLElement | null = null;
let dismissed: HTMLElement | null = null;
let timer = 0;

/** Moves the element's title aside while it's hovered, and returns the text to show. */
function grab(el: HTMLElement): string {
  const title = el.getAttribute('title');
  if (title) { el.dataset.tooltip = title; el.removeAttribute('title'); }
  return el.dataset.tooltip ?? '';
}

/** Puts the title back, unless the element was given a new one meanwhile. */
function release(el: HTMLElement) {
  if (el.dataset.tooltip === undefined) return;
  if (!el.hasAttribute('title')) el.setAttribute('title', el.dataset.tooltip);
  delete el.dataset.tooltip;
}

function hide() {
  clearTimeout(timer);
  tip?.classList.remove('show');
  if (hovered) release(hovered);
  hovered = null;
}

function show(el: HTMLElement) {
  const text = grab(el);
  if (!tip || !text || !el.isConnected) return;
  tip.textContent = text;
  const r = el.getBoundingClientRect();
  const w = tip.offsetWidth;
  const h = tip.offsetHeight;
  const below = r.bottom + 8 + h <= window.innerHeight - 8;
  const top = below ? r.bottom + 8 : Math.max(8, r.top - 8 - h);
  const left = Math.max(8, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - w - 8));
  tip.style.left = `${left}px`;
  tip.style.top = `${top}px`;
  tip.classList.add('show');
}

export function installTooltips() {
  if (tip) return;
  tip = document.createElement('div');
  tip.className = 'app-tip';
  tip.setAttribute('role', 'tooltip');
  document.body.append(tip);
  document.addEventListener('mouseover', (e) => {
    const el = (e.target as Element).closest<HTMLElement>(SELECTOR);
    if (el === hovered || (el && el === dismissed)) return;
    dismissed = null;
    // Already showing one: the next one follows at once instead of after the pause.
    const warm = tip!.classList.contains('show');
    hide();
    if (!el) return;
    hovered = el;
    grab(el);
    timer = window.setTimeout(() => show(el), warm ? 0 : DELAY);
  });
  document.addEventListener('mouseout', (e) => { if (!e.relatedTarget) { hide(); dismissed = null; } });
  document.addEventListener('pointerdown', () => { dismissed = hovered; hide(); }, true);
  for (const type of ['keydown', 'wheel', 'scroll'] as const) document.addEventListener(type, hide, { capture: true, passive: true });
  window.addEventListener('blur', hide);
}
