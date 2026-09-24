use crate::cursor::Cursor;
use crate::lexer::{Pos, TokenKind};
use dali2rust_rules_model::limits::{
    MAX_INSTANCE_GROUP, MAX_INSTANCE_NUMBER, MAX_NAME_BYTES, MAX_SHORT_ADDRESS,
};
use dali2rust_rules_model::{
    CompileError, DeviceRef, GroupRef, InputDeviceRef, InputGroupSelector, InputRef,
    InputSelector, LampRef, LightTarget,
};

pub enum NameOrId {
    Name(String, Pos),
    Id(i64, Pos),
}

pub fn checked_name(c: &mut Cursor<'_>, what: &str) -> Result<(String, Pos), CompileError> {
    let (name, pos) = c.expect_string(what)?;
    if name.is_empty() {
        return Err(pos.err(format!("{what} must not be empty")));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(pos.err(format!("{what} exceeds {MAX_NAME_BYTES} bytes")));
    }
    Ok((name, pos))
}

fn name_or_id(c: &mut Cursor<'_>, what: &str) -> Result<NameOrId, CompileError> {
    let pos = c.here();
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Str(_)) => {
            let (name, pos) = checked_name(c, what)?;
            Ok(NameOrId::Name(name, pos))
        }
        Some(TokenKind::Int(_)) => {
            let (id, pos) = c.expect_int(what)?;
            Ok(NameOrId::Id(id, pos))
        }
        _ => Err(pos.err(format!("expected {what} name or id"))),
    }
}

fn adapter_clause(c: &mut Cursor<'_>) -> Result<Option<u8>, CompileError> {
    if !c.accept(&TokenKind::Comma) {
        return Ok(None);
    }
    c.expect_kw("adapter")?;
    c.expect(&TokenKind::Assign, "`=` after adapter")?;
    let (value, pos) = c.expect_int_in("adapter", 0, 255)?;
    let adapter = value as u8;
    if !c.resolver.adapter_exists(adapter) {
        return Err(pos.err(format!("unknown adapter {adapter}")));
    }
    Ok(Some(adapter))
}

fn scope_or_primary(c: &Cursor<'_>, explicit: Option<u8>) -> u8 {
    explicit.unwrap_or_else(|| c.resolver.primary_adapter())
}

fn check_scope(
    what: &str,
    name: &str,
    pos: Pos,
    resolved: u8,
    explicit: Option<u8>,
) -> Result<(), CompileError> {
    match explicit {
        Some(adapter) if adapter != resolved => Err(pos.err(format!(
            "{what} \"{name}\" is on adapter {resolved}, not {adapter}"
        ))),
        _ => Ok(()),
    }
}

pub fn lamp_ref(c: &mut Cursor<'_>) -> Result<LampRef, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after lamp")?;
    let target = name_or_id(c, "lamp")?;
    let explicit = adapter_clause(c)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    match target {
        NameOrId::Name(name, pos) => {
            let Some(lamp) = c.resolver.resolve_lamp(&name) else {
                return Err(pos.err(format!("unknown lamp \"{name}\"")));
            };
            check_scope("lamp", &name, pos, lamp.adapter_id, explicit)?;
            Ok(lamp)
        }
        NameOrId::Id(id, pos) => {
            if !(0..=i64::from(u16::MAX)).contains(&id) {
                return Err(pos.err(format!("lamp id {id} out of range")));
            }
            Ok(LampRef { adapter_id: scope_or_primary(c, explicit), id: id as u16 })
        }
    }
}

pub fn group_ref(c: &mut Cursor<'_>) -> Result<GroupRef, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after group")?;
    let target = name_or_id(c, "group")?;
    let explicit = adapter_clause(c)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    match target {
        NameOrId::Name(name, pos) => {
            let Some(group) = c.resolver.resolve_group(&name) else {
                return Err(pos.err(format!("unknown group \"{name}\"")));
            };
            check_scope("group", &name, pos, group.adapter_id, explicit)?;
            Ok(group)
        }
        NameOrId::Id(id, pos) => {
            if !(0..=i64::from(u16::MAX)).contains(&id) {
                return Err(pos.err(format!("group id {id} out of range")));
            }
            Ok(GroupRef { adapter_id: scope_or_primary(c, explicit), id: id as u16 })
        }
    }
}

pub fn device_ref(c: &mut Cursor<'_>) -> Result<DeviceRef, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after device")?;
    let target = name_or_id(c, "device")?;
    let explicit = adapter_clause(c)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    match target {
        NameOrId::Name(name, pos) => {
            let Some(device) = c.resolver.resolve_device(&name) else {
                return Err(pos.err(format!("unknown device \"{name}\"")));
            };
            check_scope("device", &name, pos, device.adapter_id, explicit)?;
            Ok(device)
        }
        NameOrId::Id(id, pos) => {
            if !(0..=i64::from(MAX_SHORT_ADDRESS)).contains(&id) {
                return Err(pos.err(format!("short address {id} out of range 0..=63")));
            }
            Ok(DeviceRef {
                adapter_id: scope_or_primary(c, explicit),
                short_address: id as u8,
            })
        }
    }
}

#[derive(Default)]
struct InputArgs {
    dev: Option<NameOrId>,
    inst: Option<(i64, Pos)>,
    group: Option<(i64, Pos)>,
    instance_type: Option<u8>,
    adapter: Option<(u8, Pos)>,
    positionals: usize,
}

fn input_type_value(c: &mut Cursor<'_>) -> Result<u8, CompileError> {
    let pos = c.here();
    match c.peek().map(|t| &t.kind) {
        Some(TokenKind::Int(_)) => {
            let (v, _) = c.expect_int_in("instance type", 0, 31)?;
            Ok(v as u8)
        }
        Some(TokenKind::Ident(_)) => {
            let (word, pos) = c.expect_ident("instance type")?;
            match word.as_str() {
                "button" => Ok(1),
                "absolute" => Ok(2),
                "occupancy" => Ok(3),
                "light" => Ok(4),
                _ => Err(pos.err(format!("unknown instance type \"{word}\""))),
            }
        }
        _ => Err(pos.err("expected instance type")),
    }
}

fn input_named_arg(c: &mut Cursor<'_>, key: &str, pos: Pos, args: &mut InputArgs) -> Result<(), CompileError> {
    match key {
        "dev" => args.dev = Some(name_or_id(c, "input device")?),
        "inst" => args.inst = Some(c.expect_int_in("inst", 0, i64::from(MAX_INSTANCE_NUMBER))?),
        "group" => args.group = Some(c.expect_int_in("group", 0, i64::from(MAX_INSTANCE_GROUP))?),
        "type" => args.instance_type = Some(input_type_value(c)?),
        "adapter" => {
            let (value, pos) = c.expect_int_in("adapter", 0, 255)?;
            if !c.resolver.adapter_exists(value as u8) {
                return Err(pos.err(format!("unknown adapter {value}")));
            }
            args.adapter = Some((value as u8, pos));
        }
        _ => return Err(pos.err(format!("unknown input argument \"{key}\""))),
    }
    Ok(())
}

fn input_positional_arg(c: &mut Cursor<'_>, args: &mut InputArgs) -> Result<(), CompileError> {
    let pos = c.here();
    match args.positionals {
        0 => args.dev = Some(name_or_id(c, "input device")?),
        1 => args.inst = Some(c.expect_int_in("inst", 0, i64::from(MAX_INSTANCE_NUMBER))?),
        _ => return Err(pos.err("too many positional input arguments")),
    }
    args.positionals += 1;
    Ok(())
}

fn input_one_arg(c: &mut Cursor<'_>, args: &mut InputArgs) -> Result<(), CompileError> {
    let named_key = match (c.peek().map(|t| &t.kind), c.peek_second().map(|t| &t.kind)) {
        (Some(TokenKind::Ident(key)), Some(TokenKind::Assign)) => Some(key.clone()),
        _ => None,
    };
    if let Some(key) = named_key {
        let pos = c.here();
        c.advance();
        c.advance();
        input_named_arg(c, &key, pos, args)
    } else {
        input_positional_arg(c, args)
    }
}

fn input_args(c: &mut Cursor<'_>) -> Result<(InputArgs, Pos), CompileError> {
    c.expect(&TokenKind::LParen, "`(` after input")?;
    let mut args = InputArgs::default();
    loop {
        input_one_arg(c, &mut args)?;
        if !c.accept(&TokenKind::Comma) {
            break;
        }
    }
    let close = c.expect(&TokenKind::RParen, "`)`")?;
    Ok((args, close))
}

fn resolve_input_device(
    c: &Cursor<'_>,
    dev: NameOrId,
    explicit: Option<(u8, Pos)>,
) -> Result<InputDeviceRef, CompileError> {
    let adapter = explicit.map(|(a, _)| a);
    match dev {
        NameOrId::Name(name, pos) => {
            let Some(device) = c.resolver.resolve_input_device(&name) else {
                return Err(pos.err(format!("unknown input device \"{name}\"")));
            };
            check_scope("input device", &name, pos, device.adapter_id, adapter)?;
            Ok(device)
        }
        NameOrId::Id(id, pos) => {
            if !(0..=i64::from(MAX_SHORT_ADDRESS)).contains(&id) {
                return Err(pos.err(format!("device short address {id} out of range 0..=63")));
            }
            Ok(InputDeviceRef {
                adapter_id: scope_or_primary(c, adapter),
                device_short_address: id as u8,
            })
        }
    }
}

pub fn input_selector(c: &mut Cursor<'_>) -> Result<InputSelector, CompileError> {
    let (args, close) = input_args(c)?;
    if let Some((group, pos)) = args.group {
        if args.dev.is_some() || args.inst.is_some() {
            return Err(pos.err("group addressing excludes dev/inst"));
        }
        return Ok(InputSelector::Group(InputGroupSelector {
            adapter_id: scope_or_primary(c, args.adapter.map(|(a, _)| a)),
            instance_group: group as u8,
            instance_type: args.instance_type,
        }));
    }
    let Some(dev) = args.dev else {
        return Err(close.err("expected dev=… or group=…"));
    };
    let Some((inst, _)) = args.inst else {
        return Err(close.err("expected inst=…"));
    };
    let device = resolve_input_device(c, dev, args.adapter)?;
    Ok(InputSelector::Instance(InputRef {
        adapter_id: device.adapter_id,
        device_short_address: device.device_short_address,
        instance_number: inst as u8,
    }))
}

pub fn input_ref(c: &mut Cursor<'_>) -> Result<InputRef, CompileError> {
    let pos = c.here();
    match input_selector(c)? {
        InputSelector::Instance(input) => Ok(input),
        InputSelector::Group(_) => Err(pos.err(
            "instance-group addressing is only for `input(…) is <event>` triggers — use dev=…, inst=…",
        )),
    }
}

pub fn input_device_ref(c: &mut Cursor<'_>) -> Result<InputDeviceRef, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after device")?;
    let named = c.accept_kw("dev");
    if named {
        c.expect(&TokenKind::Assign, "`=` after dev")?;
    }
    let dev = name_or_id(c, "input device")?;
    let explicit = adapter_clause(c)?.map(|a| (a, c.here()));
    c.expect(&TokenKind::RParen, "`)`")?;
    resolve_input_device(c, dev, explicit)
}

pub fn light_target(c: &mut Cursor<'_>) -> Result<LightTarget, CompileError> {
    if c.accept_kw("lamp") {
        return Ok(LightTarget::Lamp(lamp_ref(c)?));
    }
    if c.accept_kw("group") {
        return Ok(LightTarget::Group(group_ref(c)?));
    }
    if c.accept_kw("broadcast") {
        return Ok(LightTarget::Broadcast {
            adapter_id: c.resolver.primary_adapter(),
        });
    }
    Err(c.err_here("expected lamp(…), group(…) or broadcast"))
}
