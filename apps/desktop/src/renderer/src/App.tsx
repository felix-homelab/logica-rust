// Spike S7's window: proves the path window → preload → MessagePort → engine process → Rust
// engine and back, and measures the cold start. The viewer itself comes in M2.
import { useEffect, useState } from 'react';
import { Client, CommandError, type Output } from '@logica/api';

const client = new Client({ call: (command, input) => window.logica.call(command, input) });

type State =
  | { kind: 'waiting' }
  | { kind: 'connected'; session: Output<'session.get'>; coldStartMs: number }
  | { kind: 'failed'; message: string };

export function App() {
  const [state, setState] = useState<State>({ kind: 'waiting' });
  const [events, setEvents] = useState(0);

  useEffect(() => {
    const stop = window.logica.onEvent(() => setEvents((n) => n + 1));
    client
      .run('session.get', {})
      .then((session) => {
        const coldStartMs = Date.now() - window.logica.launchedAt();
        setState({ kind: 'connected', session, coldStartMs });
        window.logica.ready(coldStartMs);
      })
      .catch((e: unknown) => {
        const message = e instanceof CommandError ? e.message : String(e);
        setState({ kind: 'failed', message });
        window.logica.failed(message);
      });
    return stop;
  }, []);

  return (
    <main
      style={{
        font: '14px system-ui, sans-serif',
        color: '#d8dce4',
        background: '#16181d',
        minHeight: '100vh',
        padding: 24,
        margin: 0,
      }}
    >
      <h1 style={{ fontSize: 20, fontWeight: 600 }}>Logica</h1>
      {state.kind === 'waiting' && <p>Starting the engine…</p>}
      {state.kind === 'failed' && <p role="alert">The window could not reach the engine: {state.message}</p>}
      {state.kind === 'connected' && (
        <dl>
          <dt>Engine</dt>
          <dd>Connected, command API {state.session.apiVersion}</dd>
          <dt>Cold start</dt>
          <dd>{Math.round(state.coldStartMs)} ms from the app's main process starting to the engine's first answer</dd>
          <dt>Open captures</dt>
          <dd>{state.session.captures.length}</dd>
          <dt>Devices</dt>
          <dd>{state.session.devices.length} available</dd>
          <dt>Engine events</dt>
          <dd>{events}</dd>
        </dl>
      )}
    </main>
  );
}
