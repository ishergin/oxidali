use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};
use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};
use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};

use super::physical_devices::{MemoryBankRangeRecord, MemoryBankRecord};
use super::store::Inner;

pub(crate) const FLUSH_CHUNK_BYTES: usize = 1024;

struct SessionFlavor<'a, 'b> {
    session: &'a mut Box<dyn SliceWriteSession + 'b>,
    buf: &'a mut Vec<u8>,
    store_error: &'a mut Option<StoreError>,
}

impl SessionFlavor<'_, '_> {
    fn flush_chunk(&mut self) -> postcard::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.session.append(self.buf) {
            *self.store_error = Some(e);
            return Err(postcard::Error::SerializeBufferFull);
        }
        self.buf.clear();
        Ok(())
    }
}

impl postcard::ser_flavors::Flavor for SessionFlavor<'_, '_> {
    type Output = ();

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        if self.buf.len() >= FLUSH_CHUNK_BYTES {
            self.flush_chunk()?;
        }
        self.buf.push(data);
        Ok(())
    }

    fn try_extend(&mut self, mut data: &[u8]) -> postcard::Result<()> {
        while !data.is_empty() {
            let space = FLUSH_CHUNK_BYTES - self.buf.len();
            if space == 0 {
                self.flush_chunk()?;
                continue;
            }
            let take = space.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        Ok(())
    }

    fn finalize(mut self) -> postcard::Result<()> {
        self.flush_chunk()
    }
}

pub(crate) fn write_persistence_streaming<T: Serialize>(
    store: &dyn SliceStore,
    key: SliceKey,
    chunk_buf: &mut Vec<u8>,
    value: &T,
    slice_name: &str,
) -> Result<(), StoreError> {
    let mut session = store.begin_write(key)?;
    let mut store_error: Option<StoreError> = None;
    chunk_buf.clear();
    let flavor = SessionFlavor {
        session: &mut session,
        buf: chunk_buf,
        store_error: &mut store_error,
    };
    match postcard::serialize_with_flavor(value, flavor) {
        Ok(()) => session.commit(),
        Err(e) => {
            session.abort();
            Err(store_error.take().unwrap_or_else(|| {
                StoreError::Backend(format!("postcard encode {slice_name}: {e}"))
            }))
        }
    }
}

pub(crate) struct AdaptersSliceStream<'a> {
    pub inner: &'a Inner,
}

impl Serialize for AdaptersSliceStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableAdapterSlice", 1)?;
        st.serialize_field("adapters", &AdapterRowsStream { inner: self.inner })?;
        st.end()
    }
}

struct AdapterRowsStream<'a> {
    inner: &'a Inner,
}

impl Serialize for AdapterRowsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.inner.adapters.len()))?;
        for (i, row) in self.inner.adapters.iter().enumerate() {
            seq.serialize_element(&AdapterRowStream {
                adapter_id: i as u8,
                name: row.name.as_str(),
                enabled: row.enabled,
            })?;
        }
        seq.end()
    }
}

struct AdapterRowStream<'a> {
    adapter_id: u8,
    name: &'a str,
    enabled: bool,
}

impl Serialize for AdapterRowStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableAdapterRow", 3)?;
        st.serialize_field("adapter_id", &self.adapter_id)?;
        st.serialize_field("name", self.name)?;
        st.serialize_field("enabled", &self.enabled)?;
        st.end()
    }
}

pub(crate) struct GroupsSliceStream<'a> {
    pub inner: &'a Inner,
    pub adapter_id: u8,
}

impl Serialize for GroupsSliceStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableGroupsSlice", 3)?;
        st.serialize_field("adapter_id", &self.adapter_id)?;
        st.serialize_field(
            "groups",
            &GroupRecordsStream {
                inner: self.inner,
                adapter_id: self.adapter_id,
            },
        )?;
        st.serialize_field(
            "rows",
            &GroupMatrixRowsStream {
                inner: self.inner,
                adapter_id: self.adapter_id,
            },
        )?;
        st.end()
    }
}

struct GroupRecordsStream<'a> {
    inner: &'a Inner,
    adapter_id: u8,
}

impl Serialize for GroupRecordsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(GROUP_COUNT as usize))?;
        for group_id in 0..GROUP_COUNT {
            let record = self
                .inner
                .groups
                .get(&(self.adapter_id, group_id))
                .cloned()
                .unwrap_or_default();
            seq.serialize_element(&GroupRecordStream {
                group_id,
                name: record.name.as_str(),
                ha_entity_enabled: record.ha_entity_enabled,
            })?;
        }
        seq.end()
    }
}

struct GroupRecordStream<'a> {
    group_id: u8,
    name: &'a str,
    ha_entity_enabled: bool,
}

impl Serialize for GroupRecordStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableGroupRecord", 3)?;
        st.serialize_field("group_id", &self.group_id)?;
        st.serialize_field("name", self.name)?;
        st.serialize_field("ha_entity_enabled", &self.ha_entity_enabled)?;
        st.end()
    }
}

struct GroupMatrixRowsStream<'a> {
    inner: &'a Inner,
    adapter_id: u8,
}

impl Serialize for GroupMatrixRowsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(VIRTUAL_LAMP_COUNT as usize))?;
        for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
            let record = self
                .inner
                .group_matrix
                .get(&(self.adapter_id, virtual_lamp_id))
                .copied()
                .unwrap_or_default();
            seq.serialize_element(&GroupMatrixRowStream {
                virtual_lamp_id,
                desired_groups_mask: record.desired_groups_mask,
                desired_seeded: record.desired_seeded,
                desired_from_operator: record.desired_from_operator,
            })?;
        }
        seq.end()
    }
}

struct GroupMatrixRowStream {
    virtual_lamp_id: u8,
    desired_groups_mask: u16,
    desired_seeded: bool,
    desired_from_operator: bool,
}

impl Serialize for GroupMatrixRowStream {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableGroupMatrixRow", 4)?;
        st.serialize_field("virtual_lamp_id", &self.virtual_lamp_id)?;
        st.serialize_field("desired_groups_mask", &self.desired_groups_mask)?;
        st.serialize_field("desired_seeded", &self.desired_seeded)?;
        st.serialize_field("desired_from_operator", &self.desired_from_operator)?;
        st.end()
    }
}

pub(crate) struct SceneSliceStream<'a> {
    pub inner: &'a Inner,
    pub adapter_id: u8,
    pub scene_id: u8,
}

impl Serialize for SceneSliceStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let record = self
            .inner
            .scenes
            .get(&(self.adapter_id, self.scene_id))
            .cloned()
            .unwrap_or_default();
        let mut st = serializer.serialize_struct("PersistableSceneSlice", 5)?;
        st.serialize_field("adapter_id", &self.adapter_id)?;
        st.serialize_field("scene_id", &self.scene_id)?;
        st.serialize_field("name", record.name.as_str())?;
        st.serialize_field("ha_select_enabled", &record.ha_select_enabled)?;
        st.serialize_field(
            "rows",
            &SceneRowsStream {
                inner: self.inner,
                adapter_id: self.adapter_id,
                scene_id: self.scene_id,
            },
        )?;
        st.end()
    }
}

struct SceneRowsStream<'a> {
    inner: &'a Inner,
    adapter_id: u8,
    scene_id: u8,
}

impl Serialize for SceneRowsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(VIRTUAL_LAMP_COUNT as usize))?;
        for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
            let record = self
                .inner
                .scene_matrix
                .get(&(self.adapter_id, self.scene_id, virtual_lamp_id))
                .copied()
                .unwrap_or_default();
            seq.serialize_element(&SceneRowStream {
                virtual_lamp_id,
                record,
            })?;
        }
        seq.end()
    }
}

struct SceneRowStream {
    virtual_lamp_id: u8,
    record: crate::runtime::registry::scenes::SceneDesiredRowRecord,
}

impl Serialize for SceneRowStream {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let target = self.record.target;
        let mut st = serializer.serialize_struct("PersistableSceneRow", 7)?;
        st.serialize_field("virtual_lamp_id", &self.virtual_lamp_id)?;
        st.serialize_field("included", &self.record.included)?;
        st.serialize_field("desired_seeded", &self.record.desired_seeded)?;
        st.serialize_field("desired_from_operator", &self.record.desired_from_operator)?;
        st.serialize_field("power", &target.and_then(|t| t.power))?;
        st.serialize_field("level", &target.and_then(|t| t.level))?;
        st.serialize_field("color", &target.and_then(|t| t.color))?;
        st.end()
    }
}

pub(crate) struct VirtualLampsSliceStream<'a> {
    pub inner: &'a Inner,
    pub adapter_id: u8,
}

impl Serialize for VirtualLampsSliceStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistableVirtualLampsSlice", 2)?;
        st.serialize_field("adapter_id", &self.adapter_id)?;
        st.serialize_field(
            "lamps",
            &VlRecordsStream {
                inner: self.inner,
                adapter_id: self.adapter_id,
            },
        )?;
        st.end()
    }
}

struct VlRecordsStream<'a> {
    inner: &'a Inner,
    adapter_id: u8,
}

impl Serialize for VlRecordsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ids = sorted_ids(self.inner.lamps.keys(), self.adapter_id);
        let mut seq = serializer.serialize_seq(Some(ids.len()))?;
        for vlid in ids {
            let r = &self.inner.lamps[&(self.adapter_id, vlid)];
            seq.serialize_element(&VlRecordStream {
                virtual_lamp_id: vlid,
                record: r,
            })?;
        }
        seq.end()
    }
}

struct VlRecordStream<'a> {
    virtual_lamp_id: u8,
    record: &'a super::virtual_lamps::VlRecord,
}

impl Serialize for VlRecordStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let r = self.record;
        let mut st = serializer.serialize_struct("PersistableVlRecord", 4)?;
        st.serialize_field("virtual_lamp_id", &self.virtual_lamp_id)?;
        st.serialize_field("name", r.name.as_str())?;
        st.serialize_field("ha_entity_enabled", &r.ha_entity_enabled)?;
        st.serialize_field("binding_short", &r.binding_short)?;
        st.end()
    }
}

pub(crate) struct PhysicalDevicesSliceStream<'a> {
    pub inner: &'a Inner,
    pub adapter_id: u8,
    pub bank: Option<u8>,
}

impl Serialize for PhysicalDevicesSliceStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("PersistablePhysicalDevicesSlice", 2)?;
        st.serialize_field("adapter_id", &self.adapter_id)?;
        st.serialize_field(
            "devices",
            &PdRecordsStream {
                inner: self.inner,
                adapter_id: self.adapter_id,
                bank: self.bank,
            },
        )?;
        st.end()
    }
}

struct PdRecordsStream<'a> {
    inner: &'a Inner,
    adapter_id: u8,
    bank: Option<u8>,
}

impl Serialize for PdRecordsStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let per_bank = dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK;
        let mut shorts = sorted_ids(self.inner.physical_devices.keys(), self.adapter_id);
        if let Some(bank) = self.bank {
            shorts.retain(|sa| sa / per_bank == bank);
        }
        let mut seq = serializer.serialize_seq(Some(shorts.len()))?;
        for sa in shorts {
            let r = &self.inner.physical_devices[&(self.adapter_id, sa)];
            seq.serialize_element(&PdRecordStream {
                short_address: sa,
                record: r,
            })?;
        }
        seq.end()
    }
}

struct PdRecordStream<'a> {
    short_address: u8,
    record: &'a super::physical_devices::PhysicalDeviceRecord,
}

impl Serialize for PdRecordStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let r = self.record;
        let mut st = serializer.serialize_struct("PersistablePhysicalDeviceRecord", 19)?;
        st.serialize_field("short_address", &self.short_address)?;
        st.serialize_field("random_address", &r.random_address)?;
        st.serialize_field("name", r.name.as_str())?;
        st.serialize_field("notes", &r.notes.as_ref().map(|n| n.as_str()))?;
        st.serialize_field(
            "device_type_override",
            &r.device_type_override,
        )?;
        st.serialize_field(
            "color_mode_override",
            &r.color_mode_override,
        )?;
        st.serialize_field("attributes", &r.attributes)?;
        st.serialize_field("memory_banks", &MemoryBanksStream(&r.memory_banks))?;
        st.serialize_field("device_type_discovered", &r.device_type_discovered)?;
        st.serialize_field("color_mode_discovered", &r.color_mode_discovered)?;
        st.serialize_field("cap_cct", &r.cap_cct)?;
        st.serialize_field("cap_xy", &r.cap_xy)?;
        st.serialize_field("cap_rgb", &r.cap_rgb)?;
        st.serialize_field("tc_coolest_mirek", &r.tc_coolest_mirek)?;
        st.serialize_field("tc_warmest_mirek", &r.tc_warmest_mirek)?;
        st.serialize_field("dt8_auto_activation_repair", &r.dt8_auto_activation_repair)?;
        st.serialize_field("dt8_rgbwaf_control_assert", &r.dt8_rgbwaf_control_assert)?;
        st.serialize_field("cap_rgbwaf", &r.cap_rgbwaf)?;
        st.serialize_field("supported_device_types", &r.supported_device_types)?;
        st.end()
    }
}

struct MemoryBanksStream<'a>(&'a [MemoryBankRecord]);

impl Serialize for MemoryBanksStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for bank in self.0 {
            seq.serialize_element(&MemoryBankStream(bank))?;
        }
        seq.end()
    }
}

struct MemoryBankStream<'a>(&'a MemoryBankRecord);

impl Serialize for MemoryBankStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("MemoryBankSummaryView", 4)?;
        st.serialize_field("bank", &self.0.bank)?;
        st.serialize_field("total_bytes_read", &self.0.total_bytes_read)?;
        st.serialize_field("last_read_ms", &self.0.last_read_ms)?;
        st.serialize_field("ranges", &MemoryBankRangesStream(&self.0.ranges))?;
        st.end()
    }
}

struct MemoryBankRangesStream<'a>(&'a [MemoryBankRangeRecord]);

impl Serialize for MemoryBankRangesStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for range in self.0 {
            seq.serialize_element(&MemoryBankRangeStream(range))?;
        }
        seq.end()
    }
}

struct MemoryBankRangeStream<'a>(&'a MemoryBankRangeRecord);

impl Serialize for MemoryBankRangeStream<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("MemoryBankRangeView", 2)?;
        st.serialize_field("start", &self.0.start)?;
        st.serialize_field("length", &self.0.length)?;
        st.end()
    }
}

fn sorted_ids<'a>(keys: impl Iterator<Item = &'a (u8, u8)>, adapter_id: u8) -> Vec<u8> {
    let mut ids: Vec<u8> = keys
        .filter(|(aid, _)| *aid == adapter_id)
        .map(|(_, id)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

#[cfg(test)]
mod tests {
    use dali2rust_bsp::psram::PsramBox;

    use super::super::groups::{GroupMembershipRowRecord, GroupRecord};
    use super::super::persistence_slices::{
        encode_persistence_blob, PersistableAdapterRow, PersistableAdapterSlice,
        PersistableGroupMatrixRow, PersistableGroupRecord, PersistableGroupsSlice,
        PersistablePhysicalDeviceRecord, PersistablePhysicalDevicesSlice, PersistableSceneRow,
        PersistableSceneSlice, PersistableVirtualLampsSlice, PersistableVlRecord,
        PersistenceEnvelope, ADAPTERS_SLICE_VERSION, GROUPS_SLICE_VERSION,
        PHYSICAL_DEVICES_SLICE_VERSION, SCENES_SLICE_VERSION, VIRTUAL_LAMPS_SLICE_VERSION,
    };
    use super::super::scenes::{SceneDesiredRowRecord, SceneRecord};
    use super::super::physical_devices::{
        MemoryBankRangeRecord, MemoryBankRecord, PhysicalDeviceRecord,
    };
    use super::super::store::RegistryStore;
    use crate::test_support::{CountingStore, ProbeStore};
    use super::super::virtual_lamps::VlRecord;
    use super::FLUSH_CHUNK_BYTES;
    use dali2rust_contracts::msg::{fixed_text_64, ColorMode, DeviceType, DeviceTypeSet};
    use dali2rust_domain::registry::{
        AttributeSource, MemoryBankRangeView, MemoryBankSummaryView, ObservedValue,
        PhysicalDeviceAttributesView, SceneReadPort,
    };
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};
    use std::sync::atomic::Ordering;

    fn observed<T>(value: T, ms: u64) -> Option<ObservedValue<T>> {
        Some(ObservedValue {
            value,
            source: AttributeSource::Readback,
            last_read_ms: Some(ms),
            last_write_confirmed_ms: None,
        })
    }

    fn fat_attributes(seed: u8) -> PhysicalDeviceAttributesView {
        let mut a = PhysicalDeviceAttributesView::default();
        let ms = u64::from(seed) * 1000;
        a.common102.version = observed(8, ms);
        a.common102.min_level = observed(seed, ms);
        a.common102.max_level = observed(254, ms);
        a.common102.fade_time_ms = observed(u32::from(seed) * 3, ms);
        a.groups.membership = observed(u16::from(seed), ms);
        for (i, level) in a.scenes.levels.iter_mut().enumerate() {
            *level = observed(i as u8, ms);
        }
        a.dt6_led.operating_mode = observed(1, ms);
        a.dt6_led.failure_status = observed(seed, ms);
        a.dt8_color.color_type = observed(0x20, ms);
        a.dt8_color.color_value_0 = observed(u16::from(seed) * 7, ms);
        a.extended.version_number = observed(2, ms);
        a.memory_identity.gtin = observed(4_012_345_000_000 + u64::from(seed), ms);
        a.memory_identity.firmware_version_major = observed(1, ms);
        a.memory_profile.oem_gtin = observed(9_000_000_000_000 + u64::from(seed), ms);
        a
    }

    fn fat_pd_record(seed: u8) -> PhysicalDeviceRecord {
        let mut r = PhysicalDeviceRecord::empty(seed);
        r.name = fixed_text_64(&format!("Luminaire hall corridor {seed:02}"));
        r.notes = Some(fixed_text_64("CCT downlight, maintenance loop B"));
        r.random_address = Some(0x00AB_0000 + u32::from(seed));
        r.device_type_override = Some(DeviceType::Dt6Led);
        r.color_mode_override = Some(ColorMode::Cct);
        r.supported_device_types = Some(DeviceTypeSet::from_bits(
            (1 << 6) | (1 << 8) | (1 << 52),
        ));
        r.attributes = fat_attributes(seed);
        r.memory_banks = vec![MemoryBankRecord {
            bank: 0,
            total_bytes_read: 64,
            last_read_ms: u64::from(seed) * 11,
            ranges: vec![
                MemoryBankRangeRecord {
                    start: 0,
                    length: 32,
                },
                MemoryBankRangeRecord {
                    start: 32,
                    length: 32,
                },
            ],
            bytes: vec![0xAA; 8],
        }];
        r
    }

    fn populated_store(device_count: u8) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(2);
        let mut g = store.inner.write().expect("registry lock");
        g.adapters[0].name = fixed_text_64("Hall A");
        g.adapters[0].enabled = false;
        for seed in (0..device_count).rev() {
            g.physical_devices
                .insert((0, seed), PsramBox::new(fat_pd_record(seed)));
            let vl = VlRecord {
                name: fixed_text_64(&format!("Lamp {seed}")),
                ha_entity_enabled: seed % 2 == 0,
                binding_short: Some(seed),
            };
            g.lamps.insert((0, seed), vl);
        }
        g.physical_devices.insert((1, 7), PsramBox::new(fat_pd_record(7)));
        g.lamps.insert((1, 7), VlRecord::default());
        for group_id in 0..12u8 {
            g.groups.insert(
                (0, group_id),
                GroupRecord {
                    name: fixed_text_64(&format!("Zone {group_id}")),
                    ha_entity_enabled: group_id % 3 == 0,
                },
            );
        }
        for vlid in 0..40u8 {
            g.group_matrix.insert(
                (0, vlid),
                GroupMembershipRowRecord {
                    desired_groups_mask: u16::from(vlid) << 1,
                    desired_seeded: vlid % 2 == 1,
                    desired_from_operator: vlid % 3 == 0,
                },
            );
        }
        g.scenes.insert(
            (0, 3),
            SceneRecord {
                name: fixed_text_64("Evening"),
                ha_select_enabled: false,
            },
        );
        for vlid in 0..10u8 {
            let included = vlid % 2 == 0;
            g.scene_matrix.insert(
                (0, 3, vlid),
                SceneDesiredRowRecord {
                    included,
                    target: included.then_some(dali2rust_contracts::msg::DaliSceneTargetState {
                        power: Some(dali2rust_contracts::msg::PowerState::On),
                        level: Some(100 + vlid),
                        color: (vlid % 4 == 0).then_some(dali2rust_contracts::msg::ColorValue {
                            mode: dali2rust_contracts::msg::ColorMode::Rgbwaf,
                            r: 10 + vlid,
                            g: 20 + vlid,
                            b: 30 + vlid,
                            w: 40 + vlid,
                            a: 50 + vlid,
                            f: 60 + vlid,
                            ..Default::default()
                        }),
                    }),
                    desired_seeded: true,
                    desired_from_operator: vlid % 3 == 0,
                },
            );
        }
        drop(g);
        store
    }

    fn expected_pd_blob(store: &RegistryStore, adapter_id: u8, bank: Option<u8>) -> Vec<u8> {
        let per_bank = dali2rust_platform::slice_store::SliceKey::DEVICES_PER_BANK;
        expected_pd_blob_where(store, adapter_id, move |sa| {
            bank.is_none_or(|b| sa / per_bank == b)
        })
    }

    fn expected_pd_blob_for_shorts(
        store: &RegistryStore,
        adapter_id: u8,
        shorts: std::ops::Range<u8>,
    ) -> Vec<u8> {
        expected_pd_blob_where(store, adapter_id, move |sa| shorts.contains(&sa))
    }

    fn expected_pd_blob_where(
        store: &RegistryStore,
        adapter_id: u8,
        keep: impl Fn(u8) -> bool,
    ) -> Vec<u8> {
        let g = store.inner.read().expect("registry lock");
        let mut devices: Vec<PersistablePhysicalDeviceRecord> = g
            .physical_devices
            .iter()
            .filter(|((aid, sa), _)| *aid == adapter_id && keep(*sa))
            .map(|((_, sa), r)| PersistablePhysicalDeviceRecord {
                short_address: *sa,
                random_address: r.random_address,
                name: r.name.as_str().to_string(),
                notes: r.notes.as_ref().map(|n| n.as_str().to_string()),
                device_type_override: r.device_type_override,
                color_mode_override: r.color_mode_override,
                device_type_discovered: r.device_type_discovered,
                color_mode_discovered: r.color_mode_discovered,
                cap_cct: r.cap_cct,
                cap_xy: r.cap_xy,
                cap_rgb: r.cap_rgb,
                tc_coolest_mirek: r.tc_coolest_mirek,
                tc_warmest_mirek: r.tc_warmest_mirek,
                dt8_auto_activation_repair: r.dt8_auto_activation_repair,
                dt8_rgbwaf_control_assert: r.dt8_rgbwaf_control_assert,
                cap_rgbwaf: r.cap_rgbwaf,
                supported_device_types: r.supported_device_types,
                attributes: r.attributes.clone(),
                memory_banks: r
                    .memory_banks
                    .iter()
                    .map(|bank| MemoryBankSummaryView {
                        bank: bank.bank,
                        total_bytes_read: bank.total_bytes_read,
                        last_read_ms: bank.last_read_ms,
                        ranges: bank
                            .ranges
                            .iter()
                            .map(|range| MemoryBankRangeView {
                                start: range.start,
                                length: range.length,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        devices.sort_by_key(|d| d.short_address);
        encode_persistence_blob(&PersistenceEnvelope::new(PHYSICAL_DEVICES_SLICE_VERSION, PersistablePhysicalDevicesSlice {
            adapter_id,
            devices,
        }))
        .expect("encode expected pd blob")
    }

    fn expected_vl_blob(store: &RegistryStore, adapter_id: u8) -> Vec<u8> {
        let g = store.inner.read().expect("registry lock");
        let mut lamps: Vec<PersistableVlRecord> = g
            .lamps
            .iter()
            .filter(|((aid, _), _)| *aid == adapter_id)
            .map(|((_, vlid), r)| PersistableVlRecord {
                virtual_lamp_id: *vlid,
                name: r.name.as_str().to_string(),
                ha_entity_enabled: r.ha_entity_enabled,
                binding_short: r.binding_short,
            })
            .collect();
        lamps.sort_by_key(|l| l.virtual_lamp_id);
        encode_persistence_blob(&PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, PersistableVirtualLampsSlice {
            adapter_id,
            lamps,
        }))
        .expect("encode expected vl blob")
    }

    fn expected_groups_blob(store: &RegistryStore, adapter_id: u8) -> Vec<u8> {
        let g = store.inner.read().expect("registry lock");
        let groups = (0..16)
            .map(|group_id| {
                let record = g
                    .groups
                    .get(&(adapter_id, group_id))
                    .cloned()
                    .unwrap_or_default();
                PersistableGroupRecord {
                    group_id,
                    name: record.name.as_str().to_string(),
                    ha_entity_enabled: record.ha_entity_enabled,
                }
            })
            .collect();
        let rows = (0..64)
            .map(|virtual_lamp_id| {
                let record = g
                    .group_matrix
                    .get(&(adapter_id, virtual_lamp_id))
                    .copied()
                    .unwrap_or_default();
                PersistableGroupMatrixRow {
                    virtual_lamp_id,
                    desired_groups_mask: record.desired_groups_mask,
                    desired_seeded: record.desired_seeded,
                    desired_from_operator: record.desired_from_operator,
                }
            })
            .collect();
        encode_persistence_blob(&PersistenceEnvelope::new(GROUPS_SLICE_VERSION, PersistableGroupsSlice {
            adapter_id,
            groups,
            rows,
        }))
        .expect("encode expected groups blob")
    }

    fn expected_adapters_blob(store: &RegistryStore) -> Vec<u8> {
        let g = store.inner.read().expect("registry lock");
        let adapters = g
            .adapters
            .iter()
            .enumerate()
            .map(|(i, r)| PersistableAdapterRow {
                adapter_id: i as u8,
                name: r.name.as_str().to_string(),
                enabled: r.enabled,
            })
            .collect();
        encode_persistence_blob(&PersistenceEnvelope::new(ADAPTERS_SLICE_VERSION, PersistableAdapterSlice {
            adapters,
        }))
        .expect("encode expected adapters blob")
    }

    fn expected_scene_blob(store: &RegistryStore, adapter_id: u8, scene_id: u8) -> Vec<u8> {
        let g = store.inner.read().expect("registry lock");
        let record = g
            .scenes
            .get(&(adapter_id, scene_id))
            .cloned()
            .unwrap_or_default();
        let rows = (0..64)
            .map(|virtual_lamp_id| {
                let row = g
                    .scene_matrix
                    .get(&(adapter_id, scene_id, virtual_lamp_id))
                    .copied()
                    .unwrap_or_default();
                PersistableSceneRow {
                    virtual_lamp_id,
                    included: row.included,
                    desired_seeded: row.desired_seeded,
                    desired_from_operator: row.desired_from_operator,
                    power: row.target.and_then(|t| t.power),
                    level: row.target.and_then(|t| t.level),
                    color: row.target.and_then(|t| t.color),
                }
            })
            .collect();
        encode_persistence_blob(&PersistenceEnvelope::new(SCENES_SLICE_VERSION, PersistableSceneSlice {
            adapter_id,
            scene_id,
            name: record.name.as_str().to_string(),
            ha_select_enabled: record.ha_select_enabled,
            rows,
        }))
        .expect("encode expected scene blob")
    }

    #[test]
    fn streamed_scene_flush_is_byte_identical_and_roundtrips() {
        let store = populated_store(4);
        let slices = InMemorySliceStore::new();
        store.dirty.mark_scene_dirty(0, 3);
        store.flush_dirty_slices(&slices);

        let blob = slices.load(SliceKey::Scene { adapter_id: 0, scene_id: 3 }).expect("scene blob");
        assert_eq!(blob, expected_scene_blob(&store, 0, 3));

        let restored = RegistryStore::with_adapter_count(2);
        let report = restored.hydrate_from_store(&slices, 2);
        assert!(report.is_ok(), "errors={:?}", report.errors);
        let view = restored.scene_view(0, 3).expect("scene view");
        assert_eq!(view.name, "Evening");
        assert!(!view.ha_select_enabled);
        assert_eq!(view.row_count_included, 5);
        let matrix = restored.scene_matrix_view(0, 3).expect("scene matrix");
        assert_eq!(matrix.rows[2].desired.level, Some(102));
        assert_eq!(matrix.rows[0].desired.color_mode.as_deref(), Some("rgbwaf"));
        assert_eq!(matrix.rows[0].desired.rgb, Some((10, 20, 30)));
        assert_eq!(matrix.rows[0].desired.waf, Some((40, 50, 60)));
        assert!(!matrix.rows[1].desired.included);
    }

    #[test]
    fn streamed_flush_is_byte_identical_to_materialized_encoding() {
        let store = populated_store(16);
        let slices = InMemorySliceStore::new();
        store.dirty.adapters.store(true, std::sync::atomic::Ordering::Release);
        store.dirty.mark_groups_dirty(0);
        store.dirty.mark_virtual_lamps_dirty(0);
        store.dirty.mark_all_physical_devices_dirty(0);
        store.flush_dirty_slices(&slices);

        let pd = slices
            .load(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank: 0 })
            .expect("pd bank blob");
        assert_eq!(pd, expected_pd_blob(&store, 0, Some(0)));
        assert!(
            pd.len() > FLUSH_CHUNK_BYTES,
            "fixture must exercise multi-chunk streaming, got {} bytes",
            pd.len()
        );
        let pd1 = slices
            .load(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank: 1 })
            .expect("pd bank 1 blob");
        assert_eq!(pd1, expected_pd_blob(&store, 0, Some(1)));
        assert_ne!(pd, pd1);
        assert_eq!(
            slices.load(SliceKey::VirtualLamps { adapter_id: 0 }).expect("vl blob"),
            expected_vl_blob(&store, 0)
        );
        assert_eq!(
            slices.load(SliceKey::Groups { adapter_id: 0 }).expect("groups blob"),
            expected_groups_blob(&store, 0)
        );
        assert_eq!(
            slices.load(SliceKey::Adapters).expect("adapters blob"),
            expected_adapters_blob(&store)
        );
    }

    #[test]
    fn streamed_flush_roundtrips_through_hydrate() {
        let store = populated_store(4);
        let slices = InMemorySliceStore::new();
        store.dirty.mark_physical_devices_dirty(0);
        store.dirty.mark_virtual_lamps_dirty(0);
        store.flush_dirty_slices(&slices);

        let restored = RegistryStore::with_adapter_count(2);
        let report = restored.hydrate_from_store(&slices, 2);
        assert!(report.is_ok(), "errors={:?}", report.errors);
        let view = restored
            .physical_device_view(0, 3)
            .expect("device 3 hydrates");
        assert_eq!(view.random_address, Some(0x00AB_0003));
        assert_eq!(view.name, "Luminaire hall corridor 03");
        assert_eq!(view.attributes, fat_attributes(3));
    }

    #[test]
    fn one_dirty_device_writes_one_banks_worth_of_bytes() {
        let store = populated_store(64);

        let all = CountingStore::default();
        store.dirty.mark_all_physical_devices_dirty(0);
        store.flush_dirty_slices(&all);
        let whole_segment = all.bytes_written();
        assert!(
            whole_segment > 20_000,
            "the fixture must be a real segment, got {whole_segment} B"
        );

        let one = CountingStore::default();
        store.dirty.mark_physical_device_dirty(0, 3);
        store.flush_dirty_slices(&one);
        let one_device = one.bytes_written();

        assert_eq!(one.write_count(), 1, "one device, one slice");
        assert!(
            one_device * 8 < whole_segment,
            "one device wrote {one_device} B of the segment's {whole_segment} B — \
             the ratio the bench measured at 1,00 is the defect"
        );

        let three = CountingStore::default();
        for sa in [0u8, 2, 3] {
            store.dirty.mark_physical_device_dirty(0, sa);
        }
        store.flush_dirty_slices(&three);
        assert_eq!(three.write_count(), 1);
        assert_eq!(three.bytes_written(), one_device);

        let spread = CountingStore::default();
        for sa in [0u8, 4, 8] {
            store.dirty.mark_physical_device_dirty(0, sa);
        }
        store.flush_dirty_slices(&spread);
        assert_eq!(spread.write_count(), 3);
    }

    #[test]
    fn a_legacy_whole_adapter_slice_hydrates_once_and_is_rewritten_as_banks() {
        let legacy = InMemorySliceStore::new();
        {
            let old = populated_store(16);
            let mut session = legacy
                .begin_write(SliceKey::PhysicalDevices { adapter_id: 0 })
                .expect("begin");
            session.append(&expected_pd_blob(&old, 0, None)).expect("append");
            session.commit().expect("commit");
        }

        let fresh = RegistryStore::with_adapter_count(1);
        let counts = fresh.hydrate_from_store(&legacy, 1);
        assert_eq!(counts.errors.iter().count(), 0, "the legacy slice must read cleanly");
        {
            let g = fresh.inner.read().expect("registry lock");
            assert_eq!(
                g.physical_devices.keys().filter(|(aid, _)| *aid == 0).count(),
                16,
                "every device must survive the migration — an empty PD slice \
                 orphans every binding at once"
            );
        }

        let after = CountingStore::default();
        fresh.flush_dirty_slices(&after);
        let labels = after.inner.labels();
        assert!(
            labels.contains(&"a0/physical_devices_b0".to_string())
                && labels.contains(&"a0/physical_devices_b1".to_string()),
            "the banks must be written after the migration: {labels:?}"
        );
    }

    #[test]
    fn a_bank_holding_a_foreign_address_is_rejected_and_the_fallback_catches_it() {
        let store = InMemorySliceStore::new();
        let old = populated_store(16);

        {
            let mut session = store
                .begin_write(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank: 2 })
                .expect("begin");
            session
                .append(&expected_pd_blob_for_shorts(&old, 0, 8..16))
                .expect("append");
            session.commit().expect("commit");
        }
        {
            let mut session = store
                .begin_write(SliceKey::PhysicalDevices { adapter_id: 0 })
                .expect("begin");
            session.append(&expected_pd_blob(&old, 0, None)).expect("append");
            session.commit().expect("commit");
        }

        let fresh = RegistryStore::with_adapter_count(1);
        fresh.hydrate_from_store(&store, 1);
        let g = fresh.inner.read().expect("registry lock");
        assert_eq!(
            g.physical_devices.keys().filter(|(aid, _)| *aid == 0).count(),
            16,
            "the mismatched bank must be refused and the whole-adapter slice \
             must supply everything — loading it would have kept 8..=15 and \
             dropped 0..=7 without a word"
        );
    }

    #[test]
    fn written_empty_banks_keep_the_whole_adapter_slice_out() {
        let store = InMemorySliceStore::new();
        write_banks_except(&store, &RegistryStore::with_adapter_count(1), None);
        write_slot(&store, LEGACY_SLOT, &legacy_blob());

        assert_eq!(
            device_count(&boot(&store)),
            0,
            "a bank that loads is authoritative even when empty: these are the banks a \
             forget-all writes, and the whole-adapter slice must not bring the devices back"
        );
    }

    const MIGRATED_DEVICES: u8 = 16;
    const OLD_SLOT_DEVICES: u8 = 20;
    const LEGACY_SLOT: SliceKey = SliceKey::PhysicalDevices { adapter_id: 0 };
    const REFUSED_BANK: u8 = 3;
    const BROKEN_BANK: u8 = 2;
    const UNWRITTEN_BANK: u8 = 1;
    const FORGOTTEN_SHORTS: [u8; 2] = [5, 7];
    const STALE_NAME: &str = "stale record";
    const EMPTIED_BANK: u8 = 1;
    const BOUND_LAMP: u8 = 5;
    const SCANNED_SHORT: u8 = 9;
    const SCANNED_RANDOM_ADDRESS: u32 = 0x0012_3456;

    fn bank_shorts(bank: u8) -> std::ops::Range<u8> {
        bank * SliceKey::DEVICES_PER_BANK..(bank + 1) * SliceKey::DEVICES_PER_BANK
    }

    fn random_address_of(store: &RegistryStore, short_address: u8) -> Option<u32> {
        let g = store.inner.read().expect("registry lock");
        g.physical_devices.get(&(0, short_address)).and_then(|r| r.random_address)
    }

    fn binding_of(store: &RegistryStore, virtual_lamp_id: u8) -> Option<u8> {
        let g = store.inner.read().expect("registry lock");
        g.lamps.get(&(0, virtual_lamp_id)).and_then(|lamp| lamp.binding_short)
    }

    fn bank_slot(bank: u8) -> SliceKey {
        SliceKey::PhysicalDeviceBank { adapter_id: 0, bank }
    }

    fn write_banks_except(slices: &dyn SliceStore, source: &RegistryStore, skipped: Option<u8>) {
        for bank in (0..SliceKey::PHYSICAL_DEVICE_BANKS).filter(|bank| Some(*bank) != skipped) {
            write_slot(slices, bank_slot(bank), &expected_pd_blob(source, 0, Some(bank)));
        }
    }

    fn stale_legacy_blob() -> Vec<u8> {
        let stale = populated_store(MIGRATED_DEVICES);
        for record in stale.inner.write().expect("registry lock").physical_devices.values_mut() {
            record.name = fixed_text_64(STALE_NAME);
        }
        expected_pd_blob(&stale, 0, None)
    }

    fn device_name(store: &RegistryStore, short_address: u8) -> Option<String> {
        let g = store.inner.read().expect("registry lock");
        g.physical_devices
            .get(&(0, short_address))
            .map(|r| r.name.as_str().to_string())
    }

    fn write_slot(slices: &dyn SliceStore, key: SliceKey, blob: &[u8]) {
        let mut session = slices.begin_write(key).expect("begin");
        session.append(blob).expect("append");
        session.commit().expect("commit");
    }

    fn legacy_blob() -> Vec<u8> {
        expected_pd_blob(&populated_store(MIGRATED_DEVICES), 0, None)
    }

    fn device_count(store: &RegistryStore) -> usize {
        let g = store.inner.read().expect("registry lock");
        g.physical_devices.keys().filter(|(aid, _)| *aid == 0).count()
    }

    fn boot(slices: &dyn SliceStore) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        store.hydrate_from_store(slices, 1);
        store
    }

    fn boot_and_flush(slices: &dyn SliceStore) -> RegistryStore {
        let store = boot(slices);
        store.flush_dirty_slices(slices);
        store
    }

    fn forget_every_device(store: &RegistryStore, slices: &dyn SliceStore) {
        for short in 0..MIGRATED_DEVICES {
            store.apply_physical_device_forget(0, short).expect("a migrated device");
        }
        store.flush_dirty_slices(slices);
    }

    #[test]
    fn forgetting_every_migrated_device_leaves_the_adapter_empty_after_a_reboot() {
        let slices = InMemorySliceStore::new();
        write_slot(&slices, LEGACY_SLOT, &legacy_blob());
        let migrated = boot_and_flush(&slices);
        assert_eq!(device_count(&migrated), usize::from(MIGRATED_DEVICES));

        forget_every_device(&migrated, &slices);
        assert_eq!(slices.load(LEGACY_SLOT).expect("slot"), legacy_blob(), "never written");
        assert_eq!(
            device_count(&boot(&slices)),
            0,
            "the whole-adapter slot brought the forgotten devices back"
        );
    }

    #[test]
    fn a_bank_lost_before_a_reboot_comes_back_from_the_whole_adapter_slot() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        slices.refuse_writes_to(Some(bank_slot(REFUSED_BANK)));
        drop(boot_and_flush(&slices));
        assert!(matches!(slices.inner.load(bank_slot(REFUSED_BANK)), Err(StoreError::Missing)));

        slices.refuse_writes_to(None);
        assert_eq!(device_count(&boot_and_flush(&slices)), usize::from(MIGRATED_DEVICES));
        let settled = slices.costs();
        assert_eq!(device_count(&boot_and_flush(&slices)), usize::from(MIGRATED_DEVICES));
        assert_eq!(slices.costs(), settled, "once bank {REFUSED_BANK} is written, a boot costs nothing");
    }

    #[test]
    fn a_written_bank_keeps_its_forgotten_device_out_while_a_lost_bank_falls_back() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        slices.refuse_writes_to(Some(bank_slot(REFUSED_BANK)));
        let first = boot_and_flush(&slices);
        first.apply_physical_device_forget(0, 0).expect("device 0");
        first.flush_dirty_slices(&slices);
        drop(first);

        slices.refuse_writes_to(None);
        let rebooted = boot(&slices);
        assert_eq!(device_count(&rebooted), usize::from(MIGRATED_DEVICES) - 1);
        let g = rebooted.inner.read().expect("registry lock");
        assert!(!g.physical_devices.contains_key(&(0, 0)), "bank 0 was written without it");
    }

    #[test]
    fn a_migrated_adapter_costs_no_writes_and_no_whole_adapter_read_on_later_boots() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        boot_and_flush(&slices);

        let settled = slices.costs();
        for _ in 0..2 {
            assert_eq!(device_count(&boot_and_flush(&slices)), usize::from(MIGRATED_DEVICES));
        }
        assert_eq!(slices.costs(), settled, "(writes, whole-adapter reads) moved");
    }

    #[test]
    fn an_undecodable_whole_adapter_slot_costs_one_round_of_bank_writes() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, b"not a persistence envelope");
        let first = RegistryStore::with_adapter_count(1);
        assert_eq!(first.hydrate_from_store(&slices, 1).errors.iter().count(), 1);
        first.flush_dirty_slices(&slices);
        assert!(slices.inner.load(bank_slot(0)).is_ok(), "the banks are written once");

        let settled = slices.costs();
        let second = RegistryStore::with_adapter_count(1);
        assert!(second.hydrate_from_store(&slices, 1).is_ok());
        second.flush_dirty_slices(&slices);
        assert_eq!(slices.costs(), settled, "the undecodable slot still costs every boot");
    }

    #[test]
    fn a_whole_adapter_slot_beside_written_banks_is_ignored_and_never_written() {
        let slices = ProbeStore::default();
        write_banks_except(&slices.inner, &populated_store(MIGRATED_DEVICES), None);
        let pulled = expected_pd_blob(&populated_store(OLD_SLOT_DEVICES), 0, None);
        write_slot(&slices.inner, LEGACY_SLOT, &pulled);

        let booted = boot_and_flush(&slices);
        assert_eq!(device_count(&booted), usize::from(MIGRATED_DEVICES));
        assert_eq!(slices.inner.load(LEGACY_SLOT).expect("slot"), pulled, "its CRC must hold");
        assert_eq!(slices.costs().1, 0, "the whole-adapter slot was read beside complete banks");
    }

    #[test]
    fn a_whole_adapter_slot_that_fails_to_read_is_kept_for_the_next_boot() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        slices.fail_reads_of(Some(LEGACY_SLOT));
        assert_eq!(device_count(&boot_and_flush(&slices)), 0);

        slices.fail_reads_of(None);
        assert_eq!(
            device_count(&boot(&slices)),
            usize::from(MIGRATED_DEVICES),
            "a read failure is no evidence the slot is garbage"
        );
    }

    #[test]
    fn a_broken_bank_waits_while_the_whole_adapter_slot_cannot_be_read() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        boot_and_flush(&slices);
        write_slot(&slices.inner, bank_slot(BROKEN_BANK), b"not a persistence envelope");
        slices.fail_reads_of(Some(LEGACY_SLOT));

        let written = slices.written().len();
        let survivors = MIGRATED_DEVICES - SliceKey::DEVICES_PER_BANK;
        assert_eq!(device_count(&boot_and_flush(&slices)), usize::from(survivors));
        assert_eq!(slices.written().len(), written, "bank {BROKEN_BANK} lost its only copy");

        slices.fail_reads_of(None);
        assert_eq!(device_count(&boot_and_flush(&slices)), usize::from(MIGRATED_DEVICES));
        assert_eq!(device_count(&boot(&slices)), usize::from(MIGRATED_DEVICES));
    }

    #[test]
    fn a_broken_bank_without_a_whole_adapter_slot_is_rewritten_once() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, bank_slot(BROKEN_BANK), b"not a persistence envelope");

        let first = RegistryStore::with_adapter_count(1);
        assert_eq!(first.hydrate_from_store(&slices, 1).errors.iter().count(), 1);
        first.flush_dirty_slices(&slices);
        assert_eq!(slices.banks_written(), vec![bank_slot(BROKEN_BANK)]);
        let second = RegistryStore::with_adapter_count(1);
        assert!(second.hydrate_from_store(&slices, 1).is_ok(), "bank {BROKEN_BANK} stays broken");
    }

    #[test]
    fn a_waiting_bank_is_never_written_by_a_coarse_mark_or_a_scan() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        slices.fail_reads_of(Some(LEGACY_SLOT));
        let waiting = boot_and_flush(&slices);
        waiting.dirty.mark_physical_devices_dirty(0);
        assert!(waiting.seed_discovered_dt8(SCANNED_SHORT, Some(SCANNED_RANDOM_ADDRESS)));
        waiting.flush_dirty_slices(&slices);
        assert_eq!(slices.banks_written(), Vec::<SliceKey>::new(), "a waiting bank was written");

        slices.fail_reads_of(None);
        assert_eq!(device_count(&boot(&slices)), usize::from(MIGRATED_DEVICES));
    }

    #[test]
    fn a_waiting_bank_settles_on_a_reload_that_can_read_the_old_slot() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        slices.fail_reads_of(Some(LEGACY_SLOT));
        let session = boot(&slices);
        assert!(session.seed_discovered_dt8(SCANNED_SHORT, Some(SCANNED_RANDOM_ADDRESS)));

        slices.fail_reads_of(None);
        session.hydrate_counts_from_store(&slices, 1);
        session.flush_dirty_slices(&slices);
        let all_banks = usize::from(SliceKey::PHYSICAL_DEVICE_BANKS);
        assert_eq!(slices.banks_written().len(), all_banks, "the settled banks are written");
        let rebooted = boot(&slices);
        assert_eq!(device_count(&rebooted), usize::from(MIGRATED_DEVICES));
        assert_eq!(
            random_address_of(&rebooted, SCANNED_SHORT),
            Some(SCANNED_RANDOM_ADDRESS),
            "the record this session found loses to the old slot's"
        );
    }

    #[test]
    fn a_lamp_keeps_its_binding_while_its_bank_waits() {
        let slices = ProbeStore::default();
        let vl_blob = expected_vl_blob(&populated_store(MIGRATED_DEVICES), 0);
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        write_slot(&slices.inner, SliceKey::VirtualLamps { adapter_id: 0 }, &vl_blob);
        slices.fail_reads_of(Some(LEGACY_SLOT));
        let waiting = boot(&slices);
        assert_eq!(binding_of(&waiting, BOUND_LAMP), Some(BOUND_LAMP), "dropped at hydration");
        waiting.dirty.mark_virtual_lamps_dirty(0);
        waiting.flush_dirty_slices(&slices);

        slices.fail_reads_of(None);
        assert_eq!(binding_of(&boot(&slices), BOUND_LAMP), Some(BOUND_LAMP));
    }

    #[test]
    fn a_bank_emptied_before_its_first_write_stays_empty_across_a_reload() {
        let slices = InMemorySliceStore::new();
        write_slot(&slices, LEGACY_SLOT, &legacy_blob());
        let migrated = boot(&slices);
        for short in bank_shorts(EMPTIED_BANK) {
            migrated.apply_physical_device_forget(0, short).expect("a migrated device");
        }
        migrated.hydrate_counts_from_store(&slices, 1);
        migrated.flush_dirty_slices(&slices);

        let rebooted = boot(&slices);
        for state in [&migrated, &rebooted] {
            let back = bank_shorts(EMPTIED_BANK).filter(|short| device_name(state, *short).is_some());
            assert_eq!(back.collect::<Vec<u8>>(), Vec::<u8>::new(), "forgotten devices came back");
        }
    }

    #[test]
    fn a_bank_that_cannot_be_read_waits_and_keeps_its_newer_content() {
        let slices = ProbeStore::default();
        write_slot(&slices.inner, LEGACY_SLOT, &legacy_blob());
        let migrated = boot_and_flush(&slices);
        let forgotten = BROKEN_BANK * SliceKey::DEVICES_PER_BANK;
        migrated.apply_physical_device_forget(0, forgotten).expect("a migrated device");
        migrated.flush_dirty_slices(&slices);
        let newer = slices.inner.load(bank_slot(BROKEN_BANK)).expect("bank");

        slices.fail_reads_of(Some(bank_slot(BROKEN_BANK)));
        let unread = boot(&slices);
        assert_eq!(unread.persist_counters.unread_slices.load(std::sync::atomic::Ordering::Acquire), 1);
        unread.dirty.mark_physical_devices_dirty(0);
        unread.flush_dirty_slices(&slices);
        assert!(device_name(&unread, forgotten).is_none(), "the old slot stood in for the bank");
        assert_eq!(slices.inner.load(bank_slot(BROKEN_BANK)).expect("bank"), newer);

        slices.fail_reads_of(None);
        let recovered = boot(&slices);
        assert_eq!(device_count(&recovered), usize::from(MIGRATED_DEVICES) - 1);
        assert!(device_name(&recovered, forgotten).is_none());
    }

    #[test]
    fn booting_an_empty_store_writes_no_bank() {
        let slices = ProbeStore::default();
        boot_and_flush(&slices);
        assert_eq!(slices.banks_written(), Vec::<SliceKey>::new());
    }

    #[test]
    fn an_undecodable_whole_adapter_slot_rewrites_only_the_banks_that_did_not_load() {
        let slices = ProbeStore::default();
        write_banks_except(&slices.inner, &populated_store(MIGRATED_DEVICES), Some(UNWRITTEN_BANK));
        write_slot(&slices.inner, LEGACY_SLOT, b"not a persistence envelope");

        boot_and_flush(&slices);
        assert_eq!(slices.banks_written(), vec![bank_slot(UNWRITTEN_BANK)]);
    }

    #[test]
    fn a_reload_never_lets_the_whole_adapter_slot_overwrite_a_live_bank() {
        let banked = populated_store(MIGRATED_DEVICES);
        let installed = InMemorySliceStore::new();
        write_banks_except(&installed, &banked, None);
        let live = boot(&installed);
        for short in FORGOTTEN_SHORTS {
            live.apply_physical_device_forget(0, short).expect("a live device");
        }

        let imported = InMemorySliceStore::new();
        write_banks_except(&imported, &banked, Some(UNWRITTEN_BANK));
        write_slot(&imported, LEGACY_SLOT, &stale_legacy_blob());
        live.hydrate_counts_from_store(&imported, 1);
        live.flush_dirty_slices(&imported);

        let rebooted = boot(&imported);
        for state in [&live, &rebooted] {
            assert_eq!(device_name(state, 4), device_name(&banked, 4), "the stale record won");
            assert!(FORGOTTEN_SHORTS.iter().all(|short| device_name(state, *short).is_none()));
        }
    }

    #[test]
    fn a_reload_leaves_only_the_devices_each_loaded_bank_holds() {
        let installed = InMemorySliceStore::new();
        write_banks_except(&installed, &populated_store(MIGRATED_DEVICES), None);
        let live = boot(&installed);
        live.inner.write().expect("registry lock").physical_devices
            .get_mut(&(0, KEPT_SHORT)).expect("a kept device").runtime_level = Some(KEPT_LEVEL);

        let source = populated_store(MIGRATED_DEVICES);
        {
            let mut g = source.inner.write().expect("registry lock");
            for short in FORGOTTEN_SHORTS {
                g.physical_devices.remove(&(0, short));
            }
            let moved = g.physical_devices.remove(&(0, MOVED_FROM)).expect("a device to move");
            g.physical_devices.insert((0, MOVED_TO), moved);
        }
        let imported = InMemorySliceStore::new();
        write_banks_except(&imported, &source, None);
        live.hydrate_counts_from_store(&imported, 1);

        assert!(FORGOTTEN_SHORTS.iter().all(|short| device_name(&live, *short).is_none()));
        assert!(device_name(&live, MOVED_FROM).is_none(), "the old address stayed");
        assert_eq!(device_name(&live, MOVED_TO), device_name(&source, MOVED_TO));
        assert_eq!(device_count(&live), device_count(&source));
        let kept = live.inner.read().expect("registry lock").physical_devices
            .get(&(0, KEPT_SHORT)).and_then(|record| record.runtime_level);
        assert_eq!(kept, Some(KEPT_LEVEL), "a kept device lost its runtime level");
    }

    const MOVED_FROM: u8 = 3;
    const MOVED_TO: u8 = 40;
    const KEPT_SHORT: u8 = 2;
    const KEPT_LEVEL: u8 = 77;

    #[test]
    fn flush_reports_size_measurements() {
        let record_size = std::mem::size_of::<PersistablePhysicalDeviceRecord>();
        let store = populated_store(16);
        let blob = expected_pd_blob(&store, 0, None);
        println!(
            "size_of(PersistablePhysicalDeviceRecord)={record_size}B, \
             16 fat records stream to {}B ({}B/record on wire)",
            blob.len(),
            blob.len() / 16
        );
        assert!(
            record_size >= 400,
            "record no longer fat ({record_size}B) — the streamed flush was sized for fat records"
        );
    }

    #[derive(Default)]
    struct FailingSessionStore {
        inner: InMemorySliceStore,
    }

    struct FailingSession {
        appended: usize,
    }

    impl SliceWriteSession for FailingSession {
        fn append(&mut self, _chunk: &[u8]) -> Result<(), StoreError> {
            self.appended += 1;
            if self.appended >= 2 {
                return Err(StoreError::Backend("no space".to_string()));
            }
            Ok(())
        }

        fn commit(self: Box<Self>) -> Result<(), StoreError> {
            Err(StoreError::Backend("no space".to_string()))
        }

        fn abort(self: Box<Self>) {}
    }

    impl SliceStore for FailingSessionStore {
        fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
            self.inner.load(key)
        }

        fn begin_write(&self, _key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
            Ok(Box::new(FailingSession { appended: 0 }))
        }
    }

    #[test]
    fn mid_stream_store_failure_keeps_slice_dirty_for_retry() {
        let store = populated_store(16);
        store.dirty.mark_physical_device_dirty(0, 0);

        let failing = FailingSessionStore::default();
        store.flush_dirty_slices(&failing);
        assert!(failing.inner.labels().is_empty(), "a failed stream must publish nothing");
        assert_eq!(store.persist_counters.flush_error_total.load(Ordering::Acquire), 1);
        assert!(store.dirty.any_dirty(), "slice must stay dirty for retry");

        let healthy = InMemorySliceStore::new();
        store.flush_dirty_slices(&healthy);
        assert_eq!(
            healthy
                .load(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank: 0 })
                .expect("pd"),
            expected_pd_blob(&store, 0, Some(0))
        );
    }
}

