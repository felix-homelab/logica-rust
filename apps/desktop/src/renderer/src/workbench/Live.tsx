// The live view while capturing (doc 08): the waveform follows the newest data unless the person
// scrolls away, and scrolling back to the end follows again. The length comes from the engine's
// progress events (about 20 a second), so following is smooth without reading the whole session;
// only this part of the window updates that often.
import { useEffect, useMemo, useRef, useState } from 'react';
import { Badge, Button } from '@logica/ui';
import { Pause, Play, Square } from 'lucide-react';
import { time } from '../format';
import { useStore, type CaptureInfo } from '../store';
import { LIVE_SECONDS, MIN_SPAN, following, liveView } from '../viewmath';
import { Waveform, type ViewState } from './Waveform';

const same = (a: ViewState, b: ViewState) =>
  a.from === b.from && a.to === b.to && a.cursorA === b.cursorA && a.cursorB === b.cursorB && JSON.stringify(a.selection) === JSON.stringify(b.selection);

export function LiveWaveform({
  capture,
  view,
  onView,
  expert,
}: {
  capture: CaptureInfo;
  view: ViewState;
  onView: (v: ViewState) => void;
  expert: boolean;
}) {
  const { subscribe, session, run } = useStore();
  const rate = capture.sampleRateHz;
  const [count, setCount] = useState(capture.sampleCount);
  // the width followed, or null while the person looks elsewhere
  const [span, setSpan] = useState<number | null>(() => Math.max(MIN_SPAN, Math.round(rate * LIVE_SECONDS)));
  // the last view told to the window; the engine's view at the start is not a change by the person
  const reported = useRef<ViewState | null>(view);

  useEffect(
    () =>
      subscribe((e) => {
        if (e.event === 'capture.progress' && e.capture === capture.id && typeof e.samples === 'number') {
          const n = e.samples;
          setCount((c) => Math.max(c, n));
        }
      }),
    [subscribe, capture.id],
  );

  const known = Math.max(count, capture.sampleCount);
  const shown: ViewState = span === null ? view : { ...view, ...following(span, known) };
  const live = useMemo(() => ({ ...capture, sampleCount: known }), [capture, known]);
  const shownRef = useRef(shown);
  shownRef.current = shown;

  /** The person (or the toolbar) changed the view. */
  const change = (next: ViewState) => {
    const r = liveView(shownRef.current, next, known, span);
    setSpan(r.span);
    const v = { ...next, ...r.range };
    reported.current = v;
    onView(v);
  };

  // the toolbar's zoom and "whole capture" change the view from outside
  useEffect(() => {
    if (reported.current && same(view, reported.current)) return;
    reported.current = view;
    change(view);
    // only a new view from outside matters here, not every new render of `change`
  }, [view.from, view.to]);

  // while following, tell the rest of the window (status bar, agents) once a second
  useEffect(() => {
    if (span === null) return;
    const t = window.setInterval(() => {
      reported.current = shownRef.current;
      onView(shownRef.current);
    }, 1000);
    return () => window.clearInterval(t);
  }, [span, onView]);

  // when the capture ends (this view goes), the window keeps exactly what was shown last
  const onViewRef = useRef(onView);
  onViewRef.current = onView;
  useEffect(() => () => onViewRef.current(shownRef.current), []);

  const device = session?.running.find((r) => r.capture === capture.id)?.device;
  return (
    <div className="live">
      <div className="live__bar" role="group" aria-label="Live capture">
        <Badge tone="danger">
          <span className="live__dot" aria-hidden="true" /> Live
        </Badge>
        <span className="lg-num">{time(known / rate)} captured</span>
        <span className="muted">{span === null ? 'You are looking back; new data keeps arriving.' : 'Following the newest data.'}</span>
        <span className="live__grow" />
        {span === null ? (
          <Button size="sm" variant="secondary" icon={<Play />} onClick={() => change({ ...shown, ...following(shown.to - shown.from, known) })}>
            Follow
          </Button>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            icon={<Pause />}
            onClick={() => {
              setSpan(null);
              reported.current = shown;
              onView(shown);
            }}
          >
            Stay here
          </Button>
        )}
        {device && (
          <Button size="sm" variant="secondary" icon={<Square />} onClick={() => void run('capture.stop', { device })}>
            Stop capture
          </Button>
        )}
      </div>
      <Waveform capture={live} view={shown} onView={change} expert={expert} follow={span !== null} />
    </div>
  );
}
