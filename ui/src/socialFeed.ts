// The Social page's feed: public posts from Bluesky, Dev.to and Mastodon about the user's interests. Every source is
// read without an account (their public, no-login APIs), straight from the window. The hosts are allowed in
// tauri.conf.json's CSP (connect-src for the APIs, img-src for avatars and covers).
import { useSyncExternalStore } from 'react';

export type Source = 'bluesky' | 'devto' | 'mastodon';
export const SOURCES: [Source, string][] = [['bluesky', 'Bluesky'], ['devto', 'Dev.to'], ['mastodon', 'Mastodon']];

export type Post = {
  id: string;
  source: Source;
  url: string;
  author: { name: string; handle: string; avatar?: string };
  /** Dev.to articles have a title; posts don't. */
  title?: string;
  text: string;
  image?: string;
  tags: string[];
  /** Which of the user's interests found it. */
  interest: string;
  at: number;
  likes: number;
  reposts: number;
  replies: number;
};

// ---- Settings, kept on this PC ----

type Prefs = { interests: string[]; sources: Source[]; sort: 'top' | 'latest' };
const KEY = 'scout.social';
const DEFAULTS: Prefs = { interests: ['rust', 'typescript', 'ai', 'webdev'], sources: ['bluesky', 'devto', 'mastodon'], sort: 'top' };

function load(): Prefs {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (!v || typeof v !== 'object') return DEFAULTS;
    return {
      interests: Array.isArray(v.interests) ? v.interests.filter((s: unknown) => typeof s === 'string').slice(0, 20) : DEFAULTS.interests,
      sources: Array.isArray(v.sources) ? v.sources.filter((s: unknown) => SOURCES.some(([id]) => id === s)) : DEFAULTS.sources,
      sort: v.sort === 'latest' ? 'latest' : 'top',
    };
  } catch {
    return DEFAULTS;
  }
}

let prefs = load();
const subs = new Set<() => void>();
export function setPrefs(next: Partial<Prefs>) {
  prefs = { ...prefs, ...next };
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Kept for this run only.
  }
  subs.forEach((f) => f());
}
export function useSocialPrefs(): Prefs {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f); }, () => prefs);
}

/** An interest as a hashtag/tag: Dev.to and Mastodon tags are letters and digits only. */
export const asTag = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, '');
/** What the user typed, tidied: "  Machine Learning " → "machine learning". */
export const cleanInterest = (s: string) => s.trim().replace(/^#/, '').replace(/\s+/g, ' ').toLowerCase().slice(0, 40);

// ---- Fetching ----

const DAY = 86_400_000;

const wait = (ms: number, signal: AbortSignal) => new Promise<void>((done) => {
  const t = setTimeout(done, ms);
  signal.addEventListener('abort', () => { clearTimeout(t); done(); }, { once: true });
});

/** A GET that waits and tries again when it's told to slow down (Dev.to answers 429 to a burst of requests). */
async function getJson<T>(url: string, signal: AbortSignal): Promise<T> {
  for (let tries = 0; ; tries++) {
    const r = await fetch(url, { signal, headers: { Accept: 'application/json' } });
    if (r.status === 429 && tries < 2) { await wait(1200 * (tries + 1), signal); continue; }
    if (!r.ok) throw new Error(`${r.status} ${r.statusText}`.trim());
    return r.json() as Promise<T>;
  }
}

/** Dev.to allows about two requests at once, so its interests are asked one after another. */
let devtoQueue: Promise<unknown> = Promise.resolve();
const oneAtATime = <T>(run: () => Promise<T>): Promise<T> => {
  const next = devtoQueue.then(run, run);
  devtoQueue = next.catch(() => {});
  return next;
};

/** Mastodon gives HTML. Parsed inert (DOMParser runs no scripts) and read back as plain text, keeping line breaks. */
function htmlToText(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('br').forEach((b) => b.replaceWith('\n'));
  doc.querySelectorAll('p').forEach((p) => p.append('\n\n'));
  // Mastodon hides the middle of long links in "invisible" spans; drop the protocol part the same way it does.
  doc.querySelectorAll('.invisible').forEach((s) => s.remove());
  doc.querySelectorAll('.ellipsis').forEach((s) => s.append('…'));
  return (doc.body.textContent ?? '').replace(/\n{3,}/g, '\n\n').trim();
}

const ADULT = new Set(['porn', 'sexual', 'nudity', 'graphic-media', 'gore', 'nsfw']);

/** An image, a link card's picture or a video's still; a quote post with media has one of these under `media`. */
type BskyMedia = { images?: { thumb: string }[]; external?: { thumb?: string }; thumbnail?: string };
const bskyImage = (m?: BskyMedia) => m?.images?.[0]?.thumb ?? m?.thumbnail ?? m?.external?.thumb;

type BskyPost = {
  uri: string;
  author: { did: string; handle: string; displayName?: string; avatar?: string; labels?: { val: string }[] };
  record: { text?: string; createdAt?: string; reply?: unknown };
  embed?: BskyMedia & { media?: BskyMedia };
  likeCount?: number; repostCount?: number; replyCount?: number; quoteCount?: number;
  indexedAt: string;
  labels?: { val: string }[];
};

async function bluesky(interest: string, signal: AbortSignal): Promise<Post[]> {
  const since = new Date(Date.now() - 4 * DAY).toISOString();
  const q = new URLSearchParams({ q: interest, limit: '25', sort: prefs.sort, since });
  const { posts } = await getJson<{ posts: BskyPost[] }>(`https://api.bsky.app/xrpc/app.bsky.feed.searchPosts?${q}`, signal);
  return posts
    .filter((p) => p.record.text && !p.record.reply && ![...(p.labels ?? []), ...(p.author.labels ?? [])].some((l) => ADULT.has(l.val)))
    .map((p) => {
      const rkey = p.uri.split('/').pop();
      const image = bskyImage(p.embed) ?? bskyImage(p.embed?.media);
      return {
        id: `bsky:${p.uri}`,
        source: 'bluesky' as const,
        url: `https://bsky.app/profile/${p.author.handle}/post/${rkey}`,
        author: { name: p.author.displayName || p.author.handle, handle: `@${p.author.handle}`, avatar: p.author.avatar },
        text: p.record.text ?? '',
        image,
        tags: [],
        interest,
        at: Date.parse(p.record.createdAt ?? p.indexedAt) || Date.parse(p.indexedAt),
        likes: p.likeCount ?? 0,
        reposts: (p.repostCount ?? 0) + (p.quoteCount ?? 0),
        replies: p.replyCount ?? 0,
      };
    });
}

type DevArticle = {
  id: number; title: string; description: string; url: string; cover_image: string | null; social_image?: string;
  tag_list: string[]; published_at: string; public_reactions_count: number; comments_count: number;
  user: { name: string; username: string; profile_image_90: string };
};

async function devto(interest: string, signal: AbortSignal): Promise<Post[]> {
  const tag = asTag(interest);
  if (!tag) return [];
  // Top: the week's most liked under the tag. Latest: its newest.
  const q = new URLSearchParams({ tag, per_page: '15', ...(prefs.sort === 'top' ? { top: '7' } : {}) });
  const list = await oneAtATime(() => getJson<DevArticle[]>(`https://dev.to/api/articles${prefs.sort === 'latest' ? '/latest' : ''}?${q}`, signal));
  return list.map((a) => ({
    id: `devto:${a.id}`,
    source: 'devto' as const,
    url: a.url,
    author: { name: a.user.name, handle: `@${a.user.username}`, avatar: a.user.profile_image_90 },
    title: a.title,
    text: a.description,
    image: a.cover_image ?? undefined,
    tags: a.tag_list,
    interest,
    at: Date.parse(a.published_at),
    likes: a.public_reactions_count,
    reposts: 0,
    replies: a.comments_count,
  }));
}

type MastoStatus = {
  id: string; url: string | null; uri: string; created_at: string; content: string; sensitive: boolean; spoiler_text: string;
  in_reply_to_id: string | null; reblogs_count: number; favourites_count: number; replies_count: number;
  account: { display_name: string; acct: string; avatar_static: string; bot?: boolean };
  media_attachments: { type: string; preview_url: string | null }[];
  tags: { name: string }[];
};

const MASTODON = 'https://mastodon.social';

async function mastodon(interest: string, signal: AbortSignal): Promise<Post[]> {
  const tag = asTag(interest);
  if (!tag) return [];
  const list = await getJson<MastoStatus[]>(`${MASTODON}/api/v1/timelines/tag/${tag}?limit=30`, signal);
  return list
    .filter((s) => !s.sensitive && !s.spoiler_text && !s.in_reply_to_id && !s.account.bot)
    .map((s) => ({
      id: `masto:${s.uri}`,
      source: 'mastodon' as const,
      url: s.url ?? s.uri,
      author: { name: s.account.display_name || s.account.acct, handle: `@${s.account.acct}`, avatar: s.account.avatar_static },
      text: htmlToText(s.content),
      image: s.media_attachments.find((m) => m.type === 'image' && m.preview_url)?.preview_url ?? undefined,
      tags: s.tags.map((t) => t.name),
      interest,
      at: Date.parse(s.created_at),
      likes: s.favourites_count,
      reposts: s.reblogs_count,
      replies: s.replies_count,
    }))
    .filter((p) => p.text);
}

const FETCH: Record<Source, (interest: string, signal: AbortSignal) => Promise<Post[]>> = { bluesky, devto, mastodon };

/** Engagement for its age: a post with many likes from yesterday beats one with a few from a minute ago, but
 *  freshness wins over a week-old hit. Each source counts differently, so each is weighed against its own best. */
function rank(posts: Post[]): Post[] {
  const raw = (p: Post) => Math.log1p(p.likes + 2 * p.reposts + 1.5 * p.replies);
  const best: Partial<Record<Source, number>> = {};
  for (const p of posts) best[p.source] = Math.max(best[p.source] ?? 0, raw(p));
  const score = (p: Post) => {
    const eng = raw(p) / (best[p.source] || 1);
    const hours = Math.max(0, (Date.now() - p.at) / 3_600_000);
    return (0.25 + eng) / Math.pow(1 + hours / 18, 0.7);
  };
  return posts.map((p) => [p, score(p)] as const).sort((a, b) => b[1] - a[1]).map(([p]) => p);
}

/** Spreads the list so no source or interest fills a run of cards on its own. */
function interleave(posts: Post[]): Post[] {
  const out: Post[] = [];
  const left = [...posts];
  while (left.length) {
    const last = out.slice(-2);
    const i = left.findIndex((p) => !last.some((q) => q.source === p.source && q.interest === p.interest));
    out.push(...left.splice(i < 0 ? 0 : i, 1));
  }
  return out;
}

// ---- The feed, kept for the run so going back to the page doesn't fetch it all again ----

export type Feed = { posts: Post[]; loading: boolean; errors: string[]; at: number; key: string };
let feed: Feed = { posts: [], loading: false, errors: [], at: 0, key: '' };
const feedSubs = new Set<() => void>();
const setFeed = (f: Partial<Feed>) => { feed = { ...feed, ...f }; feedSubs.forEach((s) => s()); };
export function useFeed(): Feed {
  return useSyncExternalStore((f) => { feedSubs.add(f); return () => feedSubs.delete(f); }, () => feed);
}

let running: AbortController | null = null;
const prefsKey = () => JSON.stringify([prefs.interests, [...prefs.sources].sort(), prefs.sort]);

/** Fetches the feed unless the one we have is for the same interests and under ten minutes old. */
export async function refreshFeed(force = false) {
  const key = prefsKey();
  if (!force && feed.key === key && (feed.loading || Date.now() - feed.at < 10 * 60_000)) return;
  running?.abort();
  const ctl = new AbortController();
  running = ctl;
  setFeed({ loading: true, key, errors: [] });
  const jobs = prefs.sources.flatMap((s) => prefs.interests.map((i) => ({ s, i, run: FETCH[s](i, ctl.signal) })));
  const results = await Promise.allSettled(jobs.map((j) => j.run));
  if (ctl.signal.aborted) return;
  const seen = new Set<string>();
  const posts: Post[] = [];
  const failed = new Set<Source>();
  results.forEach((r, n) => {
    if (r.status === 'rejected') { failed.add(jobs[n].s); return; }
    for (const p of r.value) {
      if (seen.has(p.id) || !Number.isFinite(p.at)) continue;
      seen.add(p.id);
      posts.push(p);
    }
  });
  const errors = [...failed].map((s) => `${SOURCES.find(([id]) => id === s)?.[1]} didn't answer for some interests.`);
  const sorted = prefs.sort === 'latest' ? posts.sort((a, b) => b.at - a.at) : interleave(rank(posts));
  setFeed({ posts: sorted.slice(0, 200), loading: false, errors, at: Date.now() });
}
