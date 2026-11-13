//! JTAG: test access port with TCK, TMS, TDI and TDO.
//!
//! Stub: the agent implementing this module replaces it (see `../signal-detective/src/core/protocols/jtag.ts`).

use crate::protocol::{Protocol, ProtocolInfo};
use crate::types::*;

pub struct Jtag {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for Jtag {
    fn default() -> Self {
        Jtag {
            roles: vec![
                RoleSpec::new("tck", "TCK (clock)", "TCK", "The clock."),
                RoleSpec::new("tms", "TMS (mode select)", "TMS", "Steps the TAP state machine."),
                RoleSpec::new("tdi", "TDI (data in)", "TDI", "From the probe to the chip.").optional(),
                RoleSpec::new("tdo", "TDO (data out)", "TDO", "From the chip to the probe.").optional(),
            ],
            info: ProtocolInfo::default(),
        }
    }
}

impl Protocol for Jtag {
    fn id(&self) -> &str {
        "jtag"
    }
    fn name(&self) -> &str {
        "JTAG"
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
