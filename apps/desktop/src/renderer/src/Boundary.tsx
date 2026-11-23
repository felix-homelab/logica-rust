// A part of the window that fails shows why and can be tried again; the rest keeps working.
import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Button } from '@logica/ui';
import { bridge } from './bridge';

interface State {
  error: Error | null;
}

export class Boundary extends Component<{ name: string; children: ReactNode }, State> {
  override state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    bridge().log('error', `The ${this.props.name} failed: ${error.stack ?? error.message}${info.componentStack ?? ''}`);
  }

  override render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="boundary" role="alert">
        <p>
          <strong>The {this.props.name} stopped working.</strong> {error.message}
        </p>
        <p className="muted">The rest of the window still works. The details are in the log file (Help, Open the log folder).</p>
        <Button size="sm" onClick={() => this.setState({ error: null })}>
          Try again
        </Button>
      </div>
    );
  }
}
