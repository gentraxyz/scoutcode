// Social: one feed of what people are posting about the user's interests on Bluesky, Dev.to and Mastodon. No
// accounts: it reads their public APIs (socialFeed.ts). Clicking a card opens the post in the user's browser.
import { useEffect, useState, type KeyboardEvent } from 'react';
import { openExternal } from './api';
import { Icon, IconButton, Spinner } from './widgets';
import { cleanInterest, refreshFeed, setPrefs, SOURCES, useFeed, useSocialPrefs, type Post, type Source } from './socialFeed';
import './social.css';

const SUGGESTED = ['rust', 'typescript', 'react', 'python', 'ai', 'webdev', 'opensource', 'linux', 'gamedev', 'security', 'devops', 'golang'];

function ago(at: number): string {
  const s = Math.max(0, (Date.now() - at) / 1000);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d`;
  return new Date(at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

const count = (n: number) => (n >= 10_000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);
const sourceName = (s: Source) => SOURCES.find(([id]) => id === s)?.[1] ?? s;

function Avatar({ src, name }: { src?: string; name: string }) {
  const [broken, setBroken] = useState(false);
  if (!src || broken) return <span className="social-avatar social-avatar-letter">{name.charAt(0).toUpperCase()}</span>;
  return <img className="social-avatar" src={src} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setBroken(true)} />;
}

function Card({ p }: { p: Post }) {
  const [imgBroken, setImgBroken] = useState(false);
  return (
    <button type="button" className="social-card" onClick={() => openExternal(p.url)} title={`Open on ${sourceName(p.source)}`}>
      <div className="social-card-head">
        <Avatar src={p.author.avatar} name={p.author.name} />
        <div className="social-who">
          <span className="social-name ellipsis">{p.author.name}</span>
          <span className="social-handle ellipsis">{p.author.handle}</span>
        </div>
        <span className={`social-source ${p.source}`}>{sourceName(p.source)}</span>
        <span className="social-time" title={new Date(p.at).toLocaleString()}>{ago(p.at)}</span>
      </div>
      {p.title && <div className="social-title">{p.title}</div>}
      {p.text && <div className={`social-text ${p.title ? 'desc' : ''}`}>{p.text}</div>}
      {p.image && !imgBroken && <img className="social-image" src={p.image} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setImgBroken(true)} />}
      <div className="social-foot">
        <span title="Likes"><Icon name="star" size={13} />{count(p.likes)}</span>
        {p.source !== 'devto' && <span title="Reposts"><Icon name="repeat" size={13} />{count(p.reposts)}</span>}
        <span title="Replies"><Icon name="chat" size={13} />{count(p.replies)}</span>
        <span className="grow" />
        <span className="social-why">#{p.interest.replace(/\s+/g, '')}</span>
      </div>
    </button>
  );
}

function Interests() {
  const { interests } = useSocialPrefs();
  const [draft, setDraft] = useState('');
  const add = (raw: string) => {
    const v = cleanInterest(raw);
    if (v && !interests.includes(v) && interests.length < 20) setPrefs({ interests: [...interests, v] });
    setDraft('');
  };
  const key = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); add(draft); }
    else if (e.key === 'Backspace' && !draft && interests.length) setPrefs({ interests: interests.slice(0, -1) });
  };
  const ideas = SUGGESTED.filter((s) => !interests.includes(s)).slice(0, 6);
  return (
    <div className="social-interests">
      <div className="social-chips">
        {interests.map((i) => (
          <span key={i} className="social-chip">
            #{i}
            <button type="button" aria-label={`Remove ${i}`} title="Remove" onClick={() => setPrefs({ interests: interests.filter((x) => x !== i) })}><Icon name="close" size={11} /></button>
          </span>
        ))}
        <input
          className="social-add"
          value={draft}
          placeholder={interests.length ? 'Add an interest…' : 'What are you into? e.g. rust, design, ai'}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={key}
          onBlur={() => draft.trim() && add(draft)}
          maxLength={40}
          spellCheck={false}
        />
      </div>
      {ideas.length > 0 && interests.length < 8 && (
        <div className="social-ideas">
          <span className="dim tiny">Try</span>
          {ideas.map((s) => <button key={s} type="button" className="social-idea" onClick={() => add(s)}>+ {s}</button>)}
        </div>
      )}
    </div>
  );
}

function Controls() {
  const { sources, sort } = useSocialPrefs();
  const { loading } = useFeed();
  const toggle = (s: Source) => {
    const next = sources.includes(s) ? sources.filter((x) => x !== s) : [...sources, s];
    if (next.length) setPrefs({ sources: next });
  };
  return (
    <div className="social-controls">
      {SOURCES.map(([id, name]) => (
        <button key={id} type="button" className={`social-toggle ${id} ${sources.includes(id) ? 'on' : ''}`} aria-pressed={sources.includes(id)} onClick={() => toggle(id)}>
          <span className="dot" />{name}
        </button>
      ))}
      <span className="grow" />
      <div className="social-seg" role="group" aria-label="Order">
        {(['top', 'latest'] as const).map((s) => (
          <button key={s} type="button" className={sort === s ? 'on' : ''} aria-pressed={sort === s} onClick={() => setPrefs({ sort: s })}>{s === 'top' ? 'Top' : 'Latest'}</button>
        ))}
      </div>
      {loading ? <span className="social-spin"><Spinner size={14} /></span> : <IconButton icon="refresh" tip="Refresh" onClick={() => refreshFeed(true)} />}
    </div>
  );
}

export function SocialPage() {
  const prefs = useSocialPrefs();
  const { posts, loading, errors } = useFeed();
  const [shown, setShown] = useState(40);
  // Settles typing before fetching, so adding three interests in a row is one round of requests.
  useEffect(() => {
    if (!prefs.interests.length) return;
    const t = window.setTimeout(() => refreshFeed(), 350);
    return () => clearTimeout(t);
  }, [prefs]);
  useEffect(() => setShown(40), [posts]);
  return (
    <div className="social">
      <div className="social-body">
        <Interests />
        <Controls />
        {errors.length > 0 && <div className="social-error tiny">{errors.join(' ')}</div>}
        {!prefs.interests.length ? (
          <div className="inbox-zero">
            <Icon name="social" size={26} />
            <div className="strong">Add an interest to build your feed</div>
            <div className="tiny">Scout gathers public posts about it from Bluesky, Dev.to and Mastodon. No accounts needed.</div>
          </div>
        ) : posts.length === 0 ? (
          loading ? (
            <div className="social-list">{[0, 1, 2, 3].map((i) => <div key={i} className="social-card social-ghost" />)}</div>
          ) : (
            <div className="inbox-zero">
              <div className="strong">Nothing new about these right now</div>
              <div className="tiny">Try a broader interest, or switch to Latest.</div>
            </div>
          )
        ) : (
          <>
            <div className={`social-list ${loading ? 'stale' : ''}`}>{posts.slice(0, shown).map((p) => <Card key={p.id} p={p} />)}</div>
            {shown < posts.length && <button type="button" className="chip social-more" onClick={() => setShown(shown + 40)}>Show more</button>}
          </>
        )}
      </div>
    </div>
  );
}
