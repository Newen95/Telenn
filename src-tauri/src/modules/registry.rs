//! Module registry: maps a stable module id to a fresh node instance.
//!
//! `song.json` (M3) and the graph compiler reference modules by id. Adding a
//! module means adding one arm here and one `mod` line — the engine core is
//! untouched.

use serde::Serialize;

use crate::engine::node::AudioNode;
use crate::modules::_template::Gain;
use crate::modules::chorus::Chorus;
use crate::modules::compressor::Compressor;
use crate::modules::delay::Delay;
use crate::modules::drum_sampler::DrumSampler;
use crate::modules::filter::Filter;
use crate::modules::fm::Fm;
use crate::modules::plucked::Plucked;
use crate::modules::reverb::Reverb;
use crate::modules::saturator::Saturator;
use crate::modules::subtractive::Subtractive;

/// Category of a module, so the UI knows where it may go: `Instrument` heads a
/// track's chain (never drag-reordered in M6), `Effect` lives in the effects
/// region and is the only kind the chain drag & drop moves around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleCategory {
    Instrument,
    Effect,
}

/// Browser-facing description of a registered module. The display name stays in
/// English here (code language); the UI maps the `id`/`category` to i18n labels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDesc {
    pub id: &'static str,
    pub name: &'static str,
    pub category: ModuleCategory,
}

/// Every registered module. The single source of truth for both `list` (browser)
/// and `create` (instances) — keep the two in sync (a test guards it).
const MODULES: &[ModuleDesc] = &[
    ModuleDesc {
        id: "subtractive",
        name: "Subtractive",
        category: ModuleCategory::Instrument,
    },
    ModuleDesc {
        id: "plucked",
        name: "Plucked String",
        category: ModuleCategory::Instrument,
    },
    ModuleDesc {
        id: "fm",
        name: "FM",
        category: ModuleCategory::Instrument,
    },
    ModuleDesc {
        id: "drum_sampler",
        name: "Drum Sampler",
        category: ModuleCategory::Instrument,
    },
    ModuleDesc {
        id: "gain",
        name: "Gain",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "filter",
        name: "Filter",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "delay",
        name: "Delay",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "reverb",
        name: "Reverb",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "chorus",
        name: "Chorus",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "saturator",
        name: "Saturator",
        category: ModuleCategory::Effect,
    },
    ModuleDesc {
        id: "compressor",
        name: "Compressor",
        category: ModuleCategory::Effect,
    },
];

fn boxed<N: AudioNode + Default + 'static>() -> Box<dyn AudioNode> {
    Box::new(N::default())
}

/// All registered modules, for the preset/module browser.
pub fn list() -> &'static [ModuleDesc] {
    MODULES
}

/// Instantiate a module by id, or `None` if unknown.
pub fn create(module_id: &str) -> Option<Box<dyn AudioNode>> {
    match module_id {
        "subtractive" => Some(boxed::<Subtractive>()),
        "plucked" => Some(boxed::<Plucked>()),
        "fm" => Some(boxed::<Fm>()),
        "drum_sampler" => Some(boxed::<DrumSampler>()),
        "gain" => Some(boxed::<Gain>()),
        "filter" => Some(boxed::<Filter>()),
        "delay" => Some(boxed::<Delay>()),
        "reverb" => Some(boxed::<Reverb>()),
        "chorus" => Some(boxed::<Chorus>()),
        "saturator" => Some(boxed::<Saturator>()),
        "compressor" => Some(boxed::<Compressor>()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_module_can_be_created() {
        for desc in list() {
            assert!(
                create(desc.id).is_some(),
                "listed module '{}' has no instance",
                desc.id
            );
        }
    }
}
