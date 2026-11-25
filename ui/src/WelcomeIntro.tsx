// The first thing a new user sees: a few agents, "Meet your agents", and what they do, over Scout's backdrop.
import { useEffect } from 'react';
import { Avatar } from './agent/Avatar';
import './agent/agent.css';
import { useTranslation } from './i18n';
import { ScoutLogo } from './logo';
import { SaturnScene } from './SaturnScene';
import { Icon } from './widgets';

/** `onSkip` takes every default and goes straight to the chat; everything is still in Settings. */
export function WelcomeIntro({ onStart, onSkip }: { onStart: () => void; onSkip: () => void }) {
  const { t } = useTranslation();

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Enter' && e.key !== ' ') return;
      e.preventDefault();
      onStart();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onStart]);

  return (
    <div className="welcome">
      <SaturnScene />
      <div className="welcome-hero">
        <div className="welcome-agents" aria-hidden>
          <Avatar shape="square" color="#22b8cf" size={46} />
          <Avatar shape="blob" color="#7c5cff" status="waiting" size={72} />
          <Avatar shape="star" color="#f5c518" size={46} />
        </div>
        <div className="welcome-copy">
          <h1 className="welcome-title">Meet your agents</h1>
          <p className="welcome-tagline">They work for you on their own computer in the cloud, even when Scout is closed.</p>
        </div>
        <span className="welcome-brand tiny dim"><ScoutLogo size={16} /> Scout</span>
      </div>
      <div className="welcome-actions">
        <button type="button" className="button primary tall welcome-start" onClick={onStart}>
          {t('welcome_start')}<Icon name="arrowRight" size={14} />
        </button>
        <button type="button" className="text-button welcome-skip" onClick={onSkip}>{t('welcome_skip')}</button>
        <span className="tiny dim">{t('welcome_skip_note')}</span>
      </div>
    </div>
  );
}
