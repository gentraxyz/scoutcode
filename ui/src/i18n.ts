// The interface language and comprehensive i18n system.
// Supports 30 full translations with instant reactive switching.
import { useSyncExternalStore } from 'react';
import { en, type TranslationKey, type Translations } from './i18n/en';
import { es } from './i18n/es';
import { fr } from './i18n/fr';
import { de } from './i18n/de';
import { it } from './i18n/it';
import { pt } from './i18n/pt';
import { ru } from './i18n/ru';
import { uk } from './i18n/uk';
import { pl } from './i18n/pl';
import { tr } from './i18n/tr';
import { nl } from './i18n/nl';
import { zh } from './i18n/zh';
import { ja } from './i18n/ja';
import { ko } from './i18n/ko';
import { ar } from './i18n/ar';
import { hi } from './i18n/hi';
import { vi } from './i18n/vi';
import { sv } from './i18n/sv';
import { da } from './i18n/da';
import { no } from './i18n/no';
import { fi } from './i18n/fi';
import { cs } from './i18n/cs';
import { el } from './i18n/el';
import { hu } from './i18n/hu';
import { ro } from './i18n/ro';
import { id } from './i18n/id';
import { th } from './i18n/th';
import { he } from './i18n/he';
import { bn } from './i18n/bn';
import { fil } from './i18n/fil';

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

const DICTIONARIES: Record<string, Translations> = {
  English: en,
  Spanish: es,
  French: fr,
  German: de,
  Italian: it,
  Portuguese: pt,
  Russian: ru,
  Ukrainian: uk,
  Polish: pl,
  Turkish: tr,
  Dutch: nl,
  Chinese: zh,
  Japanese: ja,
  Korean: ko,
  Arabic: ar,
  Hindi: hi,
  Vietnamese: vi,
  Swedish: sv,
  Danish: da,
  Norwegian: no,
  Finnish: fi,
  Czech: cs,
  Greek: el,
  Hungarian: hu,
  Romanian: ro,
  Indonesian: id,
  Thai: th,
  Hebrew: he,
  Bengali: bn,
  Filipino: fil,
};

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
  return {
    lang: currentLang,
    t: (key: TranslationKey, params?: Record<string, string | number>) =>
      translate(currentLang, key, params),
  };
}
