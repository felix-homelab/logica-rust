//! I2S audio: a free-running bit clock, word select and serial data.
//!
//! Stub: the agent implementing this module replaces it (see `../signal-detective/src/core/protocols/i2s.ts`).

use crate::protocol::{Protocol, ProtocolInfo};
use crate::types::*;

pub struct I2s {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for I2s {
    fn default() -> Self {
        I2s {
            roles: vec![
                RoleSpec::new("sck", "Bit clock (SCK, BCLK)", "the bit clock", "Runs all the time."),
                RoleSpec::new(
                    "ws",
                    "Word select (WS, LRCLK)",
                    "word select",
                    "Slow square wave at the sample rate.",
                ),
                RoleSpec::new("sd", "Data (SD)", "the data line", "Changes with the bit clock."),
            ],
            info: ProtocolInfo::default(),
        }
    }
}

impl Protocol for I2s {
    fn id(&self) -> &str {
        "i2s"
    }
    fn name(&self) -> &str {
        "I2S audio"
    }
    fn group(&self) -> &str {
        "Audio"
    }
    fn info(&self) -> &ProtocolInfo {
        &self.info
    }
    fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }
    fn settings(&self) -> &[SettingSpec] {
        &[]
    }
}
