// Planning a capture from one or several devices at once (the "Capture live" dialog, doc 08): the
// rates the devices share, each device's settings and what they come to, and what is wrong with
// them. The checks say what the engine's own check (Capabilities::check in crates/core/src/device.rs)
// would, so problems show while choosing, not after pressing start. No imports: the unit tests load
// this file in plain Node.

export type Mode = 'buffer' | 'stream';

/** What a device offers (the engine's `Capabilities`). */
export interface Caps {
  channels: number[];
  channelNames: string[];
  ratesHz: number[];
  limits: { maxChannels: number; maxRateHz: number }[];
  maxSamples?: number | null;
  modes: Mode[];
  threshold?: [number, number, number] | null;
}

/** What the person chose for one device. */
export interface Choice {
  channels: number[];
  rateHz: number;
  mode: Mode;
  thresholdVolts: number | null;
  title: string;
}

/** How long every device captures. */
export interface Length {
  untilStopped: boolean;
  seconds: number;
}

/** The longest capture "until I stop it" asks for where the device sets no limit: a day. */
export const UNTIL_STOPPED_SECONDS = 86_400;

const sameRate = (a: number, b: number) => Math.abs(a - b) <= Math.max(a, b) * 1e-9;

/** The rates every device offers, ascending; empty when they share none. */
export function commonRates(all: Caps[]): number[] {
  if (all.length === 0) return [];
  const [first, ...rest] = all;
  return [...(first?.ratesHz ?? [])].filter((r) => rest.every((c) => c.ratesHz.some((x) => sameRate(x, r)))).sort((a, b) => a - b);
}

/** "CH0 TX" when the device names the channel, else "CH0". */
export function channelLabel(c: Caps, ch: number): string {
  const name = c.channelNames[c.channels.indexOf(ch)];
  return name && name !== `CH${ch}` ? `CH${ch} ${name}` : `CH${ch}`;
}

/** The fastest rate the device allows with `n` channels enabled. */
function maxRate(c: Caps, n: number): number {
  return c.limits.find((l) => n <= l.maxChannels)?.maxRateHz ?? Infinity;
}

/**
 * A first choice for a device: every channel, and for a streaming device its lowest rate (live data
 * has to keep up); a buffering device its fastest rate for those channels.
 */
export function defaultChoice(c: Caps, model: string): Choice {
  const stream = c.modes.includes('stream');
  const channels = [...c.channels];
  const allowed = c.ratesHz.filter((r) => r <= maxRate(c, channels.length));
  const rateHz = (stream ? c.ratesHz[0] : (allowed.at(-1) ?? c.ratesHz[0])) ?? 1e6;
  return { channels, rateHz, mode: stream ? 'stream' : (c.modes[0] ?? 'buffer'), thresholdVolts: null, title: titleFor(model) };
}

/** "Live: Bench" for "Virtual analyser: Bench"; other devices by their model. */
export function titleFor(model: string): string {
  return `Live: ${model.replace(/^Virtual analyser: /, '')}`;
}

/** Samples per channel the choice comes to. */
export function samplesFor(c: Caps, rateHz: number, length: Length): number {
  if (length.untilStopped) return c.maxSamples ?? Math.round(rateHz * UNTIL_STOPPED_SECONDS);
  return Math.max(1, Math.round(length.seconds * rateHz));
}

/** What stands in the way of capturing with this choice, one sentence each; empty when nothing does. */
export function problems(c: Caps, choice: Choice, length: Length, rate: (sps: number) => string): string[] {
  const out: string[] = [];
  const n = choice.channels.length;
  if (n === 0) out.push('Choose at least one channel.');
  if (choice.title.trim() === '') out.push('Give the capture a name.');
  if (!length.untilStopped && !(length.seconds > 0)) out.push('Choose a length above zero seconds.');
  if (!c.ratesHz.some((r) => sameRate(r, choice.rateHz))) {
    out.push(`${rate(choice.rateHz)} is not a rate this device offers; it offers ${c.ratesHz.map(rate).join(', ')}.`);
  } else if (n > 0 && choice.rateHz > maxRate(c, n)) {
    out.push(`${rate(choice.rateHz)} is not available with ${n} channels; the most is ${rate(maxRate(c, n))}. Choose fewer channels or a lower rate.`);
  }
  if (!c.modes.includes(choice.mode)) out.push('This device cannot capture in that mode.');
  if (c.maxSamples != null && !length.untilStopped && length.seconds > 0 && samplesFor(c, choice.rateHz, length) > c.maxSamples) {
    const most = c.maxSamples / choice.rateHz;
    out.push(`This device holds at most ${c.maxSamples.toLocaleString('en-US')} samples per channel, ${seconds(most)} at this rate. Choose a shorter length or a lower rate.`);
  }
  if (choice.thresholdVolts !== null) {
    const t = c.threshold;
    if (!t) out.push('This device has no adjustable threshold.');
    else if (!(choice.thresholdVolts >= t[0] && choice.thresholdVolts <= t[1])) out.push(`The threshold must be between ${t[0]} V and ${t[1]} V.`);
  }
  return out;
}

function seconds(s: number): string {
  if (s >= 1) return `${Number(s.toPrecision(3))} s`;
  if (s >= 1e-3) return `${Number((s * 1e3).toPrecision(3))} ms`;
  return `${Number((s * 1e6).toPrecision(3))} µs`;
}

/** The `device.configure` input for a choice. */
export function configFor(c: Caps, choice: Choice, length: Length) {
  return {
    channels: [...choice.channels].sort((a, b) => a - b),
    sampleRateHz: choice.rateHz,
    samples: samplesFor(c, choice.rateHz, length),
    mode: choice.mode,
    ...(choice.thresholdVolts !== null ? { thresholdVolts: choice.thresholdVolts } : {}),
  };
}

/**
 * For devices captured into one capture: the channel on each that most likely carries a signal
 * they share, by a name every one of them gives a channel (a clock or sync line first), or null
 * when they name none alike.
 */
export function guessShared(all: Caps[]): number[] | null {
  if (all.length < 2) return null;
  const named = (c: Caps) => c.channels.map((ch, i) => ({ ch, name: (c.channelNames[i] ?? '').trim() })).filter((x) => x.name && x.name !== `CH${x.ch}`);
  const first = named(all[0]!);
  const common = first.filter((x) => all.every((c) => named(c).some((y) => y.name === x.name)));
  const pick = common.find((x) => /sync|ref/i.test(x.name)) ?? common.find((x) => /clk|clock/i.test(x.name)) ?? common[0];
  if (!pick) return null;
  return all.map((c) => named(c).find((y) => y.name === pick.name)!.ch);
}

/** "Live: Bench + Second" for one capture from several devices. */
export function togetherTitle(models: string[]): string {
  return `Live: ${models.map((m) => m.replace(/^Virtual analyser: /, '')).join(' + ')}`;
}

/**
 * What stands in the way of capturing several devices into one capture, beyond each device's own
 * problems: they need a rate in common, and when lined up by a shared signal, each captures the
 * channel it is on.
 */
/** A problem with capturing several devices as one, and the device it is about (index), if one. */
export interface TogetherIssue {
  text: string;
  /** The rate the devices share, or the signal that lines them up. */
  topic: 'rate' | 'signal';
  device: number | null;
}

export function togetherIssues(
  devices: { model: string; caps: Caps; choice: Choice }[],
  align: number[] | null,
  label: (c: Caps, ch: number) => string,
): TogetherIssue[] {
  const out: TogetherIssue[] = [];
  if (commonRates(devices.map((d) => d.caps)).length === 0) {
    out.push({ text: 'These devices share no sample rate, so they cannot be one capture; capture them each into a tab instead.', topic: 'rate', device: null });
  }
  if (align) {
    devices.forEach((d, k) => {
      const ch = align[k];
      if (ch === undefined || !d.caps.channels.includes(ch)) out.push({ text: `Choose the channel of ${d.model} that sees the shared signal.`, topic: 'signal', device: null });
      else if (!d.choice.channels.includes(ch)) out.push({ text: `${d.model}: capture ${label(d.caps, ch)} too; the capture is lined up by it.`, topic: 'signal', device: k });
    });
  }
  return out;
}

export function togetherProblems(
  devices: { model: string; caps: Caps; choice: Choice }[],
  align: number[] | null,
  label: (c: Caps, ch: number) => string,
): string[] {
  return togetherIssues(devices, align, label).map((i) => i.text);
}

// The dialog's steps. They follow what is chosen: the devices first, then how and how long to
// capture, how to line the devices up when they go into one capture, then one step per chosen
// device (its channels, rate, mode), then a check of everything with the start. Without a device
// chosen there is only the first step.

export type StepId = 'devices' | 'capture' | 'lineup' | 'review' | `device:${string}`;

export interface Step {
  id: StepId;
  /** Short, for the step bar and the "Next" button. */
  label: string;
}

/** A device's name without what every virtual analyser's name starts with. */
export function shortModel(model: string): string {
  return model.replace(/^Virtual analyser: /, '');
}

export function captureSteps(devices: { id: string; model: string }[], options: { lineUp?: boolean } = {}): Step[] {
  if (devices.length === 0) return [{ id: 'devices', label: 'Devices' }];
  // two devices of the same model are told apart by their place in the list
  const seen = new Map<string, number>();
  for (const d of devices) seen.set(d.model, (seen.get(d.model) ?? 0) + 1);
  const nth = new Map<string, number>();
  const perDevice: Step[] = devices.map((d) => {
    const n = (nth.get(d.model) ?? 0) + 1;
    nth.set(d.model, n);
    const name = shortModel(d.model);
    return { id: `device:${d.id}`, label: (seen.get(d.model) ?? 0) > 1 ? `${name} ${n}` : name };
  });
  const lineUp: Step[] = options.lineUp && devices.length > 1 ? [{ id: 'lineup', label: 'Line up' }] : [];
  return [{ id: 'devices', label: 'Devices' }, { id: 'capture', label: 'Capture' }, ...lineUp, ...perDevice, { id: 'review', label: 'Check and start' }];
}

/** The step to show: the one asked for while it exists, else the first (a device was unpicked). */
export function currentStep(steps: Step[], wanted: StepId): number {
  const i = steps.findIndex((s) => s.id === wanted);
  return i < 0 ? 0 : i;
}
