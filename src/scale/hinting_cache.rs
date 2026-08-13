use alloc::vec::Vec;
use skrifa::{
    instance::{NormalizedCoord, Size},
    outline::{
        Engine, HintingInstance, HintingOptions, OutlineGlyphCollection, OutlineGlyphFormat,
        SmoothMode, Target,
    },
};

use super::{HintingEngine, HintingTarget};

/// We keep this small to enable a simple LRU cache with a linear
/// search. Regenerating hinting data is low to medium cost so it's fine
/// to redo it occasionally.
const MAX_CACHED_HINT_INSTANCES: usize = 8;

pub(crate) struct HintingKey<'a> {
    pub id: [u64; 2],
    pub outlines: &'a OutlineGlyphCollection<'a>,
    pub size: Size,
    pub coords: &'a [NormalizedCoord],
    pub engine: HintingEngine,
    pub target: HintingTarget,
}

impl<'a> HintingKey<'a> {
    fn new_instance(&self) -> Option<HintingInstance> {
        HintingInstance::new(self.outlines, self.size, self.coords, self.options()).ok()
    }

    fn options(&self) -> HintingOptions {
        HintingOptions {
            engine: match self.engine {
                HintingEngine::AutoFallback => Engine::AutoFallback,
                HintingEngine::Auto => Engine::Auto(None),
                HintingEngine::Interpreter => Engine::Interpreter,
            },
            target: match self.target {
                HintingTarget::Mono => Target::Mono,
                target => Target::Smooth {
                    mode: match target {
                        HintingTarget::Light => SmoothMode::Light,
                        HintingTarget::Lcd => SmoothMode::Lcd,
                        HintingTarget::VerticalLcd => SmoothMode::VerticalLcd,
                        _ => SmoothMode::Normal,
                    },
                    symmetric_rendering: true,
                    preserve_linear_metrics: true,
                },
            },
        }
    }
}

#[derive(Default)]
pub(super) struct HintingCache {
    // Split caches because the instance type can reuse internal memory when
    // reconfigured for the same format.
    glyf_entries: Vec<HintingEntry>,
    cff_entries: Vec<HintingEntry>,
    other_entries: Vec<HintingEntry>,
    serial: u64,
}

impl HintingCache {
    pub(super) fn get(&mut self, key: &HintingKey) -> Option<&HintingInstance> {
        let entries = match key.outlines.format()? {
            OutlineGlyphFormat::Glyf => &mut self.glyf_entries,
            OutlineGlyphFormat::Cff | OutlineGlyphFormat::Cff2 => &mut self.cff_entries,
            #[allow(unreachable_patterns)]
            _ => &mut self.other_entries,
        };
        let (entry_ix, is_current) = find_hinting_entry(entries, key)?;
        let entry = entries.get_mut(entry_ix)?;
        self.serial += 1;
        entry.serial = self.serial;
        if !is_current {
            entry.id = key.id;
            entry.size = key.size;
            entry.engine = key.engine;
            entry.target = key.target;
            entry.coords.clear();
            entry.coords.extend_from_slice(key.coords);
            entry
                .instance
                .reconfigure(key.outlines, key.size, key.coords, key.options())
                .ok()?;
        }
        Some(&entry.instance)
    }
}

struct HintingEntry {
    id: [u64; 2],
    size: Size,
    coords: Vec<NormalizedCoord>,
    engine: HintingEngine,
    target: HintingTarget,
    instance: HintingInstance,
    serial: u64,
}

fn find_hinting_entry(entries: &mut Vec<HintingEntry>, key: &HintingKey) -> Option<(usize, bool)> {
    let mut found_serial = u64::MAX;
    let mut found_index = 0;
    for (ix, entry) in entries.iter().enumerate() {
        if entry.id == key.id
            && entry.size == key.size
            && entry.coords == key.coords
            && entry.engine == key.engine
            && entry.target == key.target
        {
            return Some((ix, true));
        }
        if entry.serial < found_serial {
            found_serial = entry.serial;
            found_index = ix;
        }
    }
    if entries.len() < MAX_CACHED_HINT_INSTANCES {
        let instance = key.new_instance()?;
        let ix = entries.len();
        entries.push(HintingEntry {
            id: key.id,
            size: key.size,
            coords: key.coords.to_vec(),
            engine: key.engine,
            target: key.target,
            instance,
            serial: 0,
        });
        Some((ix, true))
    } else {
        Some((found_index, false))
    }
}
