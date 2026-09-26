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
        let mut st = serializer.serialize_struct("PersistableSceneRow", 6)?;
        st.serialize_field("virtual_lamp_id", &self.virtual_lamp_id)?;
        st.serialize_field("included", &self.record.included)?;
        st.serialize_field("desired_seeded", &self.record.desired_seeded)?;
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
    use super::super::virtual_lamps::VlRecord;
    use super::FLUSH_CHUNK_BYTES;
    use dali2rust_contracts::msg::{fixed_text_64, ColorMode, DeviceType, DeviceTypeSet};
    use dali2rust_domain::registry::{
        AttributeSource, MemoryBankRangeView, MemoryBankSummaryView, ObservedValue,
        PhysicalDeviceAttributesView,
    };
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore, SliceWriteSession, StoreError};

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
        use dali2rust_domain::registry::SceneReadPort;
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
    fn one_dirty_device_writes_one_banks_worth_of_bytes_issue89() {
        use crate::test_support::CountingStore;

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
        use crate::test_support::CountingStore;

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
    fn empty_banks_do_not_suppress_the_whole_adapter_fallback() {
        let store = InMemorySliceStore::new();
        let old = populated_store(16);

        for bank in 0..SliceKey::PHYSICAL_DEVICE_BANKS {
            let mut session = store
                .begin_write(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank })
                .expect("begin");
            session
                .append(&expected_pd_blob_for_shorts(&old, 0, 200..201))
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
            "empty banks say nothing about whether the adapter has devices, and \
             reading them as `loaded` is what emptied the bench's registry"
        );
    }

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
            "record no longer fat ({record_size}B) — revisit ISSUE-1 sizing notes"
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
        use std::sync::atomic::Ordering;
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

