//! ADC presets (doc 06): data files that fill a serial ADC source's settings and scaling, as
//! register maps describe chips. Loaded from `data/adc/presets.json` and checked against the
//! sources, so a preset can never name a setting its source does not have.

use serde::{Deserialize, Serialize};

use crate::types::Settings;

/// One input of a multiplexed ADC: its name and the settings that select it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetInput {
    pub name: String,
    pub settings: Settings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdcPreset {
    pub id: String,
    pub name: String,
    /// The source it fills (`spi-adc`, `i2c-adc`).
    pub source: String,
    pub help: String,
    pub settings: Settings,
    /// The scaling's lowest and highest value, and the unit.
    pub low: f64,
    pub high: f64,
    pub unit: String,
    /// For an ADC with several inputs: one virtual channel each.
    #[serde(default)]
    pub inputs: Vec<PresetInput>,
}

/// Every built-in preset.
pub fn presets() -> Vec<AdcPreset> {
    serde_json::from_str(include_str!("../../data/adc/presets.json")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analog::source;

    #[test]
    fn every_preset_fills_settings_its_source_has() {
        let all: Vec<AdcPreset> =
            serde_json::from_str(include_str!("../../data/adc/presets.json")).expect("presets.json is valid");
        assert!(all.len() >= 3);
        for p in &all {
            let src = source(&p.source).unwrap_or_else(|| panic!("{}: no source {}", p.id, p.source));
            let keys: Vec<&str> = src.settings().iter().map(|s| s.key.as_str()).collect();
            for k in p.settings.keys().chain(p.inputs.iter().flat_map(|i| i.settings.keys())) {
                assert!(keys.contains(&k.as_str()), "{}: {k} is not a setting of {}", p.id, p.source);
            }
            assert!(p.low < p.high && p.help.ends_with('.'), "{}", p.id);
        }
        assert_eq!(all.iter().find(|p| p.id == "mcp3208").map(|p| p.inputs.len()), Some(8));
        assert_eq!(presets(), all);
    }
}
