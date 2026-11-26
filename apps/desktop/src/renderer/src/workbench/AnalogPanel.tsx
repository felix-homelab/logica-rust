// Virtual analog channels in the channel list (ADR 0010, doc 08): the channels with their range,
// and one dialog to add or change one: the source, its channels and settings, then the scaling,
// name, unit and style. Every change is a command (analog.add, analog.update), so it can be undone.
import { useEffect, useId, useState } from 'react';
import type { Input, Output } from '@logica/api';
import { Button, Dialog, IconButton, Status, TextField } from '@logica/ui';
import { Plus, SlidersHorizontal, Trash2 } from 'lucide-react';
import { int } from '../format';
import { formatValue } from '../analogplot';
import { useStore, type CaptureInfo } from '../store';
import { SettingField } from './panels';

type Sources = Output<'analog.sources'>['sources'];
type AnalogInfo = CaptureInfo['analog'][number];
type Settings = Record<string, string | number | boolean>;
type Scaling = Input<'analog.add'>['scaling'];

/** The sources of virtual analog channels, read once. */
function useSources(): Sources {
  const { run } = useStore();
  const [sources, setSources] = useState<Sources>([]);
  useEffect(() => {
    void run('analog.sources', {}).then((s) => s && setSources(s.sources));
  }, [run]);
  return sources;
}

/** What the form holds: numbers as typed, so a half-typed "-" or "0." stays. */
interface Form {
  source: string;
  roles: Record<string, number | null>;
  settings: Settings;
  kind: 'range' | 'linear';
  low: string;
  high: string;
  gain: string;
  offset: string;
  name: string;
  unit: string;
  style: 'step' | 'line';
}

const blank: Form = { source: '', roles: {}, settings: {}, kind: 'range', low: '0', high: '3.3', gain: '1', offset: '0', name: '', unit: '', style: 'step' };

function formOf(a: AnalogInfo): Form {
  const s = a.config.scaling;
  return {
    source: a.config.source,
    roles: { ...a.config.roles } as Record<string, number | null>,
    settings: { ...(a.config.settings ?? {}) } as Settings,
    kind: s.kind,
    low: s.kind === 'range' ? String(s.low) : '0',
    high: s.kind === 'range' ? String(s.high) : '1',
    gain: s.kind === 'linear' ? String(s.gain) : '1',
    offset: String(s.offset ?? 0),
    name: a.config.name,
    unit: a.config.unit ?? '',
    style: a.config.style ?? 'step',
  };
}

function scalingOf(f: Form): Scaling | null {
  const n = (t: string) => (t.trim() === '' ? NaN : Number(t));
  const offset = n(f.offset);
  if (f.kind === 'range') {
    const low = n(f.low);
    const high = n(f.high);
    return [low, high, offset].every(Number.isFinite) ? { kind: 'range', low, high, offset } : null;
  }
  const gain = n(f.gain);
  return [gain, offset].every(Number.isFinite) ? { kind: 'linear', gain, offset } : null;
}

/** The form for one channel, new (`editing` absent) or existing. */
function AnalogForm({ capture, sources, editing, onDone }: { capture: CaptureInfo; sources: Sources; editing?: AnalogInfo; onDone: () => void }) {
  const { run } = useStore();
  const uid = useId();
  const [f, setF] = useState<Form>(() => (editing ? formOf(editing) : blank));
  const set = (patch: Partial<Form>) => setF((old) => ({ ...old, ...patch }));
  const source = sources.find((s) => s.id === f.source);
  const scaling = scalingOf(f);
  const missing = source?.roles.some((r) => !r.optional && (f.roles[r.key] ?? null) === null) ?? true;
  const submit = async () => {
    if (!source || !scaling) return;
    const common = { scaling, style: f.style, ...(f.name.trim() ? { name: f.name.trim() } : {}), ...(f.unit.trim() ? { unit: f.unit.trim() } : {}) };
    const done = editing
      ? await run('analog.update', { capture: capture.id, analog: editing.id, source: source.id, roles: f.roles, settings: f.settings, replaceSettings: true, ...common })
      : await run('analog.add', { capture: capture.id, source: source.id, roles: f.roles, settings: f.settings, ...common });
    if (done) onDone();
  };
  const number = (label: string, key: 'low' | 'high' | 'gain' | 'offset', hint?: string) => (
    <TextField label={label} type="number" step="any" value={f[key]} onChange={(e) => set({ [key]: e.target.value })} {...(hint ? { hint } : {})} />
  );
  return (
    <form
      className="stack-sm"
      aria-label={editing ? `Change ${editing.config.name}` : 'Add a virtual analog channel'}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div className="lg-field">
        <label className="lg-field__label" htmlFor={`${uid}-source`}>
          Source
        </label>
        <select
          id={`${uid}-source`}
          className="lg-input"
          {...(source ? { 'aria-describedby': `${uid}-source-hint` } : {})}
          value={f.source}
          onChange={(e) => set({ source: e.target.value, roles: {}, settings: {} })}
        >
          <option value="">Choose…</option>
          {sources.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
        {source && (
          <span id={`${uid}-source-hint`} className="lg-field__hint">
            {source.help}
          </span>
        )}
      </div>
      {source?.roles.map((r) => (
        <label key={r.key} className="lg-field">
          <span className="lg-field__label">
            {r.label}
            {r.optional ? ' (optional)' : ''}
          </span>
          <select
            className="lg-input"
            value={f.roles[r.key] ?? ''}
            onChange={(e) => set({ roles: { ...f.roles, [r.key]: e.target.value === '' ? null : Number(e.target.value) } })}
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
      {source?.settings.map((s) => (
        <SettingField key={s.key} spec={s} value={f.settings[s.key]} onChange={(v) => set({ settings: { ...f.settings, [s.key]: v } })} />
      ))}
      {source && (
        <>
          <div className="lg-field">
            <label className="lg-field__label" htmlFor={`${uid}-scaling`}>
              Scaling
            </label>
            <select
              id={`${uid}-scaling`}
              className="lg-input"
              aria-describedby={`${uid}-scaling-hint`}
              value={f.kind}
              onChange={(e) => set({ kind: e.target.value as Form['kind'] })}
            >
              <option value="range">Lowest and highest value</option>
              <option value="linear">Gain and offset</option>
            </select>
            <span id={`${uid}-scaling-hint`} className="lg-field__hint">
              A fraction of full scale (an ADC code, a duty cycle) spans the lowest to the highest value; a measured frequency or time is multiplied by the gain.
            </span>
          </div>
          {f.kind === 'range' ? (
            <>
              {number('Lowest value', 'low', 'What the lowest code or 0 % stands for.')}
              {number('Highest value', 'high', 'What full scale or 100 % stands for.')}
            </>
          ) : (
            number('Gain', 'gain', 'Value per Hz or per second measured.')
          )}
          {number('Offset', 'offset', 'Added to every value, to correct a zero error.')}
          <TextField label="Name" value={f.name} placeholder={source.name} onChange={(e) => set({ name: e.target.value })} />
          <TextField label="Unit" value={f.unit} placeholder="V, %, Hz…" onChange={(e) => set({ unit: e.target.value })} />
          <label className="lg-field">
            <span className="lg-field__label">Draw as</span>
            <select className="lg-input" value={f.style} onChange={(e) => set({ style: e.target.value as Form['style'] })}>
              <option value="step">Steps: each value holds until the next</option>
              <option value="line">A line between the values</option>
            </select>
          </label>
          {!scaling && <p className="lg-field__error">Fill in every number of the scaling.</p>}
        </>
      )}
      <div className="row">
        <Button type="submit" variant="primary" size="sm" icon={editing ? undefined : <Plus />} disabled={!source || missing || !scaling}>
          {editing ? 'Apply' : 'Add channel'}
        </Button>
      </div>
    </form>
  );
}

/** The virtual analog channels of a capture, for the channel list. */
export function AnalogChannels({ capture }: { capture: CaptureInfo }) {
  const { run } = useStore();
  const sources = useSources();
  // null: closed; '': adding; an id: changing that channel
  const [dialog, setDialog] = useState<string | null>(null);
  const editing = capture.analog.find((a) => a.id === dialog);
  const range = (a: AnalogInfo) => {
    if (a.min === null || a.min === undefined || a.max === null || a.max === undefined) return 'no values';
    const unit = a.config.unit ?? '';
    return a.min === a.max ? formatValue(a.min, unit, a.step) : `${formatValue(a.min, unit, a.step)} to ${formatValue(a.max, unit, a.step)}`;
  };
  const dialogEl = (
    <Dialog
      open={dialog !== null}
      onOpenChange={(o) => !o && setDialog(null)}
      title={editing ? `${editing.config.name} settings` : 'Add a virtual analog channel'}
      description="Values computed from digital channels: an ADC, a PWM level and more. Each change can be undone."
    >
      {dialog !== null && <AnalogForm key={dialog} capture={capture} sources={sources} {...(editing ? { editing } : {})} onDone={() => setDialog(null)} />}
    </Dialog>
  );
  // with no channel yet, only the way to add one
  if (!capture.analog.length) {
    return (
      <div className="channels__add">
        <Button size="sm" icon={<Plus />} onClick={() => setDialog('')}>
          Add virtual analog channel
        </Button>
        {dialogEl}
      </div>
    );
  }
  return (
    <section className="channels__group" aria-label="Virtual analog channels">
      <div className="channels__group-head">
        <h3 className="channels__group-title">Virtual analog</h3>
        <IconButton label="Add virtual analog channel" icon={<Plus />} onClick={() => setDialog('')} />
      </div>
      <ul className="channels__list">
        {capture.analog.map((a) => {
          const problems = Object.values(a.problemCounts).reduce((x, y) => x + (y ?? 0), 0);
          return (
            <li key={a.id} className="channels__row">
              <span className="channels__name">
                {a.config.name}
                <span className="muted"> · {range(a)}</span>
                {problems > 0 && <Status tone="danger">{int(problems)} problems</Status>}
              </span>
              <IconButton label={`Settings of ${a.config.name}`} icon={<SlidersHorizontal />} onClick={() => setDialog(a.id)} />
              <IconButton label={`Remove ${a.config.name}`} icon={<Trash2 />} onClick={() => void run('analog.remove', { capture: capture.id, analog: a.id })} />
            </li>
          );
        })}
      </ul>
      {dialogEl}
    </section>
  );
}
