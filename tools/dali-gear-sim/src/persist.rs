use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_gear_model::{
    Gear, GearFleet, GearSpec, DEFAULT_RGBWAF_CHANNELS, DEVICE_TYPE_DIAGNOSTICS,
    DEVICE_TYPE_ENERGY, DEVICE_TYPE_LUMINAIRE_INFO, SCENE_COUNT,
};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "gearsim";
const KEY: &str = "fleet";
const FORMAT_VERSION: u16 = 1;
const MAX_BLOB_BYTES: usize = 8192;

#[derive(Serialize, Deserialize)]
struct StoredGear {
    short: Option<u8>,
    random: u32,
    device_types: Vec<u8>,
    caps: (bool, bool, bool),
    tc_range: (u16, u16),
    groups: u16,
    scenes: [u8; SCENE_COUNT],
    min_level: u8,
    max_level: u8,
    power_on_level: u8,
    system_failure_level: u8,
    fade_time: u8,
    fade_rate: u8,
    extended_fade_time: u8,
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct StoredFleet {
    version: u16,
    reserved: u64,
    gears: Vec<StoredGear>,
}

pub struct Store {
    nvs: EspNvs<NvsDefault>,
}

#[derive(Debug)]
pub enum StoreError {
    Nvs(esp_idf_svc::sys::EspError),
    Codec,
    Empty,
    Version(u16),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreError::Nvs(e) => write!(f, "nvs error {e}"),
            StoreError::Codec => write!(f, "blob did not decode"),
            StoreError::Empty => write!(f, "no stored fleet"),
            StoreError::Version(v) => write!(f, "stored format {v}, expected {FORMAT_VERSION}"),
        }
    }
}

impl Store {
    pub fn open() -> Result<Self, StoreError> {
        let partition = EspNvsPartition::<NvsDefault>::take().map_err(StoreError::Nvs)?;
        let nvs = EspNvs::new(partition, NAMESPACE, true).map_err(StoreError::Nvs)?;
        Ok(Self { nvs })
    }

    pub fn load(&self, reserved_fallback: u64) -> Result<GearFleet, StoreError> {
        let mut buf = vec![0u8; MAX_BLOB_BYTES];
        let raw = self
            .nvs
            .get_blob(KEY, &mut buf)
            .map_err(StoreError::Nvs)?
            .ok_or(StoreError::Empty)?;
        let stored: StoredFleet = postcard::from_bytes(raw).map_err(|_| StoreError::Codec)?;
        if stored.version != FORMAT_VERSION {
            return Err(StoreError::Version(stored.version));
        }
        let reserved = if stored.reserved == 0 {
            reserved_fallback
        } else {
            stored.reserved
        };
        Ok(rebuild(stored, reserved))
    }

    pub fn save(&mut self, fleet: &GearFleet) -> Result<usize, StoreError> {
        let stored = StoredFleet {
            version: FORMAT_VERSION,
            reserved: fleet.reserved(),
            gears: fleet.gears().iter().map(store_one).collect(),
        };
        let bytes = postcard::to_allocvec(&stored).map_err(|_| StoreError::Codec)?;
        self.nvs.set_blob(KEY, &bytes).map_err(StoreError::Nvs)?;
        Ok(bytes.len())
    }

    pub fn erase(&mut self) -> Result<(), StoreError> {
        self.nvs.remove(KEY).map_err(StoreError::Nvs)?;
        Ok(())
    }
}

fn store_one(gear: &Gear) -> StoredGear {
    StoredGear {
        short: gear.spec.short_address,
        random: gear.spec.random_address,
        device_types: gear.spec.device_types.clone(),
        caps: (
            gear.spec.tc_capable,
            gear.spec.xy_capable,
            gear.spec.rgb_capable(),
        ),
        tc_range: (gear.spec.tc_coolest_mirek, gear.spec.tc_warmest_mirek),
        groups: gear.groups,
        scenes: gear.scenes,
        min_level: gear.min_level,
        max_level: gear.max_level,
        power_on_level: gear.power_on_level,
        system_failure_level: gear.system_failure_level,
        fade_time: gear.fade_time,
        fade_rate: gear.fade_rate,
        extended_fade_time: gear.extended_fade_time,
        enabled: gear.enabled,
    }
}

fn rebuild(stored: StoredFleet, reserved: u64) -> GearFleet {
    let specs: Vec<GearSpec> = stored
        .gears
        .iter()
        .map(|g| GearSpec {
            short_address: g.short,
            random_address: g.random,
            device_types: g.device_types.clone(),
            tc_capable: g.caps.0,
            xy_capable: g.caps.1,
            rgbwaf_channels: if g.caps.2 { DEFAULT_RGBWAF_CHANNELS } else { 0 },
            tc_coolest_mirek: g.tc_range.0,
            tc_warmest_mirek: g.tc_range.1,
            phm: 1,
            energy_reporting: g.device_types.contains(&DEVICE_TYPE_ENERGY),
            diagnostics_reporting: g.device_types.contains(&DEVICE_TYPE_DIAGNOSTICS),
            luminaire_format: g
                .device_types
                .contains(&DEVICE_TYPE_LUMINAIRE_INFO)
                .then_some(LuminaireFormat::V3),
            bus_unit_extension: false,
        })
        .collect();
    let mut fleet = GearFleet::new(specs, reserved, RESTORE_RNG_SEED);
    for (gear, saved) in fleet.gears_mut().iter_mut().zip(stored.gears.iter()) {
        restore_one(gear, saved);
    }
    fleet
}

const RESTORE_RNG_SEED: u32 = 0x5EED_1234;

fn restore_one(gear: &mut Gear, saved: &StoredGear) {
    gear.groups = saved.groups;
    gear.scenes = saved.scenes;
    gear.min_level = saved.min_level;
    gear.max_level = saved.max_level;
    gear.power_on_level = saved.power_on_level;
    gear.system_failure_level = saved.system_failure_level;
    gear.fade_time = saved.fade_time;
    gear.fade_rate = saved.fade_rate;
    gear.extended_fade_time = saved.extended_fade_time;
    gear.enabled = saved.enabled;
}

pub fn fingerprint(fleet: &GearFleet) -> u64 {
    let mut hash = FNV_OFFSET;
    for gear in fleet.gears() {
        feed(&mut hash, gear.spec.short_address.unwrap_or(0xFF) as u64);
        feed(&mut hash, gear.spec.short_address.is_some() as u64);
        feed(&mut hash, gear.spec.random_address as u64);
        feed(&mut hash, gear.groups as u64);
        for slot in gear.scenes {
            feed(&mut hash, slot as u64);
        }
        feed(&mut hash, gear.min_level as u64);
        feed(&mut hash, gear.max_level as u64);
        feed(&mut hash, gear.power_on_level as u64);
        feed(&mut hash, gear.system_failure_level as u64);
        feed(&mut hash, gear.fade_time as u64);
        feed(&mut hash, gear.fade_rate as u64);
        feed(&mut hash, gear.extended_fade_time as u64);
        feed(&mut hash, gear.enabled as u64);
    }
    hash
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn feed(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(FNV_PRIME);
}
