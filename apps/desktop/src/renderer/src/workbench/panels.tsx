// The sidebar and inspector panels (doc 08): channels, analysis, decoders, search, device. Every
// change goes through a command (ADR 0001); the panels redraw from the session the store re-reads.
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { Input, Output } from '@logica/api';
import { Button, Dialog, EmptyState, IconButton, Menu, MenuItem, Status, Switch, TextField } from '@logica/ui';
import { Activity, Check, Eye, EyeOff, Palette, Plus, RefreshCw, Search as SearchIcon, SlidersHorizontal, Trash2, Usb, WandSparkles } from 'lucide-react';
import { at, hz, int } from '../format';
import { isAgent, useDeviceWatch, useStore, who, type CaptureInfo } from '../store';
import { FORMAT_LABEL, groupChannels, roleOf, type DataFormat } from '../lanes';
import type { ViewState } from './Waveform';
import { AnalogChannels } from './AnalogPanel';

type Analysis = Output<'analysis.detect'>;
type Protocols = Output<'protocol.list'>['protocols'];

const COLOURS = ['#e54242', '#ff7f00', '#007fff', '#26ad34', '#c08cff', '#f5b041', '#4cc3c3', '#e6e8ec'];

type ChannelInfo = CaptureInfo['channels'][number];

/** The channels, grouped under the decoders that use them (as in the waveform); the rest after. */
export function ChannelList({ capture }: { capture: CaptureInfo }) {
  const { run } = useStore();
  const [editing, setEditing] = useState<number | null>(null);
  const [name, setName] = useState('');
  const buses = useMemo(() => {
    const m = new Map<string, number[]>();
    for (const c of capture.channels) if (c.group) m.set(c.group, [...(m.get(c.group) ?? []), c.index]);
    return [...m.entries()];
  }, [capture.channels]);
  const grouping = useMemo(() => groupChannels(capture.channels, capture.decoders), [capture.channels, capture.decoders]);
  // the decoder whose settings are open, read fresh from the session so changes show at once
  const [settingsOf, setSettingsOf] = useState<string | null>(null);
  const open = capture.decoders.find((d) => d.id === settingsOf);
  const protocols = useProtocols();
  const position = useMemo(() => new Map(capture.channels.map((c, i) => [c.index, i])), [capture.channels]);
  const rename = (channel: number) => {
    if (name.trim()) void run('channel.rename', { capture: capture.id, channel, name: name.trim() });
    setEditing(null);
  };
  const row = (c: ChannelInfo, role: string | null) => (
    <li key={c.index} className="channels__row" data-hidden={c.hidden || undefined}>
      <span className="channels__swatch" style={{ background: c.colour ?? COLOURS[(position.get(c.index) ?? 0) % 4] }} aria-hidden="true" />
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
          {/* the role, unless the channel's name already says it ("CS#" for CS) */}
          {role && !c.name.toUpperCase().includes(role) && (
            <>
              {' '}
              <span className="channels__role">{role}</span>
            </>
          )}
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
  );
  const { groups, other } = grouping;
  return (
    <div className="channels">
      {groups.length === 0 ? (
        <ul className="channels__list" aria-label="Channels">
          {capture.channels.map((c) => row(c, null))}
        </ul>
      ) : (
        <>
          {groups.map((g) => {
            const title = [g.decoder, ...g.extra].map((d) => d.config.name).join(' + ');
            return (
              <section key={g.decoder.id} className="channels__group" aria-label={`Channels of ${title}`}>
                <div className="channels__group-head">
                  <h3 className="channels__group-title">{title}</h3>
                  {[g.decoder, ...g.extra].map((d) => (
                    <IconButton key={d.id} label={`Settings of ${d.config.name}`} icon={<SlidersHorizontal />} onClick={() => setSettingsOf(d.id)} />
                  ))}
                </div>
                <ul className="channels__list">{g.channels.map((c) => row(c, roleOf(g.decoder, c.index)))}</ul>
              </section>
            );
          })}
          {other.length > 0 && (
            <section className="channels__group" aria-label="Other channels">
              <h3 className="channels__group-title">Other channels</h3>
              <ul className="channels__list">{other.map((c) => row(c, null))}</ul>
            </section>
          )}
        </>
      )}
      <AnalogChannels capture={capture} />
      <Dialog
        open={open !== undefined}
        onOpenChange={(o) => !o && setSettingsOf(null)}
        title={open ? `${open.config.name} settings` : ''}
        description="A change decodes the capture again at once; each one can be undone."
        footer={
          <Button variant="primary" onClick={() => setSettingsOf(null)}>
            Done
          </Button>
        }
      >
        {open && (
          <div className="stack-sm">
            <DecoderSettings capture={capture} decoder={open} protocol={protocols.find((p) => p.id === open.config.protocol)} />
          </div>
        )}
      </Dialog>
      {buses.length > 0 && (
        <div className="channels__groups">
          <h3 className="section-title">Buses</h3>
          {buses.map(([g, list]) => (
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
    <div className="sections">
      <div className="section section--bar">
        <span className="section__hint">Detect protocols</span>
        <div className="row">
          <Button size="sm" icon={<WandSparkles />} onClick={() => void detect()} disabled={busy}>
            Whole capture
          </Button>
          {view && (
            <Button size="sm" onClick={() => void detect({ from: view.from, to: view.to })} disabled={busy}>
              In the view
            </Button>
          )}
        </div>
      </div>
      {busy && (
        <p className="section muted" role="status">
          Looking at the signals…
        </p>
      )}
      {a && a.buses.every((b) => b.chosen.length === 0) && (
        <EmptyState title="No protocol was recognised">The channels are listed below with what was measured on them.</EmptyState>
      )}
      {a?.buses.map((b) => (
        <section key={b.bus.id} className="section stack-sm" aria-label={`Bus ${b.bus.channels.map((c) => `CH${c}`).join(', ')}`}>
          {b.chosen.map((d) => (
            <div key={`${d.protocolId}${rolesText(d.roles)}`} className="stack-sm">
              <div className="section__head">
                <h3 className="section__title">{d.protocolId.toUpperCase()}</h3>
                <span className="section__meta lg-num" data-tone={d.confidence >= 0.8 ? undefined : d.confidence >= 0.5 ? 'warning' : 'muted'}>
                  {pct(d.confidence)} sure
                </span>
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
              <div className="row">
                <Button
                  size="sm"
                  variant={has(d.protocolId, d.roles) ? 'ghost' : 'primary'}
                  icon={has(d.protocolId, d.roles) ? <Check /> : <Plus />}
                  disabled={has(d.protocolId, d.roles)}
                  onClick={() => void run('decoder.add', { capture: capture.id, protocol: d.protocolId, roles: d.roles, settings: d.settings })}
                >
                  {has(d.protocolId, d.roles) ? 'Decoder added' : 'Add decoder'}
                </Button>
              </div>
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
        <section className="section stack-sm" aria-labelledby="analysis-channels">
          <h3 id="analysis-channels" className="section__title">
            Channels
          </h3>
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

export function SettingField({
  spec,
  value,
  onChange,
}: {
  spec: Protocols[number]['settings'][number] | Output<'analog.sources'>['sources'][number]['settings'][number];
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
      // a new value from elsewhere (an agent, undo) replaces what the field shows
      key={String(value ?? k.default)}
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

/** The protocols the engine knows (for their settings and roles). */
function useProtocols(): Protocols {
  const { run } = useStore();
  const [protocols, setProtocols] = useState<Protocols>([]);
  useEffect(() => {
    void run('protocol.list', {}).then((p) => p && setProtocols(p.protocols));
  }, [run]);
  return protocols;
}

type DecoderInfo = CaptureInfo['decoders'][number];

/** A decoder's state and settings: in the Decoders panel and in the channel list's dialog. */
function DecoderSettings({ capture, decoder: d, protocol: p }: { capture: CaptureInfo; decoder: DecoderInfo; protocol: Protocols[number] | undefined }) {
  const { run, notify } = useStore();
  const update = async (settings: Record<string, string | number | boolean>) => {
    const r = await run('decoder.update', { capture: capture.id, decoder: d.id, settings });
    if (!r) return;
    const changes = r.problemChanges.map((c) => `${c.code}: ${c.before} → ${c.after}`);
    notify('neutral', changes.length ? `Problems changed: ${changes.join(', ')}.` : 'Decoded again; the problem counts did not change.');
  };
  const problems = Object.values(d.problemCounts).reduce((a, b) => a + (b ?? 0), 0);
  return (
    <>
      <p className="muted">
        {rolesText(d.config.roles)} · {int(d.packets)} packets · {int(d.frames)} frames ·{' '}
        {problems ? <Status tone="danger">{int(problems)} problems</Status> : <Status tone="success">no problems</Status>}
      </p>
      {d.notes.map((n) => (
        <p key={n} className="muted">
          {n}
        </p>
      ))}
      <label className="lg-field">
        <span className="lg-field__label">Show data as</span>
        <select
          className="lg-input"
          value={d.config.dataFormat ?? 'ascii'}
          onChange={(e) => void run('decoder.update', { capture: capture.id, decoder: d.id, dataFormat: e.target.value as DataFormat })}
        >
          {(Object.keys(FORMAT_LABEL) as DataFormat[]).map((f) => (
            <option key={f} value={f}>
              {FORMAT_LABEL[f]}
            </option>
          ))}
        </select>
        <span className="lg-field__hint">The row under the decoder&apos;s bytes in the waveform.</span>
      </label>
      {p?.settings.map((s) => (
        <SettingField key={s.key} spec={s} value={d.config.settings[s.key]} onChange={(v) => void update({ [s.key]: v })} />
      ))}
    </>
  );
}

export function DecodersPanel({ capture }: { capture: CaptureInfo }) {
  const { run } = useStore();
  const protocols = useProtocols();
  const [adding, setAdding] = useState('');
  const [roles, setRoles] = useState<Record<string, number | null>>({});
  const byId = useMemo(() => new Map(protocols.map((p) => [p.id, p])), [protocols]);
  const chosen = byId.get(adding);
  return (
    <div className="sections">
      {capture.decoders.length === 0 && <EmptyState title="No decoders yet">Add one from the analysis, or choose a protocol below.</EmptyState>}
      {capture.decoders.map((d) => (
        <section key={d.id} className="section stack-sm" aria-label={`Decoder ${d.config.name}`}>
          <div className="section__head">
            <h3 className="section__title">{d.config.name}</h3>
            <IconButton label={`Remove ${d.config.name}`} icon={<Trash2 />} onClick={() => void run('decoder.remove', { capture: capture.id, decoder: d.id })} />
          </div>
          <DecoderSettings capture={capture} decoder={d} protocol={byId.get(d.config.protocol)} />
        </section>
      ))}
      <section className="section stack-sm" aria-label="Add a decoder">
        <h3 className="section__title">Add a decoder</h3>
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
          <div className="row">
            <Button
              variant="primary"
              size="sm"
              icon={<Plus />}
              disabled={chosen.roles.some((r) => !r.optional && (roles[r.key] ?? null) === null)}
              onClick={() => void run('decoder.add', { capture: capture.id, protocol: chosen.id, roles })}
            >
              Add {chosen.name}
            </Button>
          </div>
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
    <div className="sections">
      <form
        className="section stack-sm"
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
        <div className="row">
          <Button type="submit" variant="primary" size="sm" icon={<SearchIcon />}>
            Search
          </Button>
        </div>
        {!expert && (
          <details className="query-details">
            <summary>The query this builds</summary>
            <pre className="query">{JSON.stringify(query, null, 1)}</pre>
          </details>
        )}
      </form>
      {hits && (
        <section aria-label="Search hits" className="section stack-sm">
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

/** The devices: what they are, who holds them, and whether they capture; "Capture live…" starts one. */
export function DevicePanel({ onCapture }: { onCapture: () => void }) {
  const { session, run, refresh } = useStore();
  const [looking, setLooking] = useState(false);
  // devices that come and go (a virtual analyser) are found when asked for
  const look = useCallback(async () => {
    setLooking(true);
    await run('device.list', {});
    setLooking(false);
    refresh();
  }, [run, refresh]);
  useDeviceWatch(true);
  if (!session) return null;
  return (
    <div className="sections">
      <div className="section section--bar">
        <Button size="sm" variant="primary" icon={<Activity />} onClick={onCapture}>
          Capture live…
        </Button>
        <Button size="sm" variant="secondary" icon={<RefreshCw />} disabled={looking} onClick={() => void look()}>
          {looking ? 'Looking…' : 'Look for devices'}
        </Button>
      </div>
      {session.devices.length === 0 && (
        <p className="section muted">
          No devices found. To try live capture without hardware, run <code>cargo run -p logica-vla</code> in the repository, then look again.
        </p>
      )}
      {session.devices.map((d) => {
        const id = d.info.id;
        const caps = d.capabilities;
        const holder = d.holder;
        return (
          <section key={id} className="section stack-sm" aria-label={d.info.model}>
            <div className="section__head">
              <h3 className="section__title">{d.info.model}</h3>
              {d.capturing ? (
                <Status tone="danger" pulse>
                  capturing
                </Status>
              ) : holder ? (
                <Status tone={isAgent(holder) ? 'agent' : 'neutral'}>held by {who(holder)}</Status>
              ) : (
                <Status tone={d.connected ? 'success' : 'neutral'}>{d.connected ? 'connected' : 'not connected'}</Status>
              )}
            </div>
            <p className="muted">
              {id} · {caps.channels.length} channels · up to {hz(Math.max(...caps.ratesHz))}
            </p>
            <div className="row">
              {holder && holder !== 'user' && (
                <Button size="sm" variant="secondary" onClick={() => void run('lease.take', { device: id })}>
                  Take over from {who(holder)}
                </Button>
              )}
              {d.capturing && (
                <Button size="sm" variant="secondary" onClick={() => void run('capture.stop', { device: id })}>
                  Stop capture
                </Button>
              )}
              {!d.connected ? (
                <Button size="sm" variant="ghost" icon={<Usb />} onClick={() => void run('device.connect', { device: id })}>
                  Connect
                </Button>
              ) : (
                !d.capturing && (
                  <Button size="sm" variant="ghost" onClick={() => void run('device.disconnect', { device: id })}>
                    Disconnect
                  </Button>
                )
              )}
            </div>
          </section>
        );
      })}
      <div className="section">
        <Switch
          label="Stop agent actions"
          description="Agents cannot capture or drive outputs while this is on; running agent captures stop."
          checked={session.agentsPaused}
          onCheckedChange={(paused) => void run('session.pauseAgents', { paused })}
        />
      </div>
    </div>
  );
}
