// Which channels belong together, and the rows the waveform draws (doc 08). A decoder's channels
// sit side by side under its name; its data is drawn under the channel it is on (SPI: MOSI and
// MISO; UART: RX and TX) in two rows, the values as the decoder labels them (hex) and below them
// the same data in the format chosen for the decoder (ASCII, hex, decimal or binary). Channels no
// decoder uses come last. No window code here, so it is tested on its own.

export type DataFormat = 'ascii' | 'hex' | 'decimal' | 'binary';

export interface LaneChannel {
  index: number;
  name: string;
  hidden: boolean;
  colour?: string | null;
}

export interface LaneDecoder {
  id: string;
  config: { name: string; roles: Record<string, number | null | undefined>; dataFormat?: DataFormat | undefined };
  /** Channels its data frames are drawn under. */
  lanes: number[];
}

export interface Group<C, D> {
  /** The decoder the group is named after. */
  decoder: D;
  /** Its channels, in the capture's order (hidden ones too). */
  channels: C[];
  /** Decoders on channels of this group only (a second decoder on the same wires). */
  extra: D[];
}

export interface Grouping<C, D> {
  groups: Group<C, D>[];
  /** Channels no decoder uses. */
  other: C[];
  /** Decoders on none of the capture's channels. */
  loose: D[];
}

const used = (d: LaneDecoder): number[] => Object.values(d.config.roles).filter((x): x is number => typeof x === 'number');

/** The role a decoder gives a channel, as written on screen: "SDA", "MOSI". */
export function roleOf(d: LaneDecoder, channel: number): string | null {
  const e = Object.entries(d.config.roles).find(([, ch]) => ch === channel);
  return e ? e[0].toUpperCase() : null;
}

/**
 * Groups channels by the decoders that use them. A channel belongs to the first decoder that uses
 * it; groups are in the order of their first channel, so the capture's channel order is kept.
 */
export function groupChannels<C extends LaneChannel, D extends LaneDecoder>(channels: C[], decoders: D[]): Grouping<C, D> {
  const owner = new Map<number, Group<C, D>>();
  const groups: Group<C, D>[] = [];
  const loose: D[] = [];
  for (const d of decoders) {
    const wires = new Set(used(d));
    const own = channels.filter((c) => wires.has(c.index) && !owner.has(c.index));
    if (own.length) {
      const g: Group<C, D> = { decoder: d, channels: own, extra: [] };
      groups.push(g);
      for (const c of own) owner.set(c.index, g);
      continue;
    }
    const host = [...wires].map((w) => owner.get(w)).find((g) => g !== undefined);
    if (host) host.extra.push(d);
    else loose.push(d);
  }
  const position = new Map(channels.map((c, i) => [c.index, i]));
  groups.sort((a, b) => position.get(a.channels[0]!.index)! - position.get(b.channels[0]!.index)!);
  return { groups, other: channels.filter((c) => !owner.has(c.index)), loose };
}

export type RowKind = 'group' | 'channel' | 'data' | 'text';

export interface Row {
  kind: RowKind;
  key: string;
  label: string;
  y: number;
  h: number;
  /** Channel rows: the physical channel. */
  channel?: number;
  /** Channel rows: which of the four trace colours, by position among the visible channels. */
  colourIndex?: number;
  colour?: string | null;
  /** Data and text rows: the decoder. */
  decoder?: string;
  /** Data and text rows: the channel whose frames they draw; null for all of them. */
  lane?: number | null;
  /** Text rows: how the data is written. */
  format?: DataFormat;
  /** Which group the row belongs to (-1 for none), to draw the group's bracket. */
  group: number;
}

export const ROW_HEIGHT: Record<RowKind, number> = { group: 18, channel: 34, data: 22, text: 18 };

export const FORMAT_LABEL: Record<DataFormat, string> = { ascii: 'ASCII', hex: 'Hex', decimal: 'Decimal', binary: 'Binary' };

const isDefaultName = (c: LaneChannel) => !c.name || c.name === `CH${c.index}` || c.name === `channel ${c.index}`;

/** The rows of the waveform, from `top` down. */
export function layoutRows<C extends LaneChannel, D extends LaneDecoder>(channels: C[], decoders: D[], top: number): Row[] {
  const { groups, other } = groupChannels(channels, decoders);
  const visible = channels.filter((c) => !c.hidden);
  const colourIndex = new Map(visible.map((c, i) => [c.index, i]));
  const shown = new Set(visible.map((c) => c.index));
  // each decoder's rows go under the visible channels its data is on; with none visible, at the
  // end of its group (or of everything)
  const under = new Map<number, D[]>();
  const atEnd = new Map<Group<C, D> | null, D[]>();
  const groupOf = (d: D) => groups.find((g) => g.decoder === d || g.extra.includes(d)) ?? null;
  for (const d of decoders) {
    const lanes = d.lanes.filter((l) => shown.has(l));
    if (lanes.length) for (const l of lanes) under.set(l, [...(under.get(l) ?? []), d]);
    else atEnd.set(groupOf(d), [...(atEnd.get(groupOf(d)) ?? []), d]);
  }

  const rows: Row[] = [];
  let y = top;
  const push = (r: Omit<Row, 'y' | 'h'>) => {
    rows.push({ ...r, y, h: ROW_HEIGHT[r.kind] });
    y += ROW_HEIGHT[r.kind];
  };
  const decoderRows = (d: D, lane: number | null, owner: D | null, group: number) => {
    const role = lane === null ? null : roleOf(d, lane);
    // under its own group's channel the role says enough; elsewhere the decoder's name
    const label = d === owner && role ? role : d.config.name;
    push({ kind: 'data', key: `d${d.id}:${lane ?? '*'}`, label, decoder: d.id, lane, group });
    if (d.lanes.length) {
      const format = d.config.dataFormat ?? 'ascii';
      push({ kind: 'text', key: `t${d.id}:${lane ?? '*'}`, label: FORMAT_LABEL[format], decoder: d.id, lane, format, group });
    }
  };
  const channelRow = (c: C, role: string | null, owner: D | null, group: number) => {
    const label = !isDefaultName(c) ? `CH${c.index} ${c.name}` : role ? `CH${c.index} ${role}` : `CH${c.index}`;
    push({ kind: 'channel', key: `c${c.index}`, label, channel: c.index, colourIndex: colourIndex.get(c.index) ?? 0, colour: c.colour ?? null, group });
    for (const d of under.get(c.index) ?? []) decoderRows(d, c.index, owner, group);
  };

  groups.forEach((g, gi) => {
    const names = [g.decoder, ...g.extra].map((d) => d.config.name).join(' + ');
    push({ kind: 'group', key: `g${g.decoder.id}`, label: names, decoder: g.decoder.id, group: gi });
    for (const c of g.channels.filter((c) => !c.hidden)) channelRow(c, roleOf(g.decoder, c.index), g.decoder, gi);
    for (const d of atEnd.get(g) ?? []) decoderRows(d, null, null, gi);
  });
  for (const c of other.filter((c) => !c.hidden)) channelRow(c, null, null, -1);
  // decoders on no channel of this capture (`loose`) have no channel to sit under either
  for (const d of atEnd.get(null) ?? []) decoderRows(d, null, null, -1);
  return rows;
}

/** Data frames next to each other in one packet on one channel: one string in the text row. */
export function runs<F extends { start: number; end: number; packet: number; lane: number; text?: string | null }>(frames: F[]): F[][] {
  const out: F[][] = [];
  const sorted = frames.filter((f) => f.text !== null && f.text !== undefined).sort((a, b) => a.lane - b.lane || a.start - b.start);
  for (const f of sorted) {
    const last = out.at(-1);
    const prev = last?.at(-1);
    if (last && prev && prev.packet === f.packet && prev.lane === f.lane) last.push(f);
    else out.push([f]);
  }
  return out;
}

/** The values of a run as one string: ASCII runs together, the other formats with spaces. */
export function joinRun(texts: string[], format: DataFormat): string {
  return format === 'ascii' ? texts.join('') : texts.join(' ');
}
