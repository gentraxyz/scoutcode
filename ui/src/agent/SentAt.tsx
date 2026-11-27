// When a message was sent, shown under it while you hover the message (like most chat apps), instead of a tooltip.

const fmt = (d: Date) => {
  const now = new Date();
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (now.toDateString() === d.toDateString()) return time;
  const day = d.toLocaleDateString([], { month: 'short', day: 'numeric', year: d.getFullYear() === now.getFullYear() ? undefined : 'numeric' });
  return `${day}, ${time}`;
};

export function SentAt({ at }: { at: string | number }) {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  return (
    <time className="agent-time" dateTime={d.toISOString()}>
      <span>{fmt(d)}</span>
    </time>
  );
}
