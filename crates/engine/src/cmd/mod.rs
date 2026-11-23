//! Command handlers, and the one table that binds every command name to its input type, output
//! type and handler. The JSON Schema of the contract is generated from the same table.

mod analysis;
mod decoders;
mod devices;
mod edit;
mod files;
mod session;
mod waveform;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contract::{CommandError, ErrorCode, Invocation};
use crate::engine::{Ctx, Done, Engine, err};

pub use analysis::*;
pub use decoders::*;
pub use devices::*;
pub use edit::*;
pub use files::*;
pub use session::*;
pub use waveform::*;

fn parse<T: DeserializeOwned>(command: &str, input: Value) -> Result<T, CommandError> {
    let input = if input.is_null() { Value::Object(Default::default()) } else { input };
    serde_json::from_value(input).map_err(|e| {
        err(
            ErrorCode::InvalidInput,
            format!(
                "The input of {command} is not valid: {e}. Its schema is in the resource logica://commands."
            ),
        )
    })
}

/// Serialises an output.
pub(crate) fn out<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// An invocation built from a serialisable input.
pub(crate) fn inv<T: Serialize>(command: &str, input: &T) -> Invocation {
    Invocation { command: command.into(), input: out(input) }
}

/// Input of commands that take nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

macro_rules! commands {
    ($( $name:literal : $input:ty => $output:ty = $handler:path ;)*) => {
        pub(crate) fn dispatch(e: &mut Engine, ctx: &Ctx, name: &str, input: Value) -> Result<Done, CommandError> {
            match name {
                $( $name => { let i: $input = parse(name, input)?; $handler(e, ctx, i) } )*
                _ => Err(err(ErrorCode::NotFound, format!("There is no command \"{name}\"."))),
            }
        }

        /// (name, input schema, output schema) of every command.
        pub fn schemas(g: &mut schemars::SchemaGenerator) -> Vec<(&'static str, schemars::Schema, schemars::Schema)> {
            vec![ $( ($name, g.subschema_for::<$input>(), g.subschema_for::<$output>()) ),* ]
        }

        /// Every command name in the table.
        pub fn names() -> Vec<&'static str> {
            vec![ $( $name ),* ]
        }
    };
}

commands! {
    "session.get": Nothing => SessionOut = session::get;
    "session.pauseAgents": PauseAgentsIn => PauseAgentsOut = session::pause_agents;
    "waveform.get": WaveformIn => WaveformOut = waveform::get;
    "file.open": FileOpenIn => CaptureOut = files::open;
    "file.save": FileSaveIn => FileSaveOut = files::save;
    "capture.close": CaptureRef => Nothing = files::close;
    "capture.rename": CaptureRenameIn => Nothing = files::rename;
    "device.list": Nothing => DeviceListOut = devices::list;
    "device.connect": DeviceRef => DeviceEntry = devices::connect;
    "device.disconnect": DeviceRef => Nothing = devices::disconnect;
    "device.configure": DeviceConfigureIn => Nothing = devices::configure;
    "capture.start": CaptureStartIn => CaptureStartOut = devices::start;
    "capture.stop": DeviceRef => Nothing = devices::stop;
    "lease.take": DeviceRef => LeaseOut = devices::take;
    "lease.release": DeviceRef => LeaseOut = devices::release;
    "channel.rename": ChannelRenameIn => Nothing = edit::channel_rename;
    "channel.colour": ChannelColourIn => Nothing = edit::channel_colour;
    "channel.group": ChannelGroupIn => Nothing = edit::channel_group;
    "channel.hide": ChannelHideIn => Nothing = edit::channel_hide;
    "view.set": ViewSetIn => ViewOut = edit::view_set;
    "analysis.detect": RangeIn => logica_protocols::analyse::CaptureAnalysis = analysis::detect;
    "analysis.bytes": BytesIn => logica_protocols::bytes::ByteAnalysis = analysis::bytes;
    "measure.stats": MeasureStatsIn => MeasureStatsOut = analysis::stats;
    "measure.edges": MeasureEdgesIn => MeasureEdgesOut = analysis::edges;
    "measure.between": MeasureBetweenIn => MeasureBetweenOut = analysis::between;
    "decoder.add": DecoderAddIn => DecoderOut = decoders::add;
    "decoder.update": DecoderUpdateIn => DecoderUpdateOut = decoders::update;
    "decoder.remove": DecoderRef => Nothing = decoders::remove;
    "decoder.frames": DecoderFramesIn => DecoderFramesOut = decoders::frames;
    "search.run": SearchIn => SearchOut = analysis::search;
    "marker.add": MarkerAddIn => MarkerOut = edit::marker_add;
    "marker.update": MarkerUpdateIn => MarkerOut = edit::marker_update;
    "marker.remove": MarkerRef => Nothing = edit::marker_remove;
    "protocol.list": Nothing => ProtocolListOut = analysis::protocol_list;
    "protocol.info": ProtocolRef => ProtocolInfoOut = analysis::protocol_info;
    "protocol.addCustom": AddCustomIn => ProtocolEntry = analysis::add_custom;
    "protocol.removeCustom": ProtocolRef => Nothing = analysis::remove_custom;
    "render": RenderIn => RenderOut = analysis::render;
    "history.list": HistoryListIn => HistoryListOut = session::history_list;
    "history.undo": HistoryPick => HistoryOut = session::history_pick;
    "history.redo": HistoryPick => HistoryOut = session::history_pick;
}
