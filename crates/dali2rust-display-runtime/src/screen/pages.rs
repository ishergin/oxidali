use dali2rust_contracts::msg::DisplayPage;

use super::{
    clip, fmt_age, fmt_uptime, input_who, operation_row, poll_text, row_hcl, source_label, spread,
    summary_row, RowText, ScreenRow, ScreenState,
};
use crate::source::DisplaySample;

const PERMILLE_PER_PERCENT: u16 = 10;

pub(super) fn page_row(state: &ScreenState, row: usize) -> ScreenRow {
    match state.page {
        DisplayPage::Summary => summary_row(state, row),
        DisplayPage::Network => network_row(state, row),
        DisplayPage::Bus => bus_row(state, row),
        DisplayPage::Activity => activity_row(state, row),
    }
}

fn alarm(text: RowText) -> ScreenRow {
    ScreenRow { text, invert: true, bar: None, spark: false }
}

fn network_row(state: &ScreenState, row: usize) -> ScreenRow {
    let s = &state.sample;
    match row {
        0 => net_address(state),
        1 => role(s),
        2 => ScreenRow::text(spread("UPTIME", &fmt_uptime(state.uptime_s))),
        3 => ScreenRow::text(spread("TIME", if s.time_synced { "SYNCED" } else { "NOT SYNCED" })),
        4 => ScreenRow::text(spread("MQTT", if s.mqtt_connected { "CONNECTED" } else { "DOWN" })),
        5 => ScreenRow::text(spread("WS CLIENTS", &RowText::from_fmt(format_args!("{}", s.ws_clients)))),
        _ => flash(s),
    }
}

fn net_address(state: &ScreenState) -> ScreenRow {
    match state.ip {
        Some(ip) => ScreenRow::text(RowText::from_fmt(format_args!(
            "NET  {}.{}.{}.{}",
            ip[0], ip[1], ip[2], ip[3]
        ))),
        None => alarm(clip("NET  NO IP LINK DOWN")),
    }
}

fn role(s: &DisplaySample) -> ScreenRow {
    if !s.redundancy_enabled {
        return ScreenRow::text(spread("ROLE", "SINGLE"));
    }
    if s.controller_active {
        ScreenRow::text(spread("ROLE", "ACTIVE"))
    } else {
        alarm(spread("ROLE", "STANDBY"))
    }
}

fn flash(s: &DisplaySample) -> ScreenRow {
    if s.persistence_failed > 0 {
        alarm(spread("FLASH", "NO SAVE"))
    } else {
        ScreenRow::text(spread("FLASH", "OK"))
    }
}

fn bus_row(state: &ScreenState, row: usize) -> ScreenRow {
    let s = &state.sample;
    match row {
        0 => bus_verdict(state),
        1 => ScreenRow::text(spread("FRAMES", &per_minute(state.frames_per_min))),
        2 => ScreenRow::text(spread("FOREIGN", &per_minute(state.foreign_per_min))),
        3 => wire_load(s),
        4 => ScreenRow::text(poll_text(&state.poller)),
        5 => gear(s),
        _ => ScreenRow::text(spread(
            "INPUTS",
            &RowText::from_fmt(format_args!("{}/{}", s.input_present, s.input_known)),
        )),
    }
}

fn per_minute(count: u32) -> RowText {
    RowText::from_fmt(format_args!("{count}/MIN"))
}

fn bus_verdict(state: &ScreenState) -> ScreenRow {
    if !state.sample.adapter_enabled {
        return alarm(clip("BUS  DISABLED"));
    }
    let text = RowText::from_fmt(format_args!("BUS  {}", state.bus.label()));
    ScreenRow { text, invert: state.bus.is_alarm(), bar: None, spark: false }
}

fn wire_load(s: &DisplaySample) -> ScreenRow {
    let percent = s.wire_load_permille / PERMILLE_PER_PERCENT;
    ScreenRow {
        text: RowText::from_fmt(format_args!("LOAD {percent}%")),
        invert: false,
        bar: None,
        spark: true,
    }
}

fn gear(s: &DisplaySample) -> ScreenRow {
    let online = s.gear_known.saturating_sub(s.gear_unreachable);
    let head = RowText::from_fmt(format_args!("ANSWER {online}/{}", s.gear_known));
    let tail = RowText::from_fmt(format_args!("FAULT {}", s.gear_faulted));
    ScreenRow { text: spread(&head, &tail), invert: s.gear_faulted > 0, bar: None, spark: false }
}

fn activity_row(state: &ScreenState, row: usize) -> ScreenRow {
    match row {
        0 => state
            .operation
            .as_ref()
            .map_or_else(|| ScreenRow::text(clip("OP   IDLE")), operation_row),
        1 => last_action(state),
        2 => action_origin(state),
        3 => input_source(state),
        4 => input_event(state),
        5 => row_hcl(state),
        _ => ScreenRow::default(),
    }
}

fn last_action(state: &ScreenState) -> ScreenRow {
    let Some(act) = state.last_action.as_ref() else {
        return ScreenRow::text(clip("LAST --"));
    };
    let text = RowText::from_fmt(format_args!("LAST {} {}", act.target, act.what));
    ScreenRow { text, invert: act.failed, bar: None, spark: false }
}

fn action_origin(state: &ScreenState) -> ScreenRow {
    let Some(act) = state.last_action.as_ref() else {
        return ScreenRow::default();
    };
    let by = RowText::from_fmt(format_args!("BY   {}", act.source.map_or("--", source_label)));
    let age = RowText::from_fmt(format_args!("{} AGO", fmt_age(act.age_ms)));
    ScreenRow::text(spread(&by, &age))
}

fn input_source(state: &ScreenState) -> ScreenRow {
    let Some(last) = state.last_input.as_ref() else {
        return ScreenRow::text(clip("INPUT --"));
    };
    ScreenRow::text(RowText::from_fmt(format_args!("INPUT {}", input_who(last))))
}

fn input_event(state: &ScreenState) -> ScreenRow {
    let Some(last) = state.last_input.as_ref() else {
        return ScreenRow::default();
    };
    ScreenRow::text(RowText::from_fmt(format_args!("EVENT {}", last.what)))
}
