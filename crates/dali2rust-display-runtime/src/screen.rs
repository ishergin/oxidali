use core::fmt::Write as _;

use dali2rust_contracts::msg::{BusHealthVerdict, RuntimeSource};

use crate::source::DisplaySample;

pub const SCREEN_COLS: usize = 21;
const MS_PER_SECOND: u32 = 1000;
const SECONDS_PER_MINUTE: u32 = 60;
const SECONDS_PER_HOUR: u32 = 3600;
const SECONDS_PER_DAY: u32 = 86_400;
const AGE_SECONDS_MAX: u32 = 99;
const AGE_MINUTES_MAX_SECONDS: u32 = 5999;
const TWO_DIGIT_MAX: u32 = 99;
const BAR_FULL_PERMILLE: u32 = 1000;
pub const SPARK_COLS: usize = 16;
pub const SPARK_H: u8 = 6;
const COLS_WITH_SPARK: usize = SCREEN_COLS - 3;
pub const SCREEN_ROWS: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedAscii<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> Default for FixedAscii<N> {
    fn default() -> Self {
        Self { buf: [0; N], len: 0 }
    }
}

impl<const N: usize> FixedAscii<N> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_fmt(args: core::fmt::Arguments<'_>) -> Self {
        let mut out = Self::new();
        let _ = core::fmt::Write::write_fmt(&mut out, args);
        out
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }

    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push_ascii(&mut self, byte: u8) {
        let at = usize::from(self.len);
        if at < N {
            self.buf[at] = byte;
            self.len += 1;
        }
    }

    fn pop(&mut self) {
        self.len = self.len.saturating_sub(1);
    }

    fn ends_with_space(&self) -> bool {
        usize::from(self.len) > 0 && self.buf[usize::from(self.len) - 1] == b' '
    }

    fn truncate(&mut self, cols: usize) {
        let cols = u8::try_from(cols.min(N)).unwrap_or(u8::MAX);
        self.len = self.len.min(cols);
    }
}

impl<const N: usize> core::fmt::Write for FixedAscii<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            if usize::from(self.len) == N {
                return Ok(());
            }
            self.push_ascii(if ch.is_ascii() { ch as u8 } else { b'?' });
        }
        Ok(())
    }
}

impl<const N: usize> core::ops::Deref for FixedAscii<N> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> core::fmt::Display for FixedAscii<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> PartialEq<&str> for FixedAscii<N> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl<const N: usize> PartialEq<FixedAscii<N>> for &str {
    fn eq(&self, other: &FixedAscii<N>) -> bool {
        *self == other.as_str()
    }
}

pub type RowText = FixedAscii<SCREEN_COLS>;
pub type TargetTag = FixedAscii<8>;
pub type EventLabel = FixedAscii<16>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenRow {
    pub text: RowText,
    pub invert: bool,
    pub bar: Option<u16>,
    pub spark: bool,
}

impl ScreenRow {
    fn text(text: RowText) -> Self {
        Self { text, invert: false, bar: None, spark: false }
    }
}

pub type ScreenLines = [ScreenRow; SCREEN_ROWS];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusReading {
    #[default]
    Unprobed,
    Void,
    Clear,
    OneFailure,
    SeveralFailures,
}

impl BusReading {
    pub fn from_probe(control_answered: bool, verdict: BusHealthVerdict) -> Self {
        if !control_answered {
            return Self::Void;
        }
        match verdict {
            BusHealthVerdict::Clear => Self::Clear,
            BusHealthVerdict::One => Self::OneFailure,
            BusHealthVerdict::Several => Self::SeveralFailures,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unprobed => "?",
            Self::Void => "NO ANSWER",
            Self::Clear => "OK",
            Self::OneFailure => "LAMP FAIL",
            Self::SeveralFailures => "LAMP FAIL 2+",
        }
    }

    fn is_alarm(self) -> bool {
        matches!(self, Self::Void | Self::OneFailure | Self::SeveralFailures)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputLine {
    pub short_address: Option<u8>,
    pub instance: Option<u8>,
    pub what: EventLabel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionLine {
    pub target: TargetTag,
    pub what: EventLabel,
    pub source: Option<RuntimeSource>,
    pub failed: bool,
    pub age_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationLine {
    pub name: EventLabel,
    pub done: u16,
    pub total: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PollerLine {
    pub enabled: bool,
    pub interval_ms: u32,
    pub failing: bool,
    pub absent: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenState {
    pub ip: Option<[u8; 4]>,
    pub uptime_s: u32,
    pub sample: DisplaySample,
    pub bus: BusReading,
    pub frames_per_min: u32,
    pub foreign_per_min: u32,
    pub load_history: [u16; SPARK_COLS],
    pub spark: [u8; SPARK_COLS],
    pub poller: PollerLine,
    pub hcl_kelvin: Option<u16>,
    pub last_input: Option<InputLine>,
    pub last_action: Option<ActionLine>,
    pub operation: Option<OperationLine>,
}

pub fn render_row(state: &ScreenState, row: usize) -> ScreenRow {
    match row {
        0 => row_address(state),
        1 => row_devices(state),
        2 => row_bus(state),
        3 => row_hcl(state),
        4 => row_services(state),
        5 => row_input(state),
        _ => row_live(state),
    }
}

pub fn render(state: &ScreenState) -> ScreenLines {
    std::array::from_fn(|row| render_row(state, row))
}

fn row_address(state: &ScreenState) -> ScreenRow {
    let Some(ip) = state.ip else {
        return ScreenRow {
            text: clip("NO IP  LINK DOWN"),
            invert: true,
            bar: None,
            spark: false,
        };
    };
    let addr = RowText::from_fmt(format_args!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
    let Some(role) = role_letter(&state.sample) else {
        return ScreenRow::text(spread(&addr, &fmt_uptime(state.uptime_s)));
    };
    ScreenRow {
        text: spread(&addr, &role_and_uptime(role, state.uptime_s)),
        invert: !state.sample.controller_active,
        bar: None,
        spark: false,
    }
}

fn role_letter(sample: &DisplaySample) -> Option<char> {
    if !sample.redundancy_enabled {
        return None;
    }
    Some(if sample.controller_active { 'A' } else { 'S' })
}

fn role_and_uptime(role: char, uptime_s: u32) -> TargetTag {
    TargetTag::from_fmt(format_args!("{role}{}", fmt_uptime(uptime_s).as_str()))
}

fn row_devices(state: &ScreenState) -> ScreenRow {
    let s = state.sample;
    let online = s.gear_known.saturating_sub(s.gear_unreachable);
    let mut gear = RowText::from_fmt(format_args!("GEAR {online}/{}", s.gear_known));
    if s.gear_faulted > 0 {
        let _ = write!(gear, "!{}", s.gear_faulted);
    }
    let inputs = RowText::from_fmt(format_args!("IN {}/{}", s.input_present, s.input_known));
    ScreenRow {
        text: spread(&gear, &inputs),
        invert: s.gear_faulted > 0,
        bar: None,
        spark: false,
    }
}

fn row_bus(state: &ScreenState) -> ScreenRow {
    if !state.sample.adapter_enabled {
        return ScreenRow { text: clip("BUS  DISABLED"), invert: true, bar: None, spark: true };
    }
    let rate = RowText::from_fmt(format_args!(
        "{}F/M F{}",
        state.frames_per_min, state.foreign_per_min
    ));
    let head = RowText::from_fmt(format_args!("BUS  {}", state.bus.label()));
    ScreenRow {
        text: spread_within(COLS_WITH_SPARK, &head, &rate),
        invert: state.bus.is_alarm(),
        bar: None,
        spark: true,
    }
}

pub fn spark_heights_into(history: &[u16; SPARK_COLS], out: &mut [u8; SPARK_COLS]) {
    for (height, permille) in out.iter_mut().zip(history) {
        let scaled = (u32::from(*permille) * u32::from(SPARK_H) / BAR_FULL_PERMILLE)
            .min(u32::from(SPARK_H)) as u8;
        *height = if *permille > 0 { scaled.max(1) } else { 0 };
    }
}

fn row_hcl(state: &ScreenState) -> ScreenRow {
    if !state.sample.time_synced {
        return ScreenRow::text(clip("HCL  STOPPED NO TIME"));
    }
    let point = match state.hcl_kelvin {
        Some(k) => RowText::from_fmt(format_args!("HCL  {k}K")),
        None => RowText::from_fmt(format_args!("HCL  IDLE")),
    };
    let overrides = RowText::from_fmt(format_args!("OVR {}", state.sample.hcl_overrides_active));
    ScreenRow::text(spread(&point, &overrides))
}

fn row_services(state: &ScreenState) -> ScreenRow {
    let poll = if state.poller.enabled {
        let secs = state.poller.interval_ms.div_ceil(MS_PER_SECOND);
        let verdict = if state.poller.failing {
            "ER"
        } else if state.poller.absent {
            "NA"
        } else {
            "OK"
        };
        RowText::from_fmt(format_args!("POLL {secs}S {verdict}"))
    } else {
        RowText::from_fmt(format_args!("POLL OFF"))
    };
    if state.sample.persistence_failed > 0 {
        return ScreenRow {
            text: spread(&poll, "NO SAVE"),
            invert: true,
            bar: None,
            spark: false,
        };
    }
    let bridges = RowText::from_fmt(format_args!(
        "{} WS{}",
        if state.sample.mqtt_connected { "MQTT+" } else { "MQTT-" },
        state.sample.ws_clients.min(TWO_DIGIT_MAX)
    ));
    ScreenRow::text(spread(&poll, &bridges))
}

fn row_input(state: &ScreenState) -> ScreenRow {
    let Some(ref last) = state.last_input else {
        return ScreenRow::text(clip("IN   --"));
    };
    let who = match (last.short_address, last.instance) {
        (None, _) => TargetTag::from_fmt(format_args!("SCH0")),
        (Some(a), None) => TargetTag::from_fmt(format_args!("A{a:02}")),
        (Some(a), Some(i)) => TargetTag::from_fmt(format_args!("A{a:02}.{i}")),
    };
    ScreenRow::text(RowText::from_fmt(format_args!("IN   {who} {}", last.what)))
}

fn row_live(state: &ScreenState) -> ScreenRow {
    if let Some(ref op) = state.operation {
        return operation_row(op);
    }
    let Some(ref act) = state.last_action else {
        return ScreenRow::text(clip("OK   --"));
    };
    let age = fmt_age(act.age_ms);
    if act.failed {
        return ScreenRow {
            text: fit(&RowText::from_fmt(format_args!("! {} {}", act.target, act.what)), &age),
            invert: true,
            bar: None,
            spark: false,
        };
    }
    let src = act.source.map(source_label).unwrap_or("");
    let with_src = RowText::from_fmt(format_args!("OK {} {} {src}", act.target, act.what));
    let head = if with_src.len() + 1 + age.len() <= SCREEN_COLS {
        with_src
    } else {
        RowText::from_fmt(format_args!("OK {} {}", act.target, act.what))
    };
    ScreenRow::text(fit(head.trim_end(), &age))
}

fn operation_row(op: &OperationLine) -> ScreenRow {
    let head = if op.total > 0 {
        RowText::from_fmt(format_args!("{} {}/{}", op.name, op.done, op.total))
    } else {
        RowText::from_fmt(format_args!("{}", op.name))
    };
    let permille = if op.total > 0 {
        Some(
            (u32::from(op.done) * BAR_FULL_PERMILLE / u32::from(op.total))
                .min(BAR_FULL_PERMILLE) as u16,
        )
    } else {
        None
    };
    ScreenRow { text: head, invert: false, bar: permille, spark: false }
}

fn source_label(source: RuntimeSource) -> &'static str {
    match source {
        RuntimeSource::Poller => "POL",
        RuntimeSource::Sniffer => "BUS",
        RuntimeSource::Api => "WEB",
        RuntimeSource::Mqtt => "HA",
        RuntimeSource::Hcl => "HCL",
        RuntimeSource::Cluster => "CLU",
        RuntimeSource::AdapterProxy => "PRX",
        RuntimeSource::Rules => "RUL",
        RuntimeSource::Readback => "RD",
    }
}

pub fn fmt_age(ms: u32) -> TargetTag {
    let secs = ms / MS_PER_SECOND;
    match secs {
        0..=AGE_SECONDS_MAX => TargetTag::from_fmt(format_args!("{secs}S")),
        100..=AGE_MINUTES_MAX_SECONDS => {
            TargetTag::from_fmt(format_args!("{}M", secs / SECONDS_PER_MINUTE))
        }
        _ => TargetTag::from_fmt(format_args!(
            "{}H",
            (secs / SECONDS_PER_HOUR).min(TWO_DIGIT_MAX)
        )),
    }
}

fn fmt_uptime(secs: u32) -> TargetTag {
    let days = secs / SECONDS_PER_DAY;
    let hours = (secs % SECONDS_PER_DAY) / SECONDS_PER_HOUR;
    if days > 0 {
        TargetTag::from_fmt(format_args!("{days}D{hours:02}H"))
    } else {
        TargetTag::from_fmt(format_args!(
            "{hours:02}H{:02}M",
            (secs % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE
        ))
    }
}

fn spread(left: &str, right: &str) -> RowText {
    spread_within(SCREEN_COLS, left, right)
}

fn spread_within(cols: usize, left: &str, right: &str) -> RowText {
    let (l, r) = (left.chars().count(), right.chars().count());
    let mut out = RowText::new();
    let _ = out.write_str(left);
    if l + r + 1 > cols {
        let _ = out.write_str(" ");
    } else {
        for _ in 0..cols - l - r {
            let _ = out.write_str(" ");
        }
    }
    let _ = out.write_str(right);
    out.truncate(cols);
    out
}

fn clip(text: &str) -> RowText {
    let mut out = RowText::new();
    let _ = out.write_str(text);
    out
}

fn fit(head: &str, tail: &str) -> RowText {
    let t = tail.chars().count();
    let room = SCREEN_COLS.saturating_sub(t + 1);
    let mut out = RowText::new();
    for ch in head.chars().take(room) {
        let mut one = [0u8; 4];
        let _ = out.write_str(ch.encode_utf8(&mut one));
    }
    while out.ends_with_space() {
        out.pop();
    }
    let _ = out.write_str(" ");
    let _ = out.write_str(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed<const N: usize>(text: &str) -> FixedAscii<N> {
        FixedAscii::from_fmt(format_args!("{text}"))
    }

    fn busy_state() -> ScreenState {
        ScreenState {
            ip: Some([192, 168, 1, 42]),
            uptime_s: 3 * 86_400 + 4 * 3600,
            sample: DisplaySample {
                gear_known: 64,
                gear_unreachable: 9,
                gear_faulted: 7,
                input_known: 12,
                input_present: 11,
                adapter_enabled: true,
                time_synced: true,
                ws_clients: 4,
                mqtt_connected: true,
                hcl_overrides_active: 63,
                ..DisplaySample::default()
            },
            bus: BusReading::SeveralFailures,
            frames_per_min: 9999,
            foreign_per_min: 9999,
            load_history: [500; SPARK_COLS],
            spark: [3; SPARK_COLS],
            poller: PollerLine { enabled: true, interval_ms: 999_000, failing: true, absent: true },
            hcl_kelvin: Some(6500),
            last_input: Some(InputLine {
                short_address: Some(63),
                instance: Some(31),
                what: fixed("LONG END"),
            }),
            last_action: Some(ActionLine {
                target: fixed("A63"),
                what: fixed("->254"),
                source: Some(RuntimeSource::AdapterProxy),
                failed: false,
                age_ms: 99_000,
            }),
            operation: None,
        }
    }

    #[test]
    fn no_row_exceeds_the_column_budget() {
        let mut state = busy_state();
        for rows in [render(&state), {
            state.operation = Some(OperationLine {
                name: fixed("SCAN103"),
                done: 999,
                total: 999,
            });
            render(&state)
        }] {
            for (i, row) in rows.iter().enumerate() {
                let budget = if row.spark { COLS_WITH_SPARK } else { SCREEN_COLS };
                assert!(
                    row.text.chars().count() <= budget,
                    "row {i} is {} columns of {budget}: {:?}",
                    row.text.chars().count(),
                    row.text
                );
            }
        }
    }

    #[test]
    fn the_bus_row_keeps_its_text_beside_the_histogram() {
        let mut state = busy_state();
        state.bus = BusReading::Clear;
        state.frames_per_min = 98;
        state.foreign_per_min = 0;
        let rows = render(&state);
        assert_eq!(rows[2].text, "BUS  OK   98F/M F0");
        assert!(rows[2].spark);
    }

    #[test]
    fn load_history_quantises_to_column_heights() {
        let mut history = [0u16; SPARK_COLS];
        history[0] = 1;
        history[1] = 166;
        history[2] = 500;
        history[3] = 1000;
        history[4] = 1200;
        let mut heights = [u8::MAX; SPARK_COLS];
        spark_heights_into(&history, &mut heights);
        assert_eq!(&heights[..5], &[1, 1, 3, 6, 6]);
        assert!(heights[5..].iter().all(|h| *h == 0));
    }

    #[test]
    fn alarmed_and_disabled_bus_rows_still_carry_the_histogram() {
        let mut state = busy_state();
        let rows = render(&state);
        assert!(rows[2].invert);
        assert!(rows[2].spark);
        state.sample.adapter_enabled = false;
        let rows = render(&state);
        assert_eq!(rows[2].text, "BUS  DISABLED");
        assert!(rows[2].spark);
        assert!(rows.iter().enumerate().all(|(i, r)| r.spark == (i == 2)));
    }

    #[test]
    fn address_row_spreads_ip_and_uptime() {
        let rows = render(&busy_state());
        assert_eq!(rows[0].text, "192.168.1.42    3D04H");
        assert!(!rows[0].invert);
    }

    #[test]
    fn a_missing_address_says_why_and_inverts() {
        let state = ScreenState { ip: None, ..ScreenState::default() };
        let rows = render(&state);
        assert_eq!(rows[0].text, "NO IP  LINK DOWN");
        assert!(rows[0].invert);
    }

    #[test]
    fn devices_row_counts_both_worlds() {
        let mut state = busy_state();
        state.sample.gear_known = 12;
        state.sample.gear_unreachable = 0;
        state.sample.gear_faulted = 0;
        state.sample.input_known = 3;
        state.sample.input_present = 3;
        let rows = render(&state);
        assert_eq!(rows[1].text, "GEAR 12/12     IN 3/3");
        assert!(!rows[1].invert, "no failure, no alarm treatment");
    }

    #[test]
    fn a_disabled_adapter_takes_the_bus_row() {
        let mut state = busy_state();
        state.sample.adapter_enabled = false;
        let rows = render(&state);
        assert_eq!(rows[2].text, "BUS  DISABLED");
        assert!(rows[2].invert);
    }

    #[test]
    fn an_unprobed_bus_is_not_reported_as_healthy() {
        let mut state = busy_state();
        state.bus = BusReading::Unprobed;
        state.frames_per_min = 12;
        state.foreign_per_min = 0;
        let rows = render(&state);
        assert!(rows[2].text.starts_with("BUS  ?"), "got {:?}", rows[2].text);
        assert!(!rows[2].invert, "unknown is not an alarm");
    }

    #[test]
    fn a_void_probe_reads_as_an_alarm() {
        let mut state = busy_state();
        state.bus = BusReading::from_probe(false, BusHealthVerdict::Clear);
        let rows = render(&state);
        assert!(rows[2].text.contains("NO ANSWER"));
        assert!(rows[2].invert);
    }

    #[test]
    fn hcl_says_stopped_when_the_clock_never_landed() {
        let mut state = busy_state();
        state.sample.time_synced = false;
        let rows = render(&state);
        assert_eq!(rows[3].text, "HCL  STOPPED NO TIME");
    }

    #[test]
    fn poller_off_is_not_dressed_up_as_healthy() {
        let mut state = busy_state();
        state.poller = PollerLine { enabled: false, interval_ms: 0, failing: false, absent: false };
        let rows = render(&state);
        assert!(rows[4].text.starts_with("POLL OFF"), "got {:?}", rows[4].text);
    }

    #[test]
    fn an_identityless_event_is_named_as_such() {
        let mut state = busy_state();
        state.last_input = Some(InputLine {
            short_address: None,
            instance: None,
            what: fixed("SHORT"),
        });
        let rows = render(&state);
        assert_eq!(rows[5].text, "IN   SCH0 SHORT");
    }

    #[test]
    fn the_widest_input_row_keeps_its_verb() {
        let rows = render(&busy_state());
        assert_eq!(rows[5].text, "IN   A63.31 LONG END");
    }

    #[test]
    fn the_live_row_keeps_its_age_whole() {
        let mut state = busy_state();
        state.last_action = Some(ActionLine {
            target: fixed("A03"),
            what: fixed("ON 254"),
            source: Some(RuntimeSource::Api),
            failed: false,
            age_ms: 12_000,
        });
        let rows = render(&state);
        assert_eq!(rows[6].text, "OK A03 ON 254 WEB 12S");
        state.last_action = Some(ActionLine {
            target: fixed("G02"),
            what: fixed("254 6500K"),
            source: Some(RuntimeSource::Sniffer),
            failed: false,
            age_ms: 125_000,
        });
        let rows = render(&state);
        assert_eq!(rows[6].text, "OK G02 254 6500K 2M", "the source goes before the age does");
        assert!(rows[6].text.chars().count() <= SCREEN_COLS);
        state.last_action = Some(ActionLine {
            target: fixed("WRITE"),
            what: fixed("VERIFYUNANSW"),
            source: None,
            failed: true,
            age_ms: 12_000,
        });
        let rows = render(&state);
        assert!(rows[6].text.ends_with(" 12S"), "got {:?}", rows[6].text);
        assert_eq!(rows[6].text.chars().count(), SCREEN_COLS);
    }

    #[test]
    fn a_persistence_failure_takes_the_services_row() {
        let mut state = busy_state();
        state.sample.persistence_failed = 3;
        let rows = render(&state);
        assert!(rows[4].text.ends_with("NO SAVE"), "got {:?}", rows[4].text);
        assert!(rows[4].text.starts_with("POLL 999S ER"), "got {:?}", rows[4].text);
        assert!(rows[4].invert);
    }

    #[test]
    fn absent_reads_show_on_the_poller_row_beneath_a_failure() {
        let mut state = busy_state();
        state.poller = PollerLine { enabled: true, interval_ms: 5000, failing: false, absent: true };
        assert!(render(&state)[4].text.starts_with("POLL 5S NA"));
        state.poller.failing = true;
        assert!(render(&state)[4].text.starts_with("POLL 5S ER"));
    }

    #[test]
    fn a_running_operation_takes_the_live_row_with_a_bar() {
        let mut state = busy_state();
        state.operation = Some(OperationLine { name: fixed("SCAN"), done: 37, total: 64 });
        let rows = render(&state);
        assert_eq!(rows[6].text, "SCAN 37/64");
        assert_eq!(rows[6].bar, Some(578));
    }

    #[test]
    fn a_failed_action_inverts_and_keeps_its_code() {
        let mut state = busy_state();
        state.last_action = Some(ActionLine {
            target: fixed("A03"),
            what: fixed("VERIFY FAIL"),
            source: None,
            failed: true,
            age_ms: 120_000,
        });
        let rows = render(&state);
        assert_eq!(rows[6].text, "! A03 VERIFY FAIL 2M");
        assert!(rows[6].invert);
    }

    #[test]
    fn age_stays_two_characters_wherever_it_can() {
        assert_eq!(fmt_age(0), "0S");
        assert_eq!(fmt_age(12_000), "12S");
        assert_eq!(fmt_age(99_000), "99S");
        assert_eq!(fmt_age(120_000), "2M");
        assert_eq!(fmt_age(5_999_000), "99M");
        assert_eq!(fmt_age(6_000_000), "1H");
        assert_eq!(fmt_age(u32::MAX), "99H");
    }
}
