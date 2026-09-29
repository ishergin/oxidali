use super::*;

const EXTENDED_FADE_UNITS_MS: [(u32, u8); 4] = [(100, 1), (1_000, 2), (10_000, 3), (60_000, 4)];

pub(super) fn extended_fade_time_byte_from_ms(ms: u16) -> u8 {
    if ms == 0 {
        return 0;
    }
    for (unit, mult) in EXTENDED_FADE_UNITS_MS {
        if u32::from(ms) <= unit * 16 {
            let base = ((u32::from(ms) + unit / 2) / unit).clamp(1, 16) as u8;
            let contract_max_base = (u32::from(u16::MAX) / unit).clamp(1, 16) as u8;
            let base = base.min(contract_max_base);
            return (mult << 4) | (base - 1);
        }
    }
    (4 << 4) | 0x0F
}

pub(super) fn extended_fade_time_ms_from_byte(byte: u8) -> Option<u16> {
    let mult = (byte >> 4) & 0x07;
    if mult == 0 {
        return Some(0);
    }
    let unit = EXTENDED_FADE_UNITS_MS
        .iter()
        .find(|(_, m)| *m == mult)
        .map(|(u, _)| *u)?;
    let base = u32::from(byte & 0x0F) + 1;
    u16::try_from(unit * base).ok()
}

pub(super) fn write_extended_fade_time(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    tally: &mut WriteTally,
    extended_fade_time_ms: Option<u16>,
) -> Result<(), SemanticDaliError> {
    let Some(ms) = extended_fade_time_ms else {
        return Ok(());
    };
    let dtr0 = extended_fade_time_byte_from_ms(ms);
    let verify = |c: &mut _| send_standard_query(c, address, StandardCommand::QueryExtendedFadeTime);
    let read_back =
        send_dtr0_config_verified(controller, address, dtr0, StandardCommand::SetExtendedFadeTime, verify)?;
    if tally.proved(read_back).is_some() {
        tally.confirmed.extended_fade_time_ms = extended_fade_time_ms_from_byte(dtr0);
    }
    Ok(())
}

pub(super) struct ExtendedReadSnapshot {
    pub(super) fade_time_ms: Option<u16>,
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
) -> Result<Option<u16>, SemanticDaliError> {
    let first = read_extended_fade_byte(controller, address, content_confirm)?;
    if let Some(ms) = first.and_then(extended_fade_time_ms_from_byte) {
        return Ok(Some(ms));
    }
    if first.is_some() {
        controller.note_workaround(ReadbackWorkaround::ExtendedFadeUnrepresentable);
    }
    log::info!("extended fade time: unrepresentable readback byte, re-reading");
    Ok(read_extended_fade_byte(controller, address, content_confirm)?
        .and_then(extended_fade_time_ms_from_byte))
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
