// The interface language and comprehensive i18n system.
// Supports 30 full translations with instant reactive switching.
import { useSyncExternalStore } from 'react';
import { en, type TranslationKey, type Translations } from './i18n/en';

export type { TranslationKey, Translations };

const KEY = 'scout.uiLanguage';
const DEFAULT = 'English';

export const SUPPORTED_LANGUAGES: [string, string][] = [
  ['English', 'English'],
  ['Spanish', 'Español'],
  ['French', 'Français'],
  ['German', 'Deutsch'],
  ['Italian', 'Italiano'],
  ['Portuguese', 'Português'],
  ['Russian', 'Русский'],
  ['Ukrainian', 'Українська'],
  ['Polish', 'Polski'],
  ['Turkish', 'Türkçe'],
  ['Dutch', 'Nederlands'],
  ['Chinese', '中文'],
  ['Japanese', '日本語'],
  ['Korean', '한국어'],
  ['Arabic', 'العربية'],
  ['Hindi', 'हिन्दी'],
  ['Vietnamese', 'Tiếng Việt'],
  ['Swedish', 'Svenska'],
  ['Danish', 'Dansk'],
  ['Norwegian', 'Norsk'],
  ['Finnish', 'Suomi'],
  ['Czech', 'Čeština'],
  ['Greek', 'Ελληνικά'],
  ['Hungarian', 'Magyar'],
  ['Romanian', 'Română'],
  ['Indonesian', 'Bahasa Indonesia'],
  ['Thai', 'ไทย'],
  ['Hebrew', 'עברית'],
  ['Bengali', 'বাংলা'],
  ['Filipino', 'Filipino'],
];

/** Each translation is its own chunk, read when it is picked: only one is ever in use. */
const LOADERS: Record<string, () => Promise<Translations>> = {
  Spanish: () => import('./i18n/es').then((d) => d.es),
  French: () => import('./i18n/fr').then((d) => d.fr),
  German: () => import('./i18n/de').then((d) => d.de),
  Italian: () => import('./i18n/it').then((d) => d.it),
  Portuguese: () => import('./i18n/pt').then((d) => d.pt),
  Russian: () => import('./i18n/ru').then((d) => d.ru),
  Ukrainian: () => import('./i18n/uk').then((d) => d.uk),
  Polish: () => import('./i18n/pl').then((d) => d.pl),
  Turkish: () => import('./i18n/tr').then((d) => d.tr),
  Dutch: () => import('./i18n/nl').then((d) => d.nl),
  Chinese: () => import('./i18n/zh').then((d) => d.zh),
  Japanese: () => import('./i18n/ja').then((d) => d.ja),
  Korean: () => import('./i18n/ko').then((d) => d.ko),
  Arabic: () => import('./i18n/ar').then((d) => d.ar),
  Hindi: () => import('./i18n/hi').then((d) => d.hi),
  Vietnamese: () => import('./i18n/vi').then((d) => d.vi),
  Swedish: () => import('./i18n/sv').then((d) => d.sv),
  Danish: () => import('./i18n/da').then((d) => d.da),
  Norwegian: () => import('./i18n/no').then((d) => d.no),
  Finnish: () => import('./i18n/fi').then((d) => d.fi),
  Czech: () => import('./i18n/cs').then((d) => d.cs),
  Greek: () => import('./i18n/el').then((d) => d.el),
  Hungarian: () => import('./i18n/hu').then((d) => d.hu),
  Romanian: () => import('./i18n/ro').then((d) => d.ro),
  Indonesian: () => import('./i18n/id').then((d) => d.id),
  Thai: () => import('./i18n/th').then((d) => d.th),
  Hebrew: () => import('./i18n/he').then((d) => d.he),
  Bengali: () => import('./i18n/bn').then((d) => d.bn),
  Filipino: () => import('./i18n/fil').then((d) => d.fil),
};

const DICTIONARIES: Record<string, Translations> = { English: en };
/** Bumped when a translation arrives, so the window draws itself again in it. */
let loaded = 0;
const pending = new Set<string>();

function ensure(id: string) {
  if (DICTIONARIES[id] || !LOADERS[id] || pending.has(id)) return;
  pending.add(id);
  LOADERS[id]().then((d) => {
    pending.delete(id);
    DICTIONARIES[id] = d;
    loaded++;
    subs.forEach((f) => f());
  }, (e) => { pending.delete(id); console.error('translation', id, e); });
}

/** Languages the interface is fully translated into. All 30 supported languages now have full translations. */
export const FULL = new Set(SUPPORTED_LANGUAGES.map(([id]) => id));

export const BCP47: Record<string, string> = {
  English: 'en',
  Spanish: 'es',
  French: 'fr',
  German: 'de',
  Italian: 'it',
  Portuguese: 'pt',
  Russian: 'ru',
  Ukrainian: 'uk',
  Polish: 'pl',
  Turkish: 'tr',
  Dutch: 'nl',
  Chinese: 'zh',
  Japanese: 'ja',
  Korean: 'ko',
  Arabic: 'ar',
  Hindi: 'hi',
  Vietnamese: 'vi',
  Swedish: 'sv',
  Danish: 'da',
  Norwegian: 'no',
  Finnish: 'fi',
  Czech: 'cs',
  Greek: 'el',
  Hungarian: 'hu',
  Romanian: 'ro',
  Indonesian: 'id',
  Thai: 'th',
  Hebrew: 'he',
  Bengali: 'bn',
  Filipino: 'fil',
};

const subs = new Set<() => void>();

function load(): string {
  try {
    return localStorage.getItem(KEY) || DEFAULT;
  } catch {
    return DEFAULT;
  }
}

let language = load();
ensure(language);

function apply() {
  if (typeof document === 'undefined') return;
  const code = BCP47[language] ?? 'en';
  document.documentElement.lang = code;
  document.documentElement.dir = (language === 'Arabic' || language === 'Hebrew') ? 'rtl' : 'ltr';
}
apply();

export const isFull = (id: string) => FULL.has(id);
export const getUiLanguage = () => language;

export function setUiLanguage(id: string) {
  language = id;
  ensure(id);
  try { localStorage.setItem(KEY, id); } catch { /* still applies for this run */ }
  apply();
  subs.forEach((f) => f());
}

function subscribe(f: () => void) {
  subs.add(f);
  return () => { subs.delete(f); };
}

export function useUiLanguage(): string {
  return useSyncExternalStore(subscribe, () => language);
}

/** The AI reply language that goes with an interface language: matches the chosen language so the model is explicitly told which language to speak in. */
export const replyLanguageFor = (id: string) => id;

/** Translates a key into the given language with optional template parameter replacements. */
export function translate(
  lang: string,
  key: TranslationKey,
  params?: Record<string, string | number>,
): string {
  const dict = DICTIONARIES[lang] || en;
  let text = dict[key] || en[key] || (key as string);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      text = text.replace(new RegExp(`\\{${k}\\}`, 'g'), String(v));
    }
  }
  return text;
}

/** Translates a key into the active UI language. */
export function t(key: TranslationKey, params?: Record<string, string | number>): string {
  return translate(language, key, params);
}

/** React hook that subscribes to language changes and returns a reactive `t` function. */
export function useTranslation() {
  const currentLang = useUiLanguage();
  // Draws again once the picked translation has loaded.
  useSyncExternalStore(subscribe, () => loaded);
  return {
    lang: currentLang,
    t: (key: TranslationKey, params?: Record<string, string | number>) =>
      translate(currentLang, key, params),
  };
}
