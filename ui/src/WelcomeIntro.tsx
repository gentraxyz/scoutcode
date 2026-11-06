// The first thing a new user sees: Scout logo, "Welcome to Scout", and "The gateway to free AI models"
import { useEffect } from 'react';
import { useTranslation } from './i18n';
import { ScoutLogo } from './logo';
import { Icon } from './widgets';

export function WelcomeIntro({ onStart }: { onStart: () => void }) {
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
      <div className="welcome-hero">
        <ScoutLogo size={64} className="welcome-logo" />
        <div className="welcome-copy">
          <h1 className="welcome-title">Welcome to Scout</h1>
          <p className="welcome-tagline">The gateway to free AI models</p>
        </div>
      </div>
      <button type="button" className="button primary tall welcome-start" onClick={onStart}>
        {t('welcome_start')}<Icon name="arrowRight" size={14} />
      </button>
    </div>
  );
}
