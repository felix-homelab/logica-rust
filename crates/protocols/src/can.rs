//! CAN on the logic side of a transceiver: one line, rests high, bit stuffing.
//!
//! Stub: the agent implementing this module replaces it (see `../signal-detective/src/core/protocols/can.ts`).

use crate::protocol::{Protocol, ProtocolInfo};
use crate::types::*;

pub struct Can {
    roles: Vec<RoleSpec>,
    info: ProtocolInfo,
}

impl Default for Can {
    fn default() -> Self {
        Can {
            roles: vec![RoleSpec::new(
                "rx",
                "CAN RX",
                "CAN RX",
                "The transceiver RX pin (logic level, rests high).",
            )],
            info: ProtocolInfo::default(),
        }
    }
}

impl Protocol for Can {
    fn id(&self) -> &str {
        "can"
    }
    fn name(&self) -> &str {
        "CAN (logic side)"
    }
    fn group(&self) -> &str {
        "CAN"
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
