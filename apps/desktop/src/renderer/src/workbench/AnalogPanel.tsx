// Virtual analog channels in the channel list (ADR 0010, doc 08): the channels with their range,
// and one dialog in steps to add or change one: the source and its channels, the source's
// settings, the scaling, then the name and how it is drawn. Every change is a command
// (analog.add, analog.update), so it can be undone.
import { useEffect, useId, useState, type ReactNode } from 'react';
import type { Input, Output } from '@logica/api';
import { Button, Dialog, IconButton, Status, TextField } from '@logica/ui';
import { Check, Plus, SlidersHorizontal, Trash2 } from 'lucide-react';
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
  height: string;
}

const blank: Form = {
  source: '',
  roles: {},
  settings: {},
  kind: 'range',
  low: '0',
  high: '3.3',
  gain: '1',
  offset: '0',
  name: '',
  unit: '',
  style: 'step',
  height: '160',
};

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
    height: String(a.config.height ?? 160),
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

const STEPS = [
  { id: 'source', label: 'Source', description: 'Where the values come from, and the digital channels they are read from.' },
  { id: 'settings', label: 'Settings', description: 'How the source reads its channels.' },
  { id: 'scaling', label: 'Scaling', description: 'What the values stand for, in your own unit.' },
  { id: 'look', label: 'Name and look', description: 'How the channel is named and drawn in the waveform.' },
] as const;
type StepId = (typeof STEPS)[number]['id'];

/** The dialog for one channel, new (`editing` absent) or existing, in steps. */
function AnalogDialog({ capture, sources, editing, onClose }: { capture: CaptureInfo; sources: Sources; editing?: AnalogInfo; onClose: () => void }) {
  const { run } = useStore();
  const uid = useId();
  const [f, setF] = useState<Form>(() => (editing ? formOf(editing) : blank));
  const [at, setAt] = useState(0);
  const set = (patch: Partial<Form>) => setF((old) => ({ ...old, ...patch }));
  const source = sources.find((s) => s.id === f.source);
  const scaling = scalingOf(f);
  const height = Number(f.height);
  // what keeps each step from being finished, in words
  const problem: Record<StepId, string | null> = {
    source: !source
      ? 'Choose a source.'
      : source.roles.some((r) => !r.optional && (f.roles[r.key] ?? null) === null)
        ? 'Choose a channel for every role that is not optional.'
        : null,
    settings: null,
    scaling: scaling ? null : 'Fill in every number of the scaling.',
    look: Number.isInteger(height) && height >= 40 && height <= 800 ? null : 'The row height is a whole number from 40 to 800.',
  };
  const ready = STEPS.every((s) => !problem[s.id]);
  // a new channel goes step by step; an existing one can be changed in any step
  const reachable = (k: number) => editing !== undefined || STEPS.slice(0, k).every((s) => !problem[s.id]);
  const step = STEPS[at]!;
  const last = at === STEPS.length - 1;

  const submit = async () => {
    if (!source || !scaling || !ready) return;
    const common = {
      scaling,
      style: f.style,
      height,
      ...(f.name.trim() ? { name: f.name.trim() } : {}),
      ...(f.unit.trim() ? { unit: f.unit.trim() } : {}),
    };
    const done = editing
      ? await run('analog.update', { capture: capture.id, analog: editing.id, source: source.id, roles: f.roles, settings: f.settings, replaceSettings: true, ...common })
      : await run('analog.add', { capture: capture.id, source: source.id, roles: f.roles, settings: f.settings, ...common });
    if (done) onClose();
  };
  const forward = () => {
    if (problem[step.id]) return;
    if (last) void submit();
    else setAt(at + 1);
  };

  const number = (label: string, key: 'low' | 'high' | 'gain' | 'offset', hint: string) => (
    <TextField label={label} type="number" step="any" value={f[key]} hint={hint} onChange={(e) => set({ [key]: e.target.value })} />
  );
  const select = (id: string, label: string, value: string, onChange: (v: string) => void, options: [string, string][], hint?: ReactNode) => (
    <div className="lg-field">
      <label className="lg-field__label" htmlFor={`${uid}-${id}`}>
        {label}
      </label>
      <select
        id={`${uid}-${id}`}
        className="lg-input"
        {...(hint ? { 'aria-describedby': `${uid}-${id}-hint` } : {})}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
      {hint && (
        <span id={`${uid}-${id}-hint`} className="lg-field__hint">
          {hint}
        </span>
      )}
    </div>
  );
  const channelOptions: [string, string][] = capture.channels.map((c) => [String(c.index), `CH${c.index}${c.name !== `CH${c.index}` ? ` ${c.name}` : ''}`]);

  const stepBar = (
    <nav className="steps" aria-label="Steps">
      <ol className="steps__list">
        {STEPS.map((s, k) => (
          <li key={s.id} className="steps__item" data-state={k === at ? 'current' : k < at ? 'done' : 'todo'}>
            <button
              type="button"
              className="steps__button"
              aria-current={k === at ? 'step' : undefined}
              disabled={k !== at && !reachable(k)}
              onClick={() => setAt(k)}
            >
              <span className="steps__mark" aria-hidden="true">
                {k < at && !problem[s.id] ? <Check /> : k + 1}
              </span>
              <span className="steps__label">{s.label}</span>
            </button>
          </li>
        ))}
      </ol>
    </nav>
  );
  const footer = (
    <>
      <span className="muted">{problem[step.id] ?? ''}</span>
      {at > 0 && (
        <Button variant="secondary" onClick={() => setAt(at - 1)}>
          Back
        </Button>
      )}
      {editing && !last && (
        <Button variant="secondary" disabled={!ready} onClick={() => void submit()}>
          Apply
        </Button>
      )}
      <Button variant="primary" icon={last && !editing ? <Plus /> : undefined} disabled={last ? !ready : !!problem[step.id]} onClick={forward}>
        {last ? (editing ? 'Apply' : 'Add channel') : 'Next'}
      </Button>
    </>
  );

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title={editing ? `${editing.config.name} settings` : 'Add a virtual analog channel'}
      description={step.description}
      header={stepBar}
      footer={footer}
      wide
    >
      <form
        className="stack-sm"
        onSubmit={(e) => {
          e.preventDefault();
          forward();
        }}
      >
        {step.id === 'source' && (
          <>
            {select(
              'source',
              'Source',
              f.source,
              (v) => set({ source: v, roles: {}, settings: {} }),
              [['', 'Choose…'], ...sources.map((s): [string, string] => [s.id, s.name])],
              source?.help,
            )}
            {source?.roles.map((r) =>
              select(
                `role-${r.key}`,
                `${r.label}${r.optional ? ' (optional)' : ''}`,
                String(f.roles[r.key] ?? ''),
                (v) => set({ roles: { ...f.roles, [r.key]: v === '' ? null : Number(v) } }),
                [['', r.optional ? 'None' : 'Choose…'], ...channelOptions],
                r.help,
              ),
            )}
          </>
        )}
        {step.id === 'settings' &&
          (source?.settings.length ? (
            source.settings.map((s) => (
              <SettingField key={s.key} spec={s} value={f.settings[s.key]} onChange={(v) => set({ settings: { ...f.settings, [s.key]: v } })} />
            ))
          ) : (
            <p className="muted">This source has no settings.</p>
          ))}
        {step.id === 'scaling' && (
          <>
            {select(
              'scaling',
              'Scaling',
              f.kind,
              (v) => set({ kind: v as Form['kind'] }),
              [
                ['range', 'Lowest and highest value'],
                ['linear', 'Gain and offset'],
              ],
              'A fraction of full scale (an ADC code, a duty cycle) spans the lowest to the highest value; a measured frequency or time is multiplied by the gain.',
            )}
            {f.kind === 'range' ? (
              <>
                {number('Lowest value', 'low', 'What the lowest code or 0 % stands for.')}
                {number('Highest value', 'high', 'What full scale or 100 % stands for.')}
              </>
            ) : (
              number('Gain', 'gain', 'Value per Hz or per second measured.')
            )}
            {number('Offset', 'offset', 'Added to every value, to correct a zero error.')}
            <TextField label="Unit" value={f.unit} placeholder="V, %, Hz…" onChange={(e) => set({ unit: e.target.value })} />
          </>
        )}
        {step.id === 'look' && (
          <>
            <TextField label="Name" value={f.name} placeholder={source?.name ?? ''} onChange={(e) => set({ name: e.target.value })} />
            {select(
              'style',
              'Draw as',
              f.style,
              (v) => set({ style: v as Form['style'] }),
              [
                ['step', 'Steps: each value holds until the next'],
                ['line', 'A line between the values'],
              ],
            )}
            <TextField
              label="Row height (pixels)"
              type="number"
              min={40}
              max={800}
              step={10}
              value={f.height}
              hint="40 to 800. You can also drag the row's bottom edge in the waveform."
              onChange={(e) => set({ height: e.target.value })}
            />
          </>
        )}
        {/* Enter moves on, as the primary button does */}
        <button type="submit" hidden aria-hidden="true" tabIndex={-1} />
      </form>
    </Dialog>
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
  const dialogEl = dialog !== null && (
    <AnalogDialog key={dialog} capture={capture} sources={sources} {...(editing ? { editing } : {})} onClose={() => setDialog(null)} />
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
