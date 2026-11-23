import { Component, type ErrorInfo, type ReactNode } from 'react';
import { invoke } from './api';
import { CopyButton } from './widgets';

type Props = {
  /** What broke, in the heading: "Scout", "This settings page". */
  what: string;
  children: ReactNode;
  /** The whole window: offers Reload, and makes sure the window shows even if it broke before its first paint. */
  root?: boolean;
};
type State = { error: Error | null; stack: string };

/** Shows what went wrong and where, instead of the blank window React leaves when something throws while drawing. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, stack: '' };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error(error, info.componentStack);
    this.setState({ stack: info.componentStack ?? '' });
    // The window starts hidden and main.tsx shows it after the app paints, which a crash at startup never does.
    if (this.props.root) invoke('window_ready').catch(() => {});
  }

  render() {
    const { error, stack } = this.state;
    if (!error) return this.props.children;
    const report = [
      `${error.name}: ${error.message}`,
      error.stack ?? '',
      stack ? `Component stack:${stack}` : '',
      `User agent: ${navigator.userAgent}`,
    ].filter(Boolean).join('\n\n');
    return (
      <div className={`error-screen ${this.props.root ? 'root' : ''}`} role="alert">
        <b className="strong">{this.props.what} ran into an error</b>
        <p className="small dim">Copy the details below and send them to us on Discord, so we can fix it.</p>
        <pre className="error-details">{report}</pre>
        <div className="row">
          <CopyButton text={report} label="Copy details" className="primary" />
          <button type="button" className="button" onClick={() => this.setState({ error: null, stack: '' })}>Try again</button>
          {this.props.root && <button type="button" className="button ghost" onClick={() => location.reload()}>Reload Scout</button>}
        </div>
      </div>
    );
  }
}
