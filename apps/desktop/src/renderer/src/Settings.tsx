// The settings window (doc 08): theme, MCP for agents, expert mode, memory, updates.
import { useEffect, useState } from 'react';
import { Badge, Button, Switch, TextField } from '@logica/ui';
import { bridge, type Settings as S } from './bridge';
import { useStore } from './store';

export function Settings() {
  const { settings, mcp, session } = useStore();
  const [draft, setDraft] = useState<S | null>(settings);
  useEffect(() => setDraft(settings), [settings]);
  if (!draft) return <main className="pad">Loading the settings…</main>;
  const save = (next: S) => {
    setDraft(next);
    void bridge().setSettings(next);
  };
  const agents = new Set((session?.recent ?? []).map((h) => h.actor).filter((a) => a.startsWith('agent:')));
  return (
    <main className="settings">
      <h1 className="settings__title">Settings</h1>

      <section className="settings__group" aria-labelledby="s-look">
        <h2 id="s-look" className="section-title">
          Appearance
        </h2>
        <label className="lg-field">
          <span className="lg-field__label">Theme</span>
          <select className="lg-input" value={draft.theme} onChange={(e) => save({ ...draft, theme: e.target.value as S['theme'] })}>
            <option value="system">Like the system</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </label>
        <Switch
          label="Expert mode"
          description="Hides the explanations of detections, evidence and the queries the search builds."
          checked={draft.expert}
          onCheckedChange={(expert) => save({ ...draft, expert })}
        />
      </section>

      <section className="settings__group" aria-labelledby="s-agents">
        <h2 id="s-agents" className="section-title">
          Agents
        </h2>
        <Switch
          label="Let agents connect (MCP)"
          description="Agents such as Claude Code reach Logica on 127.0.0.1 with a token from the lock file; nothing outside this computer can."
          checked={draft.mcp}
          onCheckedChange={(on) => save({ ...draft, mcp: on })}
        />
        <p className="muted" role="status">
          {mcp.port ? (
            <>
              <Badge tone="success">on</Badge> at http://127.0.0.1:{mcp.port}/mcp
            </>
          ) : mcp.error ? (
            <>
              <Badge tone="warning">not started</Badge> {mcp.error}
            </>
          ) : (
            <Badge>off</Badge>
          )}
        </p>
        <p className="muted">
          Agents seen in this session: {agents.size ? [...agents].map((a) => a.slice(6)).join(', ') : 'none yet'}.
        </p>
      </section>

      <section className="settings__group" aria-labelledby="s-engine">
        <h2 id="s-engine" className="section-title">
          Engine
        </h2>
        <TextField
          label="Memory for capture data (MB)"
          type="number"
          min={64}
          max={65536}
          defaultValue={String(draft.memoryMb)}
          hint="Data beyond this stays on disk and is read when shown. Takes effect when Logica starts next."
          onBlur={(e) => save({ ...draft, memoryMb: Number(e.target.value) })}
        />
      </section>

      <section className="settings__group" aria-labelledby="s-updates">
        <h2 id="s-updates" className="section-title">
          Updates
        </h2>
        <Switch
          label="Check for updates"
          description="Looks for a new version on GitHub Releases at start (Windows and Linux)."
          checked={draft.updates}
          onCheckedChange={(updates) => save({ ...draft, updates })}
        />
      </section>

      <div className="row">
        <Button onClick={() => window.close()}>Close</Button>
      </div>
    </main>
  );
}
