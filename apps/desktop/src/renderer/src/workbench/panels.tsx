// The sidebar and inspector panels (doc 08): channels, analysis, decoders, search, device. Every
// change goes through a command (ADR 0001); the panels redraw from the session the store re-reads.
import { useEffect, useMemo, useState } from 'react';
import type { Input, Output } from '@logica/api';
import { Badge, Button, EmptyState, IconButton, Menu, MenuItem, Switch, TextField } from '@logica/ui';
import { Eye, EyeOff, Palette, Plus, Search as SearchIcon, Trash2, Usb, WandSparkles } from 'lucide-react';
import { at, hz, int } from '../format';
import { isAgent, useStore, who, type CaptureInfo } from '../store';
import type { ViewState } from './Waveform';

type Analysis = Output<'analysis.detect'>;
type Protocols = Output<'protocol.list'>['protocols'];

const COLOURS = ['#e54242', '#ff7f00', '#007fff', '#26ad34', '#c08cff', '#f5b041', '#4cc3c3', '#e6e8ec'];

export function ChannelList({ capture }: { capture: CaptureInfo }) {
  const { run } = useStore();
  const [editing, setEditing] = useState<number | null>(null);
  const [name, setName] = useState('');
  const groups = useMemo(() => {
    const m = new Map<string, number[]>();
    for (const c of capture.channels) if (c.group) m.set(c.group, [...(m.get(c.group) ?? []), c.index]);
    return [...m.entries()];
  }, [capture.channels]);
  const rename = (channel: number) => {
    if (name.trim()) void run('channel.rename', { capture: capture.id, channel, name: name.trim() });
    setEditing(null);
  };
  return (
    <div className="channels">
      <ul className="channels__list" aria-label="Channels">
        {capture.channels.map((c, i) => (
          <li key={c.index} className="channels__row" data-hidden={c.hidden || undefined}>
            <span className="channels__swatch" style={{ background: c.colour ?? COLOURS[i % 4] }} aria-hidden="true" />
            {editing === c.index ? (
              <input
                className="lg-input channels__edit"
                aria-label={`New name for CH${c.index}`}
                value={name}
                autoFocus
                onChange={(e) => setName(e.target.value)}
                onBlur={() => rename(c.index)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') rename(c.index);
                  if (e.key === 'Escape') setEditing(null);
                }}
              />
            ) : (
              <button
                type="button"
                className="channels__name"
                title="Double-click or F2 to rename"
                onDoubleClick={() => {
                  setEditing(c.index);
                  setName(c.name);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'F2') {
                    setEditing(c.index);
                    setName(c.name);
                  }
                }}
              >
                <span className="channels__index lg-num">CH{c.index}</span> {c.name !== `CH${c.index}` ? c.name : ''}
              </button>
            )}
            <Menu label={`Colour of CH${c.index}`} trigger={<IconButton label={`Colour of CH${c.index}`} icon={<Palette />} />}>
              {COLOURS.map((col) => (
                <MenuItem key={col} icon={<span className="channels__swatch" style={{ background: col }} />} onSelect={() => void run('channel.colour', { capture: capture.id, channel: c.index, colour: col })}>
                  {col}
                </MenuItem>
              ))}
            </Menu>
            <IconButton
              label={c.hidden ? `Show CH${c.index}` : `Hide CH${c.index}`}
              pressed={c.hidden}
              icon={c.hidden ? <EyeOff /> : <Eye />}
              onClick={() => void run('channel.hide', { capture: capture.id, channel: c.index, hidden: !c.hidden })}
            />
          </li>
        ))}
      </ul>
      {groups.length > 0 && (
        <div className="channels__groups">
          <h3 className="section-title">Buses</h3>
          {groups.map(([g, list]) => (
            <p key={g} className="muted">
              {g}: {list.map((x) => `CH${x}`).join(', ')}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

const pct = (x: number) => `${Math.round(x * 100)} %`;

function rolesText(roles: Record<string, number | null>): string {
  return Object.entries(roles)
    .filter(([, ch]) => ch !== null)
    .map(([r, ch]) => `${r} CH${ch}`)
    .join(', ');
}

export function AnalysisPanel({ capture, expert, view }: { capture: CaptureInfo; expert: boolean; view: ViewState | undefined }) {
  const { run } = useStore();
  const [result, setResult] = useState<{ capture: string; analysis: Analysis } | null>(null);
  const [busy, setBusy] = useState(false);
  const detect = async (range?: { from: number; to: number }) => {
    setBusy(true);
    const a = await run('analysis.detect', { capture: capture.id, ...(range ?? {}) });
    setBusy(false);
    if (a) setResult({ capture: capture.id, analysis: a });
  };
  useEffect(() => {
    if (result?.capture !== capture.id) void detect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [capture.id]);
  const a = result?.capture === capture.id ? result.analysis : null;
  const has = (protocol: string, roles: Record<string, number | null>) =>
    capture.decoders.some((d) => d.config.protocol === protocol && JSON.stringify(d.config.roles) === JSON.stringify(roles));
  return (
    <div className="pad stack">
      <div className="row">
        <Button size="sm" icon={<WandSparkles />} onClick={() => void detect()} disabled={busy}>
          Detect in the whole capture
        </Button>
        {view && (
          <Button size="sm" variant="ghost" onClick={() => void detect({ from: view.from, to: view.to })} disabled={busy}>
            In the view
          </Button>
        )}
      </div>
      {busy && <p className="muted" role="status">Looking at the signals…</p>}
      {a && a.buses.every((b) => b.chosen.length === 0) && (
        <EmptyState title="No protocol was recognised">The channels are listed below with what was measured on them.</EmptyState>
      )}
      {a?.buses.map((b) => (
        <section key={b.bus.id} className="card" aria-label={`Bus ${b.bus.channels.map((c) => `CH${c}`).join(', ')}`}>
          {b.chosen.map((d) => (
            <div key={`${d.protocolId}${rolesText(d.roles)}`} className="stack-sm">
              <div className="row between">
                <strong>{d.protocolId.toUpperCase()}</strong>
                <Badge tone={d.confidence >= 0.8 ? 'success' : d.confidence >= 0.5 ? 'warning' : 'neutral'}>{pct(d.confidence)} sure</Badge>
              </div>
              <p className="muted">{rolesText(d.roles)}</p>
              {!expert && (
                <ul className="evidence">
                  {d.evidence.map((e) => (
                    <li key={e.text} data-against={!e.supports || undefined}>
                      <span className="evidence__mark" aria-hidden="true">
                        {e.supports ? '+' : '−'}
                      </span>
                      <span className="lg-visually-hidden">{e.supports ? 'For: ' : 'Against: '}</span>
                      {e.text}
                    </li>
                  ))}
                </ul>
              )}
              {d.warnings.map((w) => (
                <p key={w} className="warning">
                  {w}
                </p>
              ))}
              <Button
                size="sm"
                variant="primary"
                icon={<Plus />}
                disabled={has(d.protocolId, d.roles)}
                onClick={() => void run('decoder.add', { capture: capture.id, protocol: d.protocolId, roles: d.roles, settings: d.settings })}
              >
                {has(d.protocolId, d.roles) ? 'Decoder added' : 'Add decoder'}
              </Button>
            </div>
          ))}
          {b.chosen.length === 0 && <p className="muted">{b.bus.reason}</p>}
          {!expert && b.alternatives.length > 0 && (
            <details>
              <summary>Also possible ({b.alternatives.length})</summary>
              <ul className="plain">
                {b.alternatives.map((d) => (
                  <li key={`${d.protocolId}${rolesText(d.roles)}`}>
                    {d.protocolId.toUpperCase()} on {rolesText(d.roles)}: {pct(d.confidence)}
                  </li>
                ))}
              </ul>
            </details>
          )}
          {!expert && b.rejections.length > 0 && (
            <details>
              <summary>Ruled out ({b.rejections.length})</summary>
              <ul className="plain">
                {b.rejections.map((r) => (
                  <li key={r.protocolId}>
                    <strong>{r.protocolId.toUpperCase()}</strong>: {r.because}
                  </li>
                ))}
              </ul>
            </details>
          )}
          {b.unexplained.length > 0 && <p className="muted">Not explained: {b.unexplained.map((c) => `CH${c}`).join(', ')}</p>}
        </section>
      ))}
      {a && (
        <section className="card">
          <h3 className="section-title">Channels</h3>
          <ul className="plain">
            {a.measurements.map((m) => (
              <li key={m.channel}>
                CH{m.channel}: {int(m.edgeCount)} edges, rests {m.restLevel}
                {m.periodic && capture.sampleRateHz > 0 ? `, clock-like at ${hz(capture.sampleRateHz / m.periodic.periodSamples)}` : ''}
                {m.glitches > 0 ? `, ${m.glitches} glitches` : ''}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function SettingField({
  spec,
  value,
  onChange,
}: {
  spec: Protocols[number]['settings'][number];
  value: unknown;
  onChange: (v: string | number | boolean) => void;
}) {
  const k = spec.kind;
  if (k.kind === 'boolean') {
    return <Switch label={spec.label} description={spec.help ?? undefined} checked={Boolean(value ?? k.default)} onCheckedChange={onChange} />;
  }
  if (k.kind === 'choice') {
    return (
      <label className="lg-field">
        <span className="lg-field__label">{spec.label}</span>
        <select className="lg-input" value={String(value ?? k.default)} onChange={(e) => onChange(e.target.value)}>
          {k.options.map((o) => {
            const [v, label] = o as [string, string];
            return (
              <option key={v} value={v}>
                {label}
              </option>
            );
          })}
        </select>
      </label>
    );
  }
  return (
    <TextField
      label={`${spec.label}${k.unit ? ` (${k.unit})` : ''}`}
      type="number"
      defaultValue={String(value ?? k.default)}
      {...(k.min !== null && k.min !== undefined ? { min: k.min } : {})}
      {...(k.max !== null && k.max !== undefined ? { max: k.max } : {})}
      hint={spec.help ?? undefined}
      onBlur={(e) => {
        const n = Number(e.target.value);
        if (Number.isFinite(n)) onChange(n);
      }}
    />
  );
}

export function DecodersPanel({ capture }: { capture: CaptureInfo }) {
  const { run, notify } = useStore();
  const [protocols, setProtocols] = useState<Protocols>([]);
  const [adding, setAdding] = useState('');
  const [roles, setRoles] = useState<Record<string, number | null>>({});
  useEffect(() => {
    void run('protocol.list', {}).then((p) => p && setProtocols(p.protocols));
  }, [run]);
  const byId = useMemo(() => new Map(protocols.map((p) => [p.id, p])), [protocols]);
  const chosen = byId.get(adding);
  const update = async (decoder: string, settings: Record<string, string | number | boolean>) => {
    const r = await run('decoder.update', { capture: capture.id, decoder, settings });
    if (!r) return;
    const changes = r.problemChanges.map((c) => `${c.code}: ${c.before} → ${c.after}`);
    notify('neutral', changes.length ? `Problems changed: ${changes.join(', ')}.` : 'Decoded again; the problem counts did not change.');
  };
  return (
    <div className="pad stack">
      {capture.decoders.length === 0 && <EmptyState title="No decoders yet">Add one from the analysis, or choose a protocol below.</EmptyState>}
      {capture.decoders.map((d) => {
        const p = byId.get(d.config.protocol);
        const problems = Object.values(d.problemCounts).reduce((a, b) => a + (b ?? 0), 0);
        return (
          <section key={d.id} className="card stack-sm" aria-label={`Decoder ${d.config.name}`}>
            <div className="row between">
              <strong>{d.config.name}</strong>
              <IconButton label={`Remove ${d.config.name}`} icon={<Trash2 />} onClick={() => void run('decoder.remove', { capture: capture.id, decoder: d.id })} />
            </div>
            <p className="muted">
              {rolesText(d.config.roles)} · {int(d.packets)} packets · {int(d.frames)} frames ·{' '}
              {problems ? <Badge tone="danger">{int(problems)} problems</Badge> : <Badge tone="success">no problems</Badge>}
            </p>
            {d.notes.map((n) => (
              <p key={n} className="muted">
                {n}
              </p>
            ))}
            {p?.settings.map((s) => (
              <SettingField key={s.key} spec={s} value={d.config.settings[s.key]} onChange={(v) => void update(d.id, { [s.key]: v })} />
            ))}
          </section>
        );
      })}
      <section className="card stack-sm" aria-label="Add a decoder">
        <h3 className="section-title">Add a decoder</h3>
        <label className="lg-field">
          <span className="lg-field__label">Protocol</span>
          <select
            className="lg-input"
            value={adding}
            onChange={(e) => {
              setAdding(e.target.value);
              setRoles({});
            }}
          >
            <option value="">Choose…</option>
            {protocols
              .filter((p) => p.hasDecoder)
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
          </select>
        </label>
        {chosen?.roles.map((r) => (
          <label key={r.key} className="lg-field">
            <span className="lg-field__label">
              {r.label}
              {r.optional ? ' (optional)' : ''}
            </span>
            <select
              className="lg-input"
              value={roles[r.key] ?? ''}
              onChange={(e) => setRoles({ ...roles, [r.key]: e.target.value === '' ? null : Number(e.target.value) })}
            >
              <option value="">{r.optional ? 'None' : 'Choose…'}</option>
              {capture.channels.map((c) => (
                <option key={c.index} value={c.index}>
                  CH{c.index} {c.name !== `CH${c.index}` ? c.name : ''}
                </option>
              ))}
            </select>
          </label>
        ))}
        {chosen && (
          <Button
            variant="primary"
            size="sm"
            icon={<Plus />}
            disabled={chosen.roles.some((r) => !r.optional && (roles[r.key] ?? null) === null)}
            onClick={() => void run('decoder.add', { capture: capture.id, protocol: chosen.id, roles })}
          >
            Add {chosen.name}
          </Button>
        )}
      </section>
    </div>
  );
}

type Query = Input<'search.run'>['query'];

export function SearchPanel({ capture, onJump, expert }: { capture: CaptureInfo; onJump: (from: number, to: number) => void; expert: boolean }) {
  const { run } = useStore();
  const [kind, setKind] = useState<'decoded' | 'edge' | 'pulse'>(capture.decoders.length ? 'decoded' : 'edge');
  const [decoder, setDecoder] = useState(capture.decoders[0]?.id ?? '');
  const [text, setText] = useState('');
  const [problemsOnly, setProblemsOnly] = useState(false);
  const [channel, setChannel] = useState(capture.channels[0]?.index ?? 0);
  const [direction, setDirection] = useState<'rising' | 'falling' | 'any'>('rising');
  const [level, setLevel] = useState<'high' | 'low'>('high');
  const [longer, setLonger] = useState('');
  const [hits, setHits] = useState<Output<'search.run'> | null>(null);
  const [current, setCurrent] = useState(0);
  const query = useMemo<Query>(() => {
    if (kind === 'decoded') return { decoded: { decoder, hasProblem: problemsOnly, ...(text ? { text } : {}) } };
    if (kind === 'edge') return { edge: { channel, direction } };
    return { pulse: { channel, level, ...(longer ? { longerThan: Number(longer) } : {}) } };
  }, [kind, decoder, problemsOnly, text, channel, direction, level, longer]);
  const search = async () => {
    const r = await run('search.run', { capture: capture.id, query, limit: 1000 });
    if (r) {
      setHits(r);
      setCurrent(0);
      const h = r.hits[0];
      if (h) onJump(h.start, h.end);
    }
  };
  const go = (i: number) => {
    const h = hits?.hits[i];
    if (!h) return;
    setCurrent(i);
    onJump(h.start, h.end);
  };
  const rate = capture.sampleRateHz;
  return (
    <div className="pad stack">
      <form
        className="stack-sm"
        onSubmit={(e) => {
          e.preventDefault();
          void search();
        }}
      >
        <label className="lg-field">
          <span className="lg-field__label">Find</span>
          <select className="lg-input" value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>
            <option value="decoded" disabled={!capture.decoders.length}>
              Decoded content
            </option>
            <option value="edge">Edges</option>
            <option value="pulse">Pulses</option>
          </select>
        </label>
        {kind === 'decoded' && (
          <>
            <label className="lg-field">
              <span className="lg-field__label">Decoder</span>
              <select className="lg-input" value={decoder} onChange={(e) => setDecoder(e.target.value)}>
                {capture.decoders.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.config.name}
                  </option>
                ))}
              </select>
            </label>
            <TextField label="Containing" value={text} placeholder="such as CALIBRATED or 4F" onChange={(e) => setText(e.target.value)} />
            <Switch label="Only with problems" checked={problemsOnly} onCheckedChange={setProblemsOnly} />
          </>
        )}
        {kind !== 'decoded' && (
          <label className="lg-field">
            <span className="lg-field__label">Channel</span>
            <select className="lg-input" value={channel} onChange={(e) => setChannel(Number(e.target.value))}>
              {capture.channels.map((c) => (
                <option key={c.index} value={c.index}>
                  CH{c.index} {c.name !== `CH${c.index}` ? c.name : ''}
                </option>
              ))}
            </select>
          </label>
        )}
        {kind === 'edge' && (
          <label className="lg-field">
            <span className="lg-field__label">Direction</span>
            <select className="lg-input" value={direction} onChange={(e) => setDirection(e.target.value as typeof direction)}>
              <option value="rising">Rising</option>
              <option value="falling">Falling</option>
              <option value="any">Either</option>
            </select>
          </label>
        )}
        {kind === 'pulse' && (
          <>
            <label className="lg-field">
              <span className="lg-field__label">Level</span>
              <select className="lg-input" value={level} onChange={(e) => setLevel(e.target.value as typeof level)}>
                <option value="high">High pulses</option>
                <option value="low">Low pulses</option>
              </select>
            </label>
            <TextField label="Longer than (seconds)" type="number" value={longer} placeholder="such as 0.000005" onChange={(e) => setLonger(e.target.value)} />
          </>
        )}
        <Button type="submit" variant="primary" size="sm" icon={<SearchIcon />}>
          Search
        </Button>
        {!expert && <pre className="query">{JSON.stringify(query, null, 1)}</pre>}
      </form>
      {hits && (
        <section aria-label="Search hits" className="stack-sm">
          <div className="row between">
            <span role="status">
              {hits.hits.length === 0 ? 'No hits.' : `${int(hits.hits.length)} hits${hits.truncated ? ' (more exist)' : ''}`}
            </span>
            <span className="row">
              <Button size="sm" variant="ghost" disabled={current <= 0} onClick={() => go(current - 1)}>
                Previous
              </Button>
              <Button size="sm" variant="ghost" disabled={current >= hits.hits.length - 1} onClick={() => go(current + 1)}>
                Next
              </Button>
            </span>
          </div>
          <ul className="hits">
            {hits.hits.slice(0, 300).map((h, i) => (
              <li key={`${h.start}-${i}`}>
                <button type="button" className="hits__item" aria-current={i === current || undefined} onClick={() => go(i)}>
                  <span className="lg-num muted">{at(h.start, rate)}</span> {h.what}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

export function DevicePanel() {
  const { session, run } = useStore();
  const [config, setConfig] = useState<Record<string, { channels: number[]; rate: number; samples: number }>>({});
  if (!session) return null;
  return (
    <div className="pad stack">
      <Switch
        label="Stop agent actions"
        description="Agents cannot capture or drive outputs while this is on; running agent captures stop."
        checked={session.agentsPaused}
        onCheckedChange={(paused) => void run('session.pauseAgents', { paused })}
      />
      {session.devices.map((d) => {
        const id = d.info.id;
        const caps = d.capabilities;
        const c = config[id] ?? { channels: caps.channels.slice(0, 4), rate: caps.ratesHz.at(-1) ?? 1e6, samples: 1_000_000 };
        const set = (p: Partial<typeof c>) => setConfig({ ...config, [id]: { ...c, ...p } });
        const holder = d.holder;
        const running = session.running.some((r) => r.device === id);
        return (
          <section key={id} className="card stack-sm" aria-label={d.info.model}>
            <div className="row between">
              <strong>{d.info.model}</strong>
              {holder ? <Badge tone={isAgent(holder) ? 'agent' : 'accent'}>lease: {who(holder)}</Badge> : <Badge>{d.connected ? 'connected' : 'not connected'}</Badge>}
            </div>
            <p className="muted">
              {id} · {caps.channels.length} channels · up to {hz(Math.max(...caps.ratesHz))}
            </p>
            {!d.connected ? (
              <Button size="sm" icon={<Usb />} onClick={() => void run('device.connect', { device: id })}>
                Connect
              </Button>
            ) : (
              <>
                {holder && holder !== 'user' && (
                  <Button size="sm" variant="secondary" onClick={() => void run('lease.take', { device: id })}>
                    Take over from {who(holder)}
                  </Button>
                )}
                <fieldset className="channels-pick">
                  <legend className="lg-field__label">Channels</legend>
                  {caps.channels.map((ch) => (
                    <label key={ch} className="check">
                      <input
                        type="checkbox"
                        checked={c.channels.includes(ch)}
                        onChange={(e) => set({ channels: e.target.checked ? [...c.channels, ch].sort((a, b) => a - b) : c.channels.filter((x) => x !== ch) })}
                      />
                      CH{ch}
                    </label>
                  ))}
                </fieldset>
                <label className="lg-field">
                  <span className="lg-field__label">Sample rate</span>
                  <select className="lg-input" value={c.rate} onChange={(e) => set({ rate: Number(e.target.value) })}>
                    {caps.ratesHz.map((r) => (
                      <option key={r} value={r}>
                        {hz(r)}
                      </option>
                    ))}
                  </select>
                </label>
                <TextField label="Samples per channel" type="number" value={String(c.samples)} onChange={(e) => set({ samples: Number(e.target.value) })} />
                <div className="row">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={running}
                    onClick={async () => {
                      const ok = await run('device.configure', { device: id, config: { channels: c.channels, sampleRateHz: c.rate, samples: c.samples, mode: caps.modes[0] ?? 'buffer' } });
                      if (ok) await run('capture.start', { device: id, wait: false });
                    }}
                  >
                    Capture
                  </Button>
                  <Button size="sm" variant="secondary" disabled={!running} onClick={() => void run('capture.stop', { device: id })}>
                    Stop
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => void run('device.disconnect', { device: id })}>
                    Disconnect
                  </Button>
                </div>
              </>
            )}
          </section>
        );
      })}
    </div>
  );
}
