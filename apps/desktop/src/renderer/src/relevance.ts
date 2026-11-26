// Which engine events each part of the window needs (doc 03 events). Reading again only on these
// keeps the window calm: the person's own panning sends view.set many times a second, and must not
// reset the decoded table or reload the activity feed.

export interface EngineEvent {
  event: string;
  capture?: string;
  decoder?: string;
  /** analog.updated: the virtual analog channel. */
  analog?: string;
  actor?: string;
  /** capture.progress and capture.done: samples per channel so far. */
  samples?: number;
}

/** The session summary: anything recorded, captures, leases; views only from others. */
export const sessionNeeds = (e: EngineEvent): boolean =>
  ['state.changed', 'capture.done', 'lease.changed', 'device.disconnected'].includes(e.event) ||
  (e.event === 'view.changed' && e.actor !== 'user');

/**
 * The session summary again, but not at once: a live capture reports progress and decodes about 20
 * times a second, and the summary (length, problem counts) needs a few updates a second. Changes a
 * person or agent makes also send state.changed, so nothing waits on this.
 */
export const sessionLiveNeeds = (e: EngineEvent): boolean =>
  e.event === 'capture.progress' || e.event === 'decoder.updated' || e.event === 'analog.updated';

/** A capture's waveform data: its decoders and the end of its recording. */
export const waveformNeeds = (e: EngineEvent, capture: string): boolean =>
  (e.event === 'decoder.updated' || e.event === 'capture.done') && e.capture === capture;

/** A capture's virtual analog values: any of its channels, and the end of its recording. */
export const analogNeeds = (e: EngineEvent, capture: string): boolean =>
  (e.event === 'analog.updated' || e.event === 'capture.done') && e.capture === capture;

/** A decoder's table: that decoder, or any decoder of the capture when none is chosen. */
export const tableNeeds = (e: EngineEvent, capture: string, decoder: string | null): boolean =>
  e.event === 'decoder.updated' && e.capture === capture && (!decoder || e.decoder === decoder);

/** The activity feed: every recorded command, undo and redo. */
export const activityNeeds = (e: EngineEvent): boolean => e.event === 'command.executed';
