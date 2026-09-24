use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DurationMs(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl Serialize for TimeOfDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:02}:{:02}", self.hour, self.minute))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SolarEvent {
    Sunrise,
    Sunset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum TimeBound {
    Clock(TimeOfDay),
    Solar { event: SolarEvent, offset_ms: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

pub const WEEKDAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];

    pub fn index(self) -> u8 {
        match self {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            Weekday::Sat => 5,
            Weekday::Sun => 6,
        }
    }

    pub fn name(self) -> &'static str {
        WEEKDAY_NAMES[self.index() as usize]
    }

    pub fn from_name(name: &str) -> Option<Weekday> {
        Weekday::ALL.into_iter().find(|d| d.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySet(u8);

const ALL_DAYS_MASK: u8 = 0x7F;

impl DaySet {
    pub const ALL: DaySet = DaySet(ALL_DAYS_MASK);

    pub fn empty() -> DaySet {
        DaySet(0)
    }

    pub fn insert(&mut self, day: Weekday) {
        self.0 |= 1 << day.index();
    }

    pub fn insert_range(&mut self, from: Weekday, to: Weekday) {
        let mut d = from.index();
        loop {
            self.0 |= 1 << d;
            if d == to.index() {
                break;
            }
            d = (d + 1) % 7;
        }
    }

    pub fn contains(self, day: Weekday) -> bool {
        self.0 & (1 << day.index()) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 & ALL_DAYS_MASK == 0
    }
}

impl Serialize for DaySet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let days: Vec<&str> = Weekday::ALL
            .into_iter()
            .filter(|d| self.contains(*d))
            .map(Weekday::name)
            .collect();
        let mut seq = serializer.serialize_seq(Some(days.len()))?;
        for d in days {
            seq.serialize_element(d)?;
        }
        seq.end()
    }
}
