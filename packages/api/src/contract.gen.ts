// Generated from schema/contract.json by scripts/generate.mjs. Do not edit by hand.
// Command API version 1.0.

/**
 * A formula over virtual analog channels, in their units.
 */
export type Expr = ({
channel: string
} | {
constant: number
} | {
add: Expr[]
} | {
sub: Expr[]
} | {
mul: Expr[]
} | {
div: Expr[]
} | {
min: Expr[]
} | {
max: Expr[]
} | {
abs: Expr
} | {
neg: Expr
})
/**
 * A setting's value: a number, a choice (text) or a switch.
 */
export type SettingValue = (boolean | number | string)
/**
 * How a channel is drawn: the value held until the next point, or a line between points.
 */
export type AnalogStyle = ("step" | "line")
/**
 * Input of commands that take nothing.
 */
export type Nothing = Record<string, never>
/**
 * The linear step from a source's points to values in the channel's unit.
 */
export type Scaling = ({
high: number
kind: "range"
low: number
offset?: number
} | {
gain: number
kind: "linear"
offset?: number
})
/**
 * How a decoder's data values are written out for people (the waveform's text row, doc 08).
 */
export type DataFormat = ("ascii" | "hex" | "decimal" | "binary")
export type FrameKind = ("data" | "address" | "start" | "repeated-start" | "stop" | "ack" | "nack" | "break" | "reset" | "presence" | "field" | "command" | "sample" | "cycle" | "info")
export type CaptureMode = ("buffer" | "stream")
/**
 * Who sent a command: user, agent:<client name> or cli.
 */
export type Actor = string
/**
 * A search (doc 06): a typed structure, not a text grammar. `decoded`, `edge` and `pulse` produce
 * hits; `within`, `level`, `not` and further producers filter them inside `all`.
 */
export type Query = ({
all: Query[]
} | {
any: Query[]
} | {
not: Query
} | {
decoded: DecodedQuery
} | {
edge: EdgeQuery
} | {
pulse: PulseQuery
} | {
level: LevelQuery
} | {
within: WithinQuery
} | {
analog: AnalogQuery
})
export type ProblemCode = ("frame-error" | "parity-error" | "incomplete" | "glitch" | "checksum" | "stuffing" | "timing" | "protocol")
export type Direction = ("rising" | "falling" | "any")
/**
 * A digital level.
 */
export type Level = ("low" | "high")
export type Unit = ("s" | "ms" | "us" | "ns" | "samples")
/**
 * How a raw ADC code stands for a fraction of full scale.
 */
export type Encoding = ("unsigned" | "twosComplement")
/**
 * What a source's points mean, so the scaling that fits them can be checked.
 */
export type PointKind = ("fraction" | "quantity")
export type SettingKind = ({
default: number
kind: "number"
max?: (number | null)
min?: (number | null)
presets: number[]
unit?: (string | null)
} | {
default: string
kind: "choice"
options: [string, string][]
} | {
default: boolean
kind: "boolean"
})
export type FindingId = ("text" | "transformed-text" | "modbus" | "checksum-sum" | "checksum-xor" | "bcd" | "all-ff" | "all00" | "none")
export type Tone = ("info" | "warn")
export type NoteLevel = ("info" | "warning")
/**
 * Where a capture came from.
 */
export type SourceKind = ("device" | "file" | "synthetic")
export type CaptureState = ("capturing" | "complete" | "aborted")
export type Origin = ("built-in" | "custom")
/**
 * Sent to every attached client (doc 03).
 */
export type Event = ({
capture: string
event: "capture.progress"
fraction?: (number | null)
samples: number
} | {
capture: string
event: "capture.done"
samples: number
stopped: boolean
} | {
capture: string
decoder: string
event: "decoder.updated"
} | {
analog: string
capture: string
event: "analog.updated"
} | {
actor: Actor
capture: string
event: "view.changed"
} | {
device: string
event: "lease.changed"
holder?: (Actor | null)
} | {
device: string
event: "device.disconnected"
} | {
actor: Actor
command: string
entry?: (number | null)
event: "command.executed"
summary: string
} | {
event: "state.changed"
revision: number
})
export type ErrorCode = ("cancelled" | "invalid-input" | "not-found" | "busy" | "not-allowed" | "not-undoable" | "failed")

export interface Contract {
inputs: {
"analog.add": AnalogAddIn
"analog.at": AnalogAtIn
"analog.export": AnalogExportIn
"analog.remove": AnalogRef
"analog.sources": Nothing
"analog.stats": AnalogStatsIn
"analog.update": AnalogUpdateIn
"analog.values": AnalogValuesIn
"analysis.bytes": BytesIn
"analysis.detect": RangeIn
"capture.close": CaptureRef
"capture.rename": CaptureRenameIn
"capture.start": CaptureStartIn
"capture.stop": DeviceRef
"channel.colour": ChannelColourIn
"channel.group": ChannelGroupIn
"channel.hide": ChannelHideIn
"channel.rename": ChannelRenameIn
"decoder.add": DecoderAddIn
"decoder.frames": DecoderFramesIn
"decoder.remove": DecoderRef
"decoder.update": DecoderUpdateIn
"device.configure": DeviceConfigureIn
"device.connect": DeviceRef
"device.disconnect": DeviceRef
"device.list": Nothing
"file.open": FileOpenIn
"file.save": FileSaveIn
"history.list": HistoryListIn
"history.redo": HistoryPick
"history.undo": HistoryPick
"lease.release": DeviceRef
"lease.take": DeviceRef
"marker.add": MarkerAddIn
"marker.remove": MarkerRef
"marker.update": MarkerUpdateIn
"measure.between": MeasureBetweenIn
"measure.edges": MeasureEdgesIn
"measure.stats": MeasureStatsIn
"protocol.addCustom": AddCustomIn
"protocol.info": ProtocolRef
"protocol.list": Nothing
"protocol.removeCustom": ProtocolRef
render: RenderIn
"search.run": SearchIn
"session.get": Nothing
"session.pauseAgents": PauseAgentsIn
"view.set": ViewSetIn
"waveform.get": WaveformIn
}
outputs: {
"analog.add": AnalogOut
"analog.at": AnalogAtOut
"analog.export": AnalogExportOut
"analog.remove": Nothing
"analog.sources": AnalogSourcesOut
"analog.stats": AnalogStatsOut
"analog.update": AnalogOut
"analog.values": AnalogValuesOut
"analysis.bytes": ByteAnalysis
"analysis.detect": CaptureAnalysis
"capture.close": Nothing
"capture.rename": Nothing
"capture.start": CaptureStartOut
"capture.stop": Nothing
"channel.colour": Nothing
"channel.group": Nothing
"channel.hide": Nothing
"channel.rename": Nothing
"decoder.add": DecoderOut
"decoder.frames": DecoderFramesOut
"decoder.remove": Nothing
"decoder.update": DecoderUpdateOut
"device.configure": Nothing
"device.connect": DeviceEntry
"device.disconnect": Nothing
"device.list": DeviceListOut
"file.open": CaptureOut
"file.save": FileSaveOut
"history.list": HistoryListOut
"history.redo": HistoryOut
"history.undo": HistoryOut
"lease.release": LeaseOut
"lease.take": LeaseOut
"marker.add": MarkerOut
"marker.remove": Nothing
"marker.update": MarkerOut
"measure.between": MeasureBetweenOut
"measure.edges": MeasureEdgesOut
"measure.stats": MeasureStatsOut
"protocol.addCustom": ProtocolEntry
"protocol.info": ProtocolInfoOut
"protocol.list": ProtocolListOut
"protocol.removeCustom": Nothing
render: RenderOut
"search.run": SearchOut
"session.get": SessionOut
"session.pauseAgents": PauseAgentsOut
"view.set": ViewOut
"waveform.get": WaveformOut
}
event: Event
error: CommandError
invocation: Invocation
historyEntry: HistoryEntry
}
export interface AnalogAddIn {
capture: string
/**
 * A decoder of the capture to read from (SPI and I2C for the ADC sources, PWM for the PWM
 * level): its channels and bus settings are used instead of `roles`, and follow it when it
 * changes.
 */
decoder?: (string | null)
/**
 * For the source `math`: the formula over other virtual analog channels, such as
 * `{ "sub": [ { "channel": "analog-3" }, { "channel": "analog-4" } ] }`.
 */
expression?: (Expr | null)
/**
 * Row height in pixels, 40 to 800 (160 unless given).
 */
height?: (number | null)
/**
 * Chosen by the engine; redo and undo use the id it had.
 */
id?: (string | null)
name?: (string | null)
roles: {
[k: string]: number | null
}
/**
 * For a fraction (an ADC code, a PWM duty): `{ "kind": "range", "low", "high", "offset" }`;
 * for a measured quantity (Hz, s): `{ "kind": "linear", "gain", "offset" }`.
 */
scaling: ({
high: number
kind: "range"
low: number
offset?: number
} | {
gain: number
kind: "linear"
offset?: number
})
settings?: {
[k: string]: SettingValue
}
/**
 * The source's id (`analog.sources`).
 */
source: string
style?: (AnalogStyle | null)
unit?: (string | null)
}
export interface AnalogAtIn {
/**
 * Sample indices to read every virtual analog channel at (1 to 16), such as the cursors.
 */
at: number[]
capture: string
}
export interface AnalogExportIn {
/**
 * The channels, in this order; all of the capture's when absent.
 */
analogs?: (string[] | null)
capture: string
/**
 * The range; the whole capture when absent.
 */
from?: (number | null)
/**
 * Where to write the CSV file, on the machine running the engine.
 */
path: string
/**
 * Exclusive.
 */
to?: (number | null)
}
export interface AnalogRef {
analog: string
capture: string
}
export interface AnalogStatsIn {
analog: string
capture: string
/**
 * The range; the whole capture when absent.
 */
from?: (number | null)
/**
 * Exclusive.
 */
to?: (number | null)
}
export interface AnalogUpdateIn {
analog: string
capture: string
/**
 * A math channel's new formula.
 */
expression?: (Expr | null)
height?: (number | null)
name?: (string | null)
replaceSettings?: boolean
roles?: ({
[k: string]: number | null
} | null)
scaling?: (Scaling | null)
/**
 * Merged into the current settings, unless `replaceSettings` is set.
 */
settings?: ({
[k: string]: SettingValue
} | null)
source?: (string | null)
style?: (AnalogStyle | null)
unit?: (string | null)
}
export interface AnalogValuesIn {
analog: string
/**
 * How many buckets the range is drawn in (1 to 8192): points when there are at most two per
 * bucket, else one envelope per bucket.
 */
buckets: number
capture: string
from: number
/**
 * Exclusive.
 */
to: number
}
export interface BytesIn {
bytes?: (number[] | null)
/**
 * Hex as typed by a person: "48 65 6C", "0x48,0x65".
 */
hex?: (string | null)
}
export interface RangeIn {
capture: string
/**
 * First sample; after the start-up artefact when absent.
 */
from?: (number | null)
/**
 * End sample (exclusive); the end of the detection window when absent.
 */
to?: (number | null)
}
export interface CaptureRef {
capture: string
}
export interface CaptureRenameIn {
capture: string
title: string
}
export interface CaptureStartIn {
/**
 * Lines the devices up by a signal they all see (a shared clock or any shared line): for
 * `device` and every device in `with`, the channel that carries it, by device id. The offset
 * is found from its rising edges (between equally good offsets, as with a steady clock, the
 * smallest) and each clock's drift is followed while capturing.
 */
align?: ({
[k: string]: number
} | null)
device: string
/**
 * The tab title; "Capture N" when absent.
 */
title?: (string | null)
/**
 * Wait until the capture ends (default). Without waiting, the capture runs on and
 * `capture.done` follows.
 */
wait?: (boolean | null)
/**
 * More devices captured at the same time into the same capture, after `device`'s channels
 * (numbered on, and named by device). Each is configured first, all at the same sample rate.
 * Devices do not share a clock: they are lined up at their starts, or by `align`.
 */
with?: (string[] | null)
}
export interface DeviceRef {
/**
 * A device id from device.list, such as "vla/127.0.0.1:7171".
 */
device: string
}
export interface ChannelColourIn {
capture: string
channel: number
/**
 * "#rrggbb", or none for the default.
 */
colour?: (string | null)
}
export interface ChannelGroupIn {
/**
 * Replaces every group at once (what an undo uses).
 */
all?: ({
[k: string]: number[]
} | null)
capture: string
/**
 * Its channels; none or empty removes the group. A channel leaves any other group.
 */
channels?: (number[] | null)
/**
 * The bus group to set.
 */
group?: (string | null)
}
export interface ChannelHideIn {
capture: string
channel: number
hidden: boolean
}
export interface ChannelRenameIn {
capture: string
/**
 * Physical channel number.
 */
channel: number
name: string
}
export interface DecoderAddIn {
capture: string
/**
 * How its data is written in the waveform's text row (default ASCII).
 */
dataFormat?: (DataFormat | null)
/**
 * The decoder id to use (redo and undo use the id it had); a new one when absent.
 */
id?: (string | null)
name?: (string | null)
/**
 * A protocol id from protocol.list, such as "i2c".
 */
protocol: string
/**
 * Role to physical channel, such as {"scl": 5, "sda": 4}.
 */
roles: {
[k: string]: number | null
}
settings?: {
[k: string]: SettingValue
}
}
export interface DecoderFramesIn {
capture: string
/**
 * From the previous page's `nextCursor`.
 */
cursor?: (number | null)
decoder: string
/**
 * Group the problems and explain likely causes.
 */
explain?: boolean
from?: (number | null)
/**
 * Only frames of this kind.
 */
kind?: (FrameKind | null)
/**
 * Items per page (default 100, at most 1,000).
 */
limit?: (number | null)
/**
 * Packets (a UART line, an I2C transaction) instead of single frames.
 */
packets?: boolean
/**
 * Only frames (or packets with frames) that have problems.
 */
problems?: boolean
/**
 * Only items whose label, detail, title or text contains this (any case).
 */
text?: (string | null)
to?: (number | null)
}
export interface DecoderRef {
capture: string
decoder: string
}
export interface DecoderUpdateIn {
capture: string
/**
 * How its data is written in the waveform's text row; changing only this does not decode
 * again.
 */
dataFormat?: (DataFormat | null)
decoder: string
name?: (string | null)
/**
 * Replace all settings instead of merging (what an undo uses).
 */
replaceSettings?: boolean
/**
 * New roles (all of them).
 */
roles?: ({
[k: string]: number | null
} | null)
/**
 * Settings to change; the others keep their values.
 */
settings?: ({
[k: string]: SettingValue
} | null)
}
export interface DeviceConfigureIn {
config: CaptureConfig
device: string
}
export interface CaptureConfig {
/**
 * Enabled physical channels, ascending.
 */
channels: number[]
mode: CaptureMode
sampleRateHz: number
/**
 * Samples per channel.
 */
samples: number
thresholdVolts?: (number | null)
}
export interface FileOpenIn {
/**
 * The capture id to use (redo uses the id it had); a new one when absent.
 */
id?: (string | null)
/**
 * A path on the machine running the engine.
 */
path: string
/**
 * The tab title; the file name when absent.
 */
title?: (string | null)
}
export interface FileSaveIn {
capture: string
/**
 * Where to save; a path ending in .lacap. Without it, the .lacap the capture came from.
 */
path?: (string | null)
}
export interface HistoryListIn {
/**
 * Most entries to return (default 50).
 */
limit?: (number | null)
}
export interface HistoryPick {
/**
 * A history entry by id; without it, the actor's own last action.
 */
entry?: (number | null)
}
export interface MarkerAddIn {
at: number
/**
 * Who placed it, when an undo restores someone else's marker (honoured for the person only).
 */
by?: (Actor | null)
capture: string
/**
 * End of a range note.
 */
end?: (number | null)
/**
 * The marker id to use (redo and undo use the id it had); a new one when absent.
 */
id?: (string | null)
label: string
/**
 * Move the view so the marker is in the middle.
 */
moveView?: boolean
note?: (string | null)
}
export interface MarkerRef {
capture: string
marker: string
}
export interface MarkerUpdateIn {
at?: (number | null)
capture: string
clearEnd?: boolean
clearNote?: boolean
end?: (number | null)
label?: (string | null)
marker: string
note?: (string | null)
}
export interface MeasureBetweenIn {
capture: string
from: number
to: number
}
export interface MeasureEdgesIn {
capture: string
channels: number[]
/**
 * From the previous page's `nextCursor`.
 */
cursor?: (number | null)
from?: (number | null)
/**
 * Edges per page (default 1,000, at least 64, at most 10,000).
 */
limit?: (number | null)
to?: (number | null)
}
export interface MeasureStatsIn {
capture: string
channel: number
from?: (number | null)
to?: (number | null)
}
export interface AddCustomIn {
/**
 * A custom protocol as JSON: the "pulse-code" or "preset" family (doc 06).
 */
config: {
[k: string]: unknown
}
}
export interface ProtocolRef {
protocol: string
}
export interface RenderIn {
capture: string
/**
 * First sample; the person's view when absent.
 */
from?: (number | null)
/**
 * End sample (exclusive); the person's view, or the whole capture, when absent.
 */
to?: (number | null)
/**
 * Pixels (default 1200, 320 to 4000).
 */
width?: (number | null)
}
export interface SearchIn {
capture: string
/**
 * Most hits (default 100, at most 1,000).
 */
limit?: (number | null)
query: Query
}
export interface DecodedQuery {
/**
 * A decoder id on the capture.
 */
decoder: string
/**
 * Only frames with any problem.
 */
hasProblem?: boolean
kind?: (FrameKind | null)
/**
 * Exact label, such as "NACK" or "4F".
 */
label?: (string | null)
/**
 * Only frames with this problem.
 */
problem?: (ProblemCode | null)
/**
 * Contained in the label or detail (any case).
 */
text?: (string | null)
value?: (number | null)
}
export interface EdgeQuery {
channel: number
direction: Direction
}
export interface PulseQuery {
channel: number
level: Level
/**
 * Seconds.
 */
longerThan?: (number | null)
/**
 * Seconds.
 */
shorterThan?: (number | null)
}
export interface LevelQuery {
channel: number
level: Level
}
export interface WithinQuery {
from: number
to: number
/**
 * Seconds by default.
 */
unit?: (Unit | null)
}
/**
 * Values of a virtual analog channel, in its unit: where it crosses a level, or the stretches
 * where it is above, below or between levels (a gap ends a stretch). As a filter inside `all`, the
 * value at the start of a hit.
 */
export interface AnalogQuery {
/**
 * Find stretches above this value (both with `below`: between them).
 */
above?: (number | null)
/**
 * A virtual analog channel id on the capture.
 */
analog: string
/**
 * Find stretches below this value.
 */
below?: (number | null)
/**
 * Find where the value passes this level.
 */
crosses?: (number | null)
/**
 * With `crosses`: which way (any by default).
 */
direction?: (Direction | null)
/**
 * With `above` or `below`: only stretches longer or shorter than this, in seconds.
 */
longerThan?: (number | null)
shorterThan?: (number | null)
}
export interface PauseAgentsIn {
/**
 * true stops agents' capture and output commands; false lets them run again.
 */
paused: boolean
}
export interface ViewSetIn {
/**
 * Bring this capture's tab to the front.
 */
activate?: boolean
capture: string
clearCursors?: boolean
clearSelection?: boolean
cursorA?: (number | null)
cursorB?: (number | null)
from?: (number | null)
/**
 * @minItems 2
 * @maxItems 2
 */
selection?: ([number, number] | null)
to?: (number | null)
}
export interface WaveformIn {
/**
 * Pixel columns to fill (1 to 8192).
 */
buckets: number
capture: string
/**
 * Physical channels; the visible ones when absent.
 */
channels?: (number[] | null)
/**
 * Include decoder frames (default true).
 */
decoders?: (boolean | null)
from: number
/**
 * End sample (exclusive); limited to the capture's length.
 */
to: number
}
export interface AnalogOut {
analog: AnalogSummary
}
/**
 * What `session.get` says about a virtual analog channel.
 */
export interface AnalogSummary {
config: AnalogConfig
id: string
/**
 * Still following a running capture.
 */
live: boolean
max?: (number | null)
/**
 * Lowest and highest value, in the channel's unit.
 */
min?: (number | null)
notes: string[]
output: SourceOutput
/**
 * Entries in the series (values and gaps).
 */
points: number
problemCounts: {
checksum?: number
"frame-error"?: number
glitch?: number
incomplete?: number
"parity-error"?: number
protocol?: number
stuffing?: number
timing?: number
}
/**
 * One step of the code in the channel's unit, where the points are codes.
 */
step?: (number | null)
}
/**
 * A virtual analog channel's definition: what `analog.add` takes, an undo restores and a
 * `.lacap` saves.
 */
export interface AnalogConfig {
/**
 * The decoder it reads from, if any: its channels and bus settings follow that decoder's.
 */
decoder?: (string | null)
/**
 * Math channels (source `math`): the formula over other channels.
 */
expression?: (Expr | null)
/**
 * Height of its row in the waveform, in pixels (40 to 800).
 */
height?: number
name: string
roles: {
[k: string]: number | null
}
scaling: Scaling
settings?: {
[k: string]: SettingValue
}
/**
 * The source's id (`analog.sources` lists them).
 */
source: string
/**
 * How a channel is drawn: the value held until the next point, or a line between points.
 */
style?: ("step" | "line")
/**
 * The unit of its values, such as `V` or `%`.
 */
unit?: string
}
/**
 * What the points of a source mean with given settings.
 */
export interface SourceOutput {
/**
 * Bits of the code behind a fraction, for the raw code and the step size; `None` when the
 * fraction is not a code (a PWM duty).
 */
bits?: (number | null)
encoding: Encoding
kind: PointKind
/**
 * The unit of a measured quantity (`Hz`, `s`); `None` for fractions.
 */
unit?: (string | null)
}
export interface AnalogAtOut {
channels: AnalogAtChannel[]
}
/**
 * One channel's values at the asked samples, in its unit.
 */
export interface AnalogAtChannel {
analog: string
name: string
/**
 * One step of the code in the unit, where the source reads codes.
 */
step?: (number | null)
unit: string
/**
 * In the order asked; `value` is absent where the channel has none (before its first value,
 * in a gap).
 */
values: AnalogPoint[]
}
/**
 * A value in the channel's unit from `sample` on; `value` is absent in a gap.
 */
export interface AnalogPoint {
/**
 * The raw code, where the source reads codes.
 */
code?: (number | null)
sample: number
value?: (number | null)
}
export interface AnalogExportOut {
bytes: number
path: string
/**
 * Rows of values, without the header.
 */
rows: number
}
export interface AnalogSourcesOut {
sources: AnalogSourceEntry[]
}
export interface AnalogSourceEntry {
/**
 * Its settings that then come from the decoder (the bus's own).
 */
decoderSettings: string[]
/**
 * Decoder protocols it can read from instead of its own channels (`spi`, `i2c`, `pwm`).
 */
decoders: string[]
help: string
id: string
name: string
/**
 * Chips it knows (ADC presets): settings, scaling and, for several inputs, one channel each.
 */
presets: AdcPreset[]
roles: RoleSpec[]
settings: SettingSpec[]
}
export interface AdcPreset {
help: string
high: number
id: string
/**
 * For an ADC with several inputs: one virtual channel each.
 */
inputs?: PresetInput[]
/**
 * The scaling's lowest and highest value, and the unit.
 */
low: number
name: string
settings: {
[k: string]: SettingValue
}
/**
 * The source it fills (`spi-adc`, `i2c-adc`).
 */
source: string
unit: string
}
/**
 * One input of a multiplexed ADC: its name and the settings that select it.
 */
export interface PresetInput {
name: string
settings: {
[k: string]: SettingValue
}
}
/**
 * A channel a protocol needs.
 */
export interface RoleSpec {
help: string
key: string
label: string
/**
 * Short noun for setup messages: "the data line" in "Choose a channel for the data line."
 */
noun: string
optional: boolean
}
export interface SettingSpec {
help?: (string | null)
key: string
kind: SettingKind
label: string
}
/**
 * A virtual analog channel over a range, in its unit. Mean and RMS weigh each value by how long it
 * holds; gaps do not count.
 */
export interface AnalogStatsOut {
analog: string
/**
 * Values that start in the range.
 */
count: number
/**
 * The share of the range that has a value.
 */
covered: number
from: number
max?: (number | null)
mean?: (number | null)
min?: (number | null)
name: string
peakToPeak?: (number | null)
rms?: (number | null)
step?: (number | null)
to: number
unit: string
}
export interface AnalogValuesOut {
/**
 * Samples per bucket.
 */
bucket: number
buckets?: (AnalogBucket[] | null)
from: number
/**
 * The value held at `from`, from a point before it.
 */
held?: (number | null)
points?: (AnalogPoint[] | null)
/**
 * At most 1,000.
 */
problems: AnalogProblem[]
to: number
unit: string
}
/**
 * One bucket in the channel's unit.
 */
export interface AnalogBucket {
/**
 * Values starting in the bucket.
 */
count: number
/**
 * Whether a gap starts in it.
 */
gap: boolean
/**
 * The value held at the bucket's start.
 */
held?: (number | null)
max?: (number | null)
min?: (number | null)
}
/**
 * A problem in the range: a stretch without a value or with a doubtful one, and why.
 */
export interface AnalogProblem {
end: number
message: string
start: number
}
export interface ByteAnalysis {
bytes: number[]
findings: Finding[]
/**
 * The first byte read as an I2C address byte, when that makes sense.
 */
i2cFirstByte?: (I2CByte | null)
isText: boolean
/**
 * Pairs read as 16-bit values, when the bytes are not text.
 */
pairs?: (PairRow[] | null)
}
export interface Finding {
body: string
id: FindingId
/**
 * Monospace excerpt (decoded text, BCD digits).
 */
mono?: (string | null)
title: string
tone: Tone
}
export interface I2CByte {
address: number
device?: (string | null)
read: boolean
}
export interface PairRow {
bigEndian: number
bigEndianSigned: number
hex: string
littleEndian: number
littleEndianSigned: number
}
export interface CaptureAnalysis {
buses: BusAnalysis[]
measurements: ChannelMeasurement[]
/**
 * Observations across buses (a TX/RX pair), and why the window is short if it is.
 */
notes: string[]
/**
 * The samples detection looked at.
 * 
 * @minItems 2
 * @maxItems 2
 */
window: [number, number]
}
export interface BusAnalysis {
/**
 * Other candidates, for comparison and learning.
 */
alternatives: Detection[]
bus: Bus
/**
 * The best non-overlapping explanation of the bus, most confident first.
 */
chosen: Detection[]
rejections: Rejection[]
/**
 * Channels of the bus that no detection explains.
 */
unexplained: number[]
}
export interface Detection {
/**
 * Physical channels this detection explains, ascending.
 */
channels: number[]
/**
 * 0 to 1.
 */
confidence: number
evidence: Evidence[]
/**
 * Why each setting (or role, keyed "role:<key>") has its value.
 */
notes: {
[k: string]: SettingNote
}
protocolId: string
roles: {
[k: string]: number | null
}
settings: {
[k: string]: SettingValue
}
warnings: string[]
}
export interface Evidence {
/**
 * Whether the observation supports the protocol or counts against it.
 */
supports: boolean
text: string
}
export interface SettingNote {
/**
 * 0 to 1.
 */
confidence: number
reason: string
}
export interface Bus {
/**
 * Physical channel numbers, ascending.
 */
channels: number[]
id: string
/**
 * Why the channels were put together (or left alone), in plain words.
 */
reason: string
}
export interface Rejection {
because: string
protocolId: string
}
export interface ChannelMeasurement {
burstGap: number
/**
 * Stretches of activity, split at pauses longer than `burst_gap`.
 */
bursts: Span[]
/**
 * Physical channel number.
 */
channel: number
/**
 * Pulse widths grouped, narrowest first.
 */
clusters: WidthCluster[]
/**
 * Regular periods with a changing duty; only when it is not [`Self::periodic`].
 */
dutyLevel?: (DutyLevel | null)
edgeCount: number
firstEdge?: (number | null)
/**
 * Pulses well under half the narrowest repeated width.
 */
glitches: number
/**
 * Fraction of the active stretch spent high.
 */
highFraction: number
lastEdge?: (number | null)
/**
 * Narrowest pulse that occurs repeatedly (single glitches ignored).
 */
narrowest?: (Narrowest | null)
periodic?: (Periodicity | null)
/**
 * A digital level.
 */
restLevel: ("low" | "high")
}
/**
 * A half-open stretch of samples; fractional ends come from averaged periods.
 */
export interface Span {
end: number
start: number
}
/**
 * A group of similar pulse widths.
 */
export interface WidthCluster {
count: number
high: number
low: number
max: number
mean: number
min: number
}
/**
 * A PWM that carries a level: periods of the same length, a high time that changes (a fan, a
 * dimmer, a DAC by PWM). Clocks and steady PWM are [`Periodicity`] instead.
 */
export interface DutyLevel {
/**
 * The signal never pauses between its first and last edge and spans most of the window.
 */
continuous: boolean
dutyMax: number
dutyMean: number
/**
 * Lowest and highest duty, ignoring the outer 2 % of periods, and the mean.
 */
dutyMin: number
/**
 * Average rising-to-rising period, in samples.
 */
periodSamples: number
/**
 * Periods measured.
 */
periods: number
/**
 * Fraction of periods within a few percent of the typical one.
 */
regularity: number
}
export interface Narrowest {
count: number
samples: number
}
export interface Periodicity {
bursts: Span[]
/**
 * The signal never pauses between its first and last edge and spans most of the window.
 */
continuous: boolean
dutyCycle: number
/**
 * Average high time, in samples.
 */
highSamples: number
/**
 * Average rising-to-rising period inside bursts, in samples.
 */
periodSamples: number
/**
 * Clock pulses per burst (bursts split at pauses longer than 3 periods).
 */
pulsesPerBurst: number[]
/**
 * Fraction of periods within a few percent of the typical one (1 = perfectly regular).
 */
regularity: number
}
export interface CaptureStartOut {
/**
 * The finished capture, when waited for.
 */
capture?: (CaptureSummary | null)
/**
 * The id the capture gets.
 */
id: string
running: boolean
}
export interface CaptureSummary {
/**
 * Virtual analog channels (ADR 0010).
 */
analog: AnalogSummary[]
/**
 * Samples left out of analysis (a start-up artefact).
 * 
 * @minItems 2
 * @maxItems 2
 */
artefact?: ([number, number] | null)
channels: ChannelEntry[]
decoders: DecoderSummary[]
id: string
markers: Marker[]
notes: Note[]
path?: (string | null)
sampleCount: number
sampleRateHz: number
/**
 * Decoder settings found in the file: for comparison, never used by detection.
 */
savedDecoders: SavedDecoder[]
seconds: number
source: Source
state: CaptureState
title: string
trigger?: (number | null)
/**
 * Closing it loses something: a capture from a device that was never saved, or decoders,
 * markers, names or groups changed since the file was opened or saved.
 */
unsaved: boolean
view: View
}
export interface ChannelEntry {
colour?: (string | null)
group?: (string | null)
hidden: boolean
index: number
name: string
}
export interface DecoderSummary {
/**
 * Its channels in the protocol's role order (SPI: CLK, CS, MOSI, MISO): the order they are
 * shown in, grouped under the decoder.
 */
channels: number[]
config: DecoderConfig
frames: number
id: string
/**
 * Channels its data appears under (SPI: MOSI and MISO; UART: RX and TX), in order.
 */
lanes: number[]
notes: string[]
packets: number
problemCounts: {
checksum?: number
"frame-error"?: number
glitch?: number
incomplete?: number
"parity-error"?: number
protocol?: number
stuffing?: number
timing?: number
}
}
/**
 * A decoder's configuration: what `decoder.add` takes and an undo restores.
 */
export interface DecoderConfig {
/**
 * How a decoder's data values are written out for people (the waveform's text row, doc 08).
 */
dataFormat?: ("ascii" | "hex" | "decimal" | "binary")
name: string
protocol: string
roles: {
[k: string]: number | null
}
settings: {
[k: string]: SettingValue
}
}
export interface Marker {
/**
 * Sample of the marker, or the start of a range note.
 */
at: number
/**
 * Who sent a command: user, agent:<client name> or cli.
 */
by: string
/**
 * End of a range note.
 */
end?: (number | null)
id: string
label: string
note?: (string | null)
}
/**
 * Format surprises, dropped data, artefacts: shown to the person and in MCP summaries.
 */
export interface Note {
level: NoteLevel
message: string
}
/**
 * Decoder settings found in an imported file: shown for comparison, never used by detection.
 */
export interface SavedDecoder {
/**
 * Role name to physical channel; `None` where the file leaves it unassigned.
 */
channels: [string, (number | null)][]
/**
 * Option name to value, as text.
 */
options: [string, string][]
/**
 * The file's own protocol name, normalised to lower case (`uart`, `i2c`, `spi`, …).
 */
protocol: string
}
export interface Source {
/**
 * For example "FNIRSI .demo file mystery_D.demo" or "synthetic device, program mix".
 */
detail: string
kind: SourceKind
}
/**
 * A tab's view (doc 08): the visible range, cursors and selection.
 */
export interface View {
cursorA?: (number | null)
cursorB?: (number | null)
from: number
/**
 * Selected range, start and end.
 * 
 * @minItems 2
 * @maxItems 2
 */
selection?: ([number, number] | null)
to: number
}
export interface DecoderOut {
decoder: DecoderSummary
}
export interface DecoderFramesOut {
explanation?: (Explanation | null)
frames: Frame[]
nextCursor?: (number | null)
packets: PacketItem[]
/**
 * Items that match, over all pages.
 */
total: number
}
export interface Explanation {
/**
 * Symptoms, causes and actions for this protocol (doc 06, symptom tables).
 */
causes: Symptom[]
problems: ProblemGroup[]
}
export interface Symptom {
action: string
cause: string
symptom: string
}
export interface ProblemGroup {
code: ProblemCode
count: number
/**
 * Sample of the first one, to jump to.
 */
first?: (number | null)
/**
 * Different messages seen (at most three).
 */
messages: string[]
}
/**
 * One annotation.
 */
export interface Frame {
/**
 * Longer explanation for the table and tooltips.
 */
detail?: (string | null)
end: number
kind: FrameKind
/**
 * Short label under the waveform, such as "4F".
 */
label: string
/**
 * Physical channel it belongs under.
 */
lane: number
/**
 * Index of the packet it belongs to.
 */
packet: number
problems: Problem[]
/**
 * Annotation row under the lane: 0 for bytes and fields, 1 for single bits.
 */
row: number
start: number
value?: (number | null)
}
export interface Problem {
code: ProblemCode
message: string
}
export interface PacketItem {
packet: Packet
/**
 * Every stream as text, labelled: "Write to 0x57: ·· | Read from 0x57: CALIBRATED 2026".
 */
text: string
}
/**
 * A group of frames (a UART line, an I2C transaction, a chip-select frame) with its bytes.
 */
export interface Packet {
end: number
index: number
start: number
streams: PacketStream[]
title: string
}
export interface PacketStream {
bytes: number[]
/**
 * "MOSI", "Write to 0x50", "RX", …
 */
label: string
lane: number
}
export interface DecoderUpdateOut {
decoder: DecoderSummary
/**
 * How the problem counts changed.
 */
problemChanges: ProblemChange[]
}
export interface ProblemChange {
after: number
before: number
code: ProblemCode
}
export interface DeviceEntry {
capabilities: Capabilities
/**
 * Whether a capture is running on it.
 */
capturing: boolean
connected: boolean
/**
 * Who holds the lease, if anyone.
 */
holder?: (Actor | null)
info: DeviceInfo
}
export interface Capabilities {
/**
 * The names the device gives its channels, in the order of `channels`; empty when it names
 * none (then they are CH0, CH1, …).
 */
channelNames: string[]
/**
 * Physical channel numbers.
 */
channels: number[]
/**
 * Rate limits by number of enabled channels, fewest channels first.
 */
limits: RateLimit[]
/**
 * Most samples per channel in one capture, where limited (buffer depth).
 */
maxSamples?: (number | null)
modes: CaptureMode[]
/**
 * Outputs (PWM); added with the first device that has them (M3).
 */
outputs: number
/**
 * Rates offered.
 */
ratesHz: number[]
/**
 * Adjustable threshold: lowest, highest and step in volts.
 * 
 * @minItems 3
 * @maxItems 3
 */
threshold?: ([number, number, number] | null)
}
/**
 * The fastest rate with up to `max_channels` channels enabled.
 */
export interface RateLimit {
maxChannels: number
maxRateHz: number
}
export interface DeviceInfo {
channels: number
/**
 * Driver id: "vla", "fnirsi-dla", "fx2lafw" ("synthetic" in tests).
 */
driver: string
firmware?: (string | null)
/**
 * Stable id, such as "vla/127.0.0.1:7171" or a USB serial.
 */
id: string
model: string
serial?: (string | null)
}
export interface DeviceListOut {
devices: DeviceEntry[]
}
export interface CaptureOut {
capture: CaptureSummary
}
export interface FileSaveOut {
bytes: number
path: string
}
export interface HistoryListOut {
/**
 * Newest first.
 */
entries: HistoryEntry[]
}
/**
 * One entry of the engine-wide history (ADR 0008).
 */
export interface HistoryEntry {
actor: Actor
/**
 * Undone, and then the same actor did something new: it can no longer be redone.
 */
discarded: boolean
done: Invocation
id: number
/**
 * `None` when the command cannot be undone.
 */
inverse?: (Invocation | null)
/**
 * Why it cannot be undone, when it cannot.
 */
neverBecause?: (string | null)
/**
 * Plain-language line for the activity feed: "Claude added an I2C decoder on CH4 and CH5".
 */
summary: string
undone: boolean
}
/**
 * A command call as data: what the history stores and `run_command` takes.
 */
export interface Invocation {
command: string
input: unknown
}
export interface HistoryOut {
entry: HistoryEntry
}
export interface LeaseOut {
holder?: (Actor | null)
}
export interface MarkerOut {
marker: Marker
}
export interface MeasureBetweenOut {
/**
 * 1 ÷ the time, when it is not zero.
 */
frequencyHz?: (number | null)
samples: number
seconds: number
}
export interface MeasureEdgesOut {
edges: EdgeOut[]
nextCursor?: (number | null)
}
export interface EdgeOut {
channel: number
/**
 * A digital level.
 */
level: ("low" | "high")
sample: number
seconds: number
}
export interface MeasureStatsOut {
channel: number
dutyCycle?: (number | null)
/**
 * Level changes in the range (exact).
 */
edges: number
frequencyHz?: (number | null)
from: number
/**
 * Pulses well under half the narrowest repeated width.
 */
glitches: number
highSeconds?: (number | null)
/**
 * Narrowest pulse that occurs repeatedly.
 */
narrowestPulseSeconds?: (number | null)
/**
 * Set when only part of the range was measured.
 */
note?: (string | null)
periodSeconds?: (number | null)
restLevel: Level
seconds: number
to: number
}
export interface ProtocolEntry {
aliasOf?: (string | null)
group: string
hasDecoder: boolean
id: string
name: string
origin: Origin
roles: RoleSpec[]
settings: SettingSpec[]
}
export interface ProtocolInfoOut {
info: ProtocolInfo
protocol: ProtocolEntry
}
/**
 * What the person (or agent) can learn about a protocol from the tool itself.
 */
export interface ProtocolInfo {
/**
 * How to confirm it on the waveform.
 */
confirm: string
/**
 * What to set in the decoder.
 */
decoderTips: string
/**
 * What else looks similar and how to tell them apart.
 */
lookalikes: string
symptoms: Symptom[]
/**
 * What it is and where you meet it.
 */
what: string
}
export interface ProtocolListOut {
protocols: ProtocolEntry[]
}
export interface RenderOut {
from: number
height: number
/**
 * The PNG, base64-encoded.
 */
pngBase64: string
/**
 * What the picture shows, top to bottom: the time axis, channels, decoder rows, markers.
 */
rows: string[]
to: number
width: number
}
export interface SearchOut {
hits: Hit[]
/**
 * More hits exist than were returned.
 */
truncated: boolean
}
export interface Hit {
end: number
seconds: number
start: number
/**
 * What was found: a frame label, "rising edge on CH5", "high pulse of 80 ns on CH3".
 */
what: string
}
export interface SessionOut {
/**
 * The capture whose tab is in front.
 */
active?: (string | null)
/**
 * Agents' capture and output commands are refused (the person's "stop agent actions").
 */
agentsPaused: boolean
/**
 * "major.minor" (doc 03).
 */
apiVersion: string
captures: CaptureSummary[]
devices: DeviceEntry[]
/**
 * The latest history entries, newest first.
 */
recent: HistoryEntry[]
/**
 * Grows with every change; `state.changed` events carry it.
 */
revision: number
running: RunningEntry[]
}
export interface RunningEntry {
capture: string
device: string
}
export interface PauseAgentsOut {
agentsPaused: boolean
/**
 * Captures started by agents that were stopped by this switch.
 */
stopped: string[]
}
export interface ViewOut {
view: View
}
export interface WaveformOut {
/**
 * Samples per bucket (at least 1).
 */
bucket: number
decoders: WaveDecoder[]
from: number
to: number
traces: WaveTrace[]
}
export interface WaveDecoder {
/**
 * Frames in the range, when there are few enough to draw one by one.
 */
frames?: (WaveFrame[] | null)
id: string
name: string
/**
 * Otherwise stretches of frames closer than a bucket, per channel drawn under, as
 * (start, end, any problem, lane).
 */
spans?: ([number, number, boolean, number][] | null)
}
export interface WaveFrame {
end: number
kind: FrameKind
label: string
/**
 * Physical channel it is drawn under: one of the decoder's data channels (`lanes` in the
 * session); frames on another channel (an I2C start on SCL) go under the first of them.
 */
lane: number
/**
 * Packet it belongs to: neighbours in one packet join into a string in the text row.
 */
packet: number
problem: boolean
start: number
/**
 * For data frames: the value in the decoder's data format ("A", "65", "01000001").
 */
text?: (string | null)
}
export interface WaveTrace {
channel: number
/**
 * Otherwise transitions per bucket of `bucket` samples from `from`.
 */
counts?: (number[] | null)
/**
 * Every edge in the range (first samples at a new level), when there are few enough to draw.
 */
edges?: (number[] | null)
/**
 * And the level at the start of each bucket: "0" or "1" per bucket.
 */
levels?: (string | null)
/**
 * A digital level.
 */
startLevel: ("low" | "high")
}
/**
 * An error worded for the person or agent: what is wrong and what to do.
 */
export interface CommandError {
code: ErrorCode
message: string
}
