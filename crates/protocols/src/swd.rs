//! SWD: the ARM serial wire debug port, a clock and one bidirectional data line.
//!
//! Stub: the agent implementing this module replaces it (see `../signal-detective/src/core/protocols/swd.ts`).

use crate::protocol::{Protocol, ProtocolInfo};
use crate::types::*;

pub struct Swd {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for Swd {
    fn default() -> Self {
        Swd {
            roles: vec![
                RoleSpec::new("swclk", "SWCLK", "SWCLK", "The clock from the debug probe."),
                RoleSpec::new("swdio", "SWDIO", "SWDIO", "The data line, driven by both sides in turn."),
            ],
            info: ProtocolInfo::default(),
        }
    }
}

impl Protocol for Swd {
    fn id(&self) -> &str {
        "swd"
    }
    fn name(&self) -> &str {
        "SWD debug port"
    }
    fn group(&self) -> &str {
        "Debug"
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
