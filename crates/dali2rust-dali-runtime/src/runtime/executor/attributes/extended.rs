use core::cmp::Reverse;

use super::*;

// IEC 62386-102 §9.5.4, Table 6, Table 7
const EXTENDED_FADE_UNITS_MS: [(u32, u8); 4] = [(100, 1), (1_000, 2), (10_000, 3), (60_000, 4)];
const EXTENDED_FADE_BASES: u32 = 16;
const EXTENDED_FADE_TOP_BYTE: u8 = 0x4F;

pub(super) fn extended_fade_time_byte_from_ms(ms: u32) -> u8 {
    if ms == 0 {
        return 0;
    }
    EXTENDED_FADE_UNITS_MS
        .iter()
        .map(|&(unit, mult)| {
            let base = (ms.saturating_add(unit / 2) / unit).clamp(1, EXTENDED_FADE_BASES);
            (unit * base, (mult << 4) | (base as u8 - 1))
        })
        .min_by_key(|&(grid, _)| (grid.abs_diff(ms), Reverse(grid)))
        .map_or(EXTENDED_FADE_TOP_BYTE, |(_, byte)| byte)
}

pub(super) fn extended_fade_time_ms_from_byte(byte: u8) -> Option<u32> {
    let mult = byte >> 4;
    if mult == 0 {
        return Some(0);
    }
    let unit = EXTENDED_FADE_UNITS_MS
        .iter()
        .find(|(_, m)| *m == mult)
        .map(|(u, _)| *u)?;
    Some(unit * (u32::from(byte & 0x0F) + 1))
}

pub(super) fn write_extended_fade_time(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    extended_fade_time_ms: Option<u32>,
) -> Result<(), SemanticDaliError> {
    let Some(ms) = extended_fade_time_ms else {
        return Ok(());
    };
    let dtr0 = extended_fade_time_byte_from_ms(ms);
    let verify =
        |c: &mut _| send_standard_response(c, address, StandardCommand::QueryExtendedFadeTime);
    let read_back =
        send_dtr0_config_verified(controller, address, dtr0, StandardCommand::SetExtendedFadeTime, verify)?;
    if tally.proved(read_back).is_some() {
        tally.confirmed.extended_fade_time_ms = extended_fade_time_ms_from_byte(dtr0);
    }
    Ok(())
}

pub(super) struct ExtendedReadSnapshot {
    pub(super) fade_time_ms: Option<u32>,
    pub(super) version_number: Option<u8>,
    pub(super) versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS],
}

#[derive(Clone, Copy)]
pub(super) struct KnownVersions {
    pub(super) device_types: Option<DeviceTypeSet>,
    pub(super) dt6: Option<u8>,
}

pub(super) fn read_extended_snapshot(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    known: KnownVersions,
) -> Result<ExtendedReadSnapshot, SemanticDaliError> {
    let fade_time_ms = read_extended_fade_time(controller, address, content_confirm)?;
    let device_types = match known.device_types {
        Some(types) => Some(types),
        None => super::super::discovery::read_declared_device_types(controller, address, content_confirm)?,
    };
    let versions = read_extended_versions(controller, address, content_confirm, device_types, known.dt6)?;
    let version_number = versions
        .iter()
        .flatten()
        .find(|e| e.device_type == DeviceType::Led.code())
        .and_then(|e| e.version_number);
    Ok(ExtendedReadSnapshot {
        fade_time_ms,
        version_number,
        versions,
    })
}

// IEC 62386-102 §9.18, §11.6.2
fn read_extended_versions(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
    device_types: Option<DeviceTypeSet>,
    dt6_version: Option<u8>,
) -> Result<[Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS], SemanticDaliError> {
    let mut versions = [None; MAX_EXTENDED_VERSIONS];
    let Some(device_types) = device_types else {
        return Ok(versions);
    };
    for (slot, device_type) in versions.iter_mut().zip(device_types.iter()) {
        let version_number = match dt6_version.filter(|_| device_type == DeviceType::Led.code()) {
            Some(already_read) => Some(already_read),
            None => send_extended_query_stable(
                controller,
                content_confirm,
                address,
                ExtendedCommand::ExtendedVersion { device_type },
            )?,
        };
        *slot = Some(ExtendedVersionEntry { device_type, version_number });
    }
    Ok(versions)
}

fn read_extended_fade_time(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u32>, SemanticDaliError> {
    let first = read_extended_fade_byte(controller, address, content_confirm)?;
    if let Some(ms) = first.and_then(extended_fade_time_ms_from_byte) {
        return Ok(Some(ms));
    }
    log::info!("extended fade time: unrepresentable readback byte, re-reading");
    let reread = read_extended_fade_byte(controller, address, content_confirm)?;
    if first.is_some() && reread.is_some() && reread != first {
        controller.note_workaround(ReadbackWorkaround::ExtendedFadeCorrected);
    }
    Ok(reread.and_then(extended_fade_time_ms_from_byte))
}

fn read_extended_fade_byte(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    content_confirm: ContentConfirmPolicy,
) -> Result<Option<u8>, SemanticDaliError> {
    send_standard_query_stable(
        controller,
        content_confirm,
        address,
        StandardCommand::QueryExtendedFadeTime,
    )
}
