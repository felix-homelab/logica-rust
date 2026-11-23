// The bottom panel (doc 08): the decoded table, measurements, and the activity feed with undo and
// "stop agent actions".
import { useEffect, useRef, useState } from 'react';
import type { Output } from '@logica/api';
import { Badge, Button, EmptyState, Switch, TextField, VirtualList } from '@logica/ui';
import { Undo2, Redo2 } from 'lucide-react';
import { client } from '../bridge';
import { at, hz, int, time } from '../format';
import { activityNeeds, tableNeeds } from '../relevance';
import { isAgent, useEventCount, useStore, who, type CaptureInfo } from '../store';
import type { ViewState } from './Waveform';

type Page = Output<'decoder.frames'>;
const PAGE = 200;

export function DecodedTable({ capture, onJump }: { capture: CaptureInfo; onJump: (from: number, to: number) => void }) {
  const [decoder, setDecoder] = useState(capture.decoders[0]?.id ?? '');
  const changes = useEventCount((e) => tableNeeds(e, capture.id, decoder || null));
  const [text, setText] = useState('');
  const [problems, setProblems] = useState(false);
  const [total, setTotal] = useState(0);
  const [pages, setPages] = useState<Map<number, Page>>(new Map());
  const loading = useRef(new Set<number>());
  const [selected, setSelected] = useState<number | null>(null);
  useEffect(() => {
    if (!capture.decoders.some((d) => d.id === decoder)) setDecoder(capture.decoders[0]?.id ?? '');
  }, [capture.decoders, decoder]);
  const load = (page: number, reset = false) => {
    if (!decoder || (!reset && loading.current.has(page))) return;
    loading.current.add(page);
    void client
        .run('decoder.frames', {
          capture: capture.id,
          decoder,
          packets: true,
          problems,
          cursor: page * PAGE,
          limit: PAGE,
          ...(text ? { text } : {}),
        })
        .then((p) => {
          setTotal(p.total);
          setPages((m) => new Map(reset ? [[page, p]] : [...m, [page, p]]));
        })
        .catch(() => {})
        .finally(() => loading.current.delete(page));
  };
  useEffect(() => {
    loading.current.clear();
    setPages(new Map());
    load(0, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [capture.id, decoder, text, problems, changes]);
  if (capture.decoders.length === 0) {
    return <EmptyState title="Nothing decoded yet">Add a decoder from the analysis to see packets here.</EmptyState>;
  }
  const row = (i: number) => {
    const p = pages.get(Math.floor(i / PAGE));
    if (!p) {
      load(Math.floor(i / PAGE));
      return <div className="table__row muted">…</div>;
    }
    const item = p.packets[i % PAGE];
    if (!item) return null;
    const pk = item.packet;
    return (
      <button
        type="button"
        className="table__row"
        aria-current={selected === i || undefined}
        onClick={() => {
          setSelected(i);
          onJump(pk.start, pk.end);
        }}
      >
        <span className="table__index lg-num">{pk.index}</span>
        <span className="table__time lg-num">{at(pk.start, capture.sampleRateHz)}</span>
        <span className="table__text">{item.text}</span>
      </button>
    );
  };
  return (
    <div className="table">
      <div className="table__filters row">
        {capture.decoders.length > 1 && (
          <select className="lg-input" aria-label="Decoder" value={decoder} onChange={(e) => setDecoder(e.target.value)}>
            {capture.decoders.map((d) => (
              <option key={d.id} value={d.id}>
                {d.config.name}
              </option>
            ))}
          </select>
        )}
        <TextField label="Filter" hideLabel placeholder="Filter by text" value={text} onChange={(e) => setText(e.target.value)} />
        <Switch label="Only problems" checked={problems} onCheckedChange={setProblems} />
        <span className="muted lg-num" role="status">
          {int(total)} packets
        </span>
      </div>
      <div className="table__body">
        <VirtualList count={total} rowHeight={26} render={row} label="Decoded packets" />
      </div>
    </div>
  );
}

export function Measurements({ capture, view }: { capture: CaptureInfo; view: ViewState | undefined }) {
  const { run } = useStore();
  const [channel, setChannel] = useState(capture.channels[0]?.index ?? 0);
  const [stats, setStats] = useState<Output<'measure.stats'> | null>(null);
  const rate = capture.sampleRateHz;
  const range = view?.selection ?? (view ? [view.from, view.to] : null);
  useEffect(() => {
    if (!range) return;
    const t = window.setTimeout(() => {
      void run('measure.stats', { capture: capture.id, channel, from: range[0], to: range[1] }).then((s) => s && setStats(s));
    }, 150);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [capture.id, channel, range?.[0], range?.[1]]);
  const a = view?.cursorA ?? null;
  const b = view?.cursorB ?? null;
  return (
    <div className="pad measure">
      <dl className="facts">
        <dt>Cursor A</dt>
        <dd className="lg-num">{a === null ? 'Click the waveform' : at(a, rate)}</dd>
        <dt>Cursor B</dt>
        <dd className="lg-num">{b === null ? 'Alt+click the waveform' : at(b, rate)}</dd>
        {a !== null && b !== null && (
          <>
            <dt>B − A</dt>
            <dd className="lg-num">
              {time((b - a) / rate)} ({int(Math.abs(b - a))} samples
              {a !== b ? `, ${hz(rate / Math.abs(b - a))}` : ''})
            </dd>
          </>
        )}
        {view?.selection && (
          <>
            <dt>Selection</dt>
            <dd className="lg-num">{time((view.selection[1] - view.selection[0]) / rate)}</dd>
          </>
        )}
      </dl>
      <div className="stack-sm">
        <label className="lg-field">
          <span className="lg-field__label">Channel ({view?.selection ? 'in the selection' : 'in the view'})</span>
          <select className="lg-input" value={channel} onChange={(e) => setChannel(Number(e.target.value))}>
            {capture.channels.map((c) => (
              <option key={c.index} value={c.index}>
                CH{c.index} {c.name !== `CH${c.index}` ? c.name : ''}
              </option>
            ))}
          </select>
        </label>
        {stats && (
          <dl className="facts">
            <dt>Edges</dt>
            <dd className="lg-num">{int(stats.edges)}</dd>
            <dt>Frequency</dt>
            <dd className="lg-num">{stats.frequencyHz ? hz(stats.frequencyHz) : '–'}</dd>
            <dt>Period</dt>
            <dd className="lg-num">{stats.periodSeconds ? time(stats.periodSeconds) : '–'}</dd>
            <dt>Duty cycle</dt>
            <dd className="lg-num">{stats.dutyCycle !== null && stats.dutyCycle !== undefined ? `${(stats.dutyCycle * 100).toFixed(1)} %` : '–'}</dd>
            <dt>Narrowest pulse</dt>
            <dd className="lg-num">{stats.narrowestPulseSeconds ? time(stats.narrowestPulseSeconds) : '–'}</dd>
            {stats.note && (
              <>
                <dt>Note</dt>
                <dd>{stats.note}</dd>
              </>
            )}
          </dl>
        )}
      </div>
    </div>
  );
}

type Entry = Output<'history.list'>['entries'][number];

export function ActivityFeed() {
  const { session, run } = useStore();
  const changes = useEventCount(activityNeeds, 150);
  const [entries, setEntries] = useState<Entry[]>([]);
  useEffect(() => {
    void run('history.list', { limit: 300 }).then((h) => h && setEntries(h.entries));
  }, [run, changes]);
  if (!session) return null;
  return (
    <div className="activity">
      <div className="activity__bar row between">
        <Switch
          label="Stop agent actions"
          description="Agents cannot capture or drive outputs while this is on."
          checked={session.agentsPaused}
          onCheckedChange={(paused) => void run('session.pauseAgents', { paused })}
        />
      </div>
      {entries.length === 0 ? (
        <EmptyState title="Nothing has happened yet">Every command appears here with who ran it, and what can be undone has an undo button.</EmptyState>
      ) : (
        <ol className="activity__list" aria-label="Activity">
          {entries.map((e) => (
            <li key={e.id} className="activity__item" data-agent={isAgent(e.actor) || undefined} data-undone={e.undone || undefined}>
              <Badge tone={isAgent(e.actor) ? 'agent' : 'neutral'}>{who(e.actor)}</Badge>
              <span className="activity__text">
                {e.summary}
                {e.undone && <span className="muted"> (undone)</span>}
              </span>
              {e.inverse && !e.undone && !e.discarded && (
                <Button size="sm" variant="ghost" icon={<Undo2 />} onClick={() => void run('history.undo', { entry: e.id })}>
                  Undo
                </Button>
              )}
              {e.undone && !e.discarded && (
                <Button size="sm" variant="ghost" icon={<Redo2 />} onClick={() => void run('history.redo', { entry: e.id })}>
                  Redo
                </Button>
              )}
              {!e.inverse && e.neverBecause && (
                <span className="muted activity__never" title={e.neverBecause}>
                  cannot be undone
                </span>
              )}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
