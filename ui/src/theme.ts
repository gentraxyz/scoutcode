// Color themes: a background and an element color, each picked from a short list. There are no free color pickers on
// purpose, so every combination stays readable. The choice is about this window only, so it lives in localStorage and
// is applied before the first render.
import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Vars = Record<string, string>;

export const BACKGROUNDS: { id: string; name: string; vars: Vars }[] = [
  { id: 'graphite', name: 'Graphite', vars: { bg: '#1f1f1f', sidebar: '#1a1a1a', surface: '#262626', 'surface-hi': '#2e2e2e', 'code-bg': '#1a1a1a', 'user-bg': '#2a2a2a', text: '#cccccc', strong: '#ededed', muted: '#a3a3a3', dim: '#7a7a7a', faint: '#4d4d4d' } },
  { id: 'midnight', name: 'Midnight', vars: { bg: '#171b24', sidebar: '#13161e', surface: '#1e2330', 'surface-hi': '#262c3b', 'code-bg': '#13161e', 'user-bg': '#232937', text: '#c9cfdb', strong: '#eceff5', muted: '#9ba3b4', dim: '#737c8f', faint: '#474e5e' } },
  { id: 'forest', name: 'Forest', vars: { bg: '#18201c', sidebar: '#141a17', surface: '#1f2924', 'surface-hi': '#27322c', 'code-bg': '#141a17', 'user-bg': '#232d28', text: '#c8d2cc', strong: '#ebf0ed', muted: '#9caaa2', dim: '#74827a', faint: '#48524c' } },
  { id: 'mocha', name: 'Mocha', vars: { bg: '#211c19', sidebar: '#1b1714', surface: '#2a2420', 'surface-hi': '#332c27', 'code-bg': '#1b1714', 'user-bg': '#2e2723', text: '#d4cbc4', strong: '#f0e9e3', muted: '#aa9f97', dim: '#82776f', faint: '#524943' } },
  { id: 'plum', name: 'Plum', vars: { bg: '#1e1a24', sidebar: '#18151e', surface: '#26212e', 'surface-hi': '#2f2938', 'code-bg': '#18151e', 'user-bg': '#2a2433', text: '#cfc8d8', strong: '#eee9f4', muted: '#a59db0', dim: '#7c7488', faint: '#4e4758' } },
  { id: 'black', name: 'Black', vars: { bg: '#0e0e0e', sidebar: '#090909', surface: '#171717', 'surface-hi': '#1f1f1f', 'code-bg': '#090909', 'user-bg': '#1b1b1b', text: '#c4c4c4', strong: '#ededed', muted: '#9a9a9a', dim: '#707070', faint: '#424242' } },
];

/** The color of primary buttons, the send button and checkboxes. */
export const ELEMENTS: { id: string; name: string; vars: Vars }[] = [
  { id: 'gray', name: 'Gray', vars: { primary: '#e8e8e8', 'primary-hi': '#ffffff', 'primary-ink': '#1a1a1a' } },
  { id: 'blue', name: 'Blue', vars: { primary: '#6ea8fe', 'primary-hi': '#8dbbff', 'primary-ink': '#0b1626' } },
  { id: 'teal', name: 'Teal', vars: { primary: '#4cc4c0', 'primary-hi': '#6fd3cf', 'primary-ink': '#04201f' } },
  { id: 'green', name: 'Green', vars: { primary: '#5cc98a', 'primary-hi': '#7ad6a0', 'primary-ink': '#07140c' } },
  { id: 'amber', name: 'Amber', vars: { primary: '#f5a524', 'primary-hi': '#ffb845', 'primary-ink': '#1a1203' } },
  { id: 'rose', name: 'Rose', vars: { primary: '#f07a8f', 'primary-hi': '#f697a8', 'primary-ink': '#2a0b12' } },
  { id: 'violet', name: 'Violet', vars: { primary: '#a68cf5', 'primary-hi': '#bba6fa', 'primary-ink': '#150d2b' } },
];

/**
 * What's behind the window. Solid is the background color; the rest let Windows draw the desktop through it, blurred,
 * while the theme's colors stay solid where text is (styles.css says which parts open up). `alpha` is null for Solid.
 */
export const MATERIALS: { id: string; name: string; note: string; alpha: Record<string, number> | null }[] = [
  { id: 'solid', name: 'Solid', note: 'The background color, nothing behind it.', alpha: null },
  { id: 'mica', name: 'Mica', note: 'The sidebar and title bar take on your wallpaper\'s color, like Windows 11 apps. Needs Windows 11.', alpha: {} },
  { id: 'acrylic', name: 'Acrylic', note: 'The sidebar and title bar are frosted glass over what\'s behind the window.', alpha: {} },
  { id: 'glass', name: 'Liquid Glass', note: 'The sidebar, message box, dock and menus float over your work as panes of glass.', alpha: {} },
];

export type Theme = { background: string; elements: string; material: string; logo: string };

const KEY = 'scout.theme';
const DEFAULT: Theme = { background: 'graphite', elements: 'gray', material: 'solid', logo: 'classic' };
/** Why the material couldn't be applied (an older Windows), shown in Settings. */
let materialError: string | null = null;
let appliedMaterial = '';
const subs = new Set<() => void>();

function load(): Theme {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Partial<Theme> | null;
    // Ids that are no longer in the lists fall back to the default.
    return {
      background: BACKGROUNDS.some((b) => b.id === saved?.background) ? saved!.background! : DEFAULT.background,
      elements: ELEMENTS.some((e) => e.id === saved?.elements) ? saved!.elements! : DEFAULT.elements,
      material: MATERIALS.some((m) => m.id === saved?.material) ? saved!.material! : DEFAULT.material,
      logo: typeof saved?.logo === 'string' ? saved.logo : DEFAULT.logo,
    };
  } catch {
    return DEFAULT;
  }
}

let theme = load();

/** "#1f1f1f" at `alpha` as rgb(31 31 31 / 62%). */
function translucent(hex: string, alpha: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255} / ${Math.round(alpha * 100)}%)`;
}

function apply() {
  const root = document.documentElement;
  const material = MATERIALS.find((m) => m.id === theme.material)!;
  const vars: Record<string, string> = { ...BACKGROUNDS.find((b) => b.id === theme.background)!.vars, ...ELEMENTS.find((e) => e.id === theme.elements)!.vars };
  for (const [name, alpha] of Object.entries(material.alpha ?? {})) vars[name] = translucent(vars[name], alpha);
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(`--${name}`, value);
  if (material.alpha) root.dataset.material = material.id;
  else delete root.dataset.material;
  if (appliedMaterial !== material.id) {
    appliedMaterial = material.id;
    invoke('set_material', { material: material.id })
      .then(() => { materialError = null; })
      .catch((e) => {
        // Without the effect, translucent colors would show black: back to solid, and say why.
        materialError = `${material.name} isn't available on this version of Windows. ${String(e)}`.trim();
        theme = { ...theme, material: 'solid' };
        appliedMaterial = '';
        apply();
      })
      .finally(() => subs.forEach((f) => f()));
  }
}
apply();

export function setTheme(patch: Partial<Theme>) {
  theme = { ...theme, ...patch };
  try { localStorage.setItem(KEY, JSON.stringify(theme)); } catch { /* still applies for this run */ }
  apply();
  subs.forEach((f) => f());
}

export const getTheme = () => theme;
export const getMaterialError = () => materialError;

export function subscribeTheme(f: () => void) {
  subs.add(f);
  return () => { subs.delete(f); };
}

export function useTheme(): Theme {
  return useSyncExternalStore(subscribeTheme, () => theme);
}

/** A theme color as it is right now, for what can't read CSS variables (the terminal). */
export function themeColor(name: string) {
  return getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();
}
