use core::f64::consts::PI;

const SUNRISE_ZENITH_DEG: f64 = 90.833;
const MINUTES_PER_DEGREE: f64 = 4.0;
const SOLAR_NOON_MINUTES: f64 = 720.0;
const DAYS_PER_YEAR: f64 = 365.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}

impl Location {
    pub fn from_microdeg(latitude: Option<i32>, longitude: Option<i32>) -> Option<Self> {
        Some(Self {
            latitude_deg: f64::from(latitude?) / 1_000_000.0,
            longitude_deg: f64::from(longitude?) / 1_000_000.0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarEvents {
    pub sunrise_minutes: Option<i32>,
    pub sunset_minutes: Option<i32>,
}

fn fractional_year(year_day: u16) -> f64 {
    2.0 * PI * (f64::from(year_day) - 1.0) / DAYS_PER_YEAR
}

fn equation_of_time(gamma: f64) -> f64 {
    229.18
        * (0.000_075 + 0.001_868 * gamma.cos()
            - 0.032_077 * gamma.sin()
            - 0.014_615 * (2.0 * gamma).cos()
            - 0.040_849 * (2.0 * gamma).sin())
}

fn solar_declination(gamma: f64) -> f64 {
    0.006_918 - 0.399_912 * gamma.cos() + 0.070_257 * gamma.sin()
        - 0.006_758 * (2.0 * gamma).cos()
        + 0.000_907 * (2.0 * gamma).sin()
        - 0.002_697 * (3.0 * gamma).cos()
        + 0.001_48 * (3.0 * gamma).sin()
}

fn sunrise_hour_angle_deg(latitude_deg: f64, declination: f64) -> Option<f64> {
    let latitude = latitude_deg.to_radians();
    let zenith = SUNRISE_ZENITH_DEG.to_radians();
    let cos_hour_angle =
        zenith.cos() / (latitude.cos() * declination.cos()) - latitude.tan() * declination.tan();
    if !(-1.0..=1.0).contains(&cos_hour_angle) {
        return None;
    }
    Some(cos_hour_angle.acos().to_degrees())
}

pub fn solar_events(
    location: Location,
    year_day: u16,
    utc_offset_minutes: i16,
) -> SolarEvents {
    let gamma = fractional_year(year_day);
    let eq_time = equation_of_time(gamma);
    let declination = solar_declination(gamma);
    let Some(hour_angle) = sunrise_hour_angle_deg(location.latitude_deg, declination) else {
        return SolarEvents {
            sunrise_minutes: None,
            sunset_minutes: None,
        };
    };
    let offset = f64::from(utc_offset_minutes);
    let event = |angle: f64| {
        let utc_minutes =
            SOLAR_NOON_MINUTES - MINUTES_PER_DEGREE * (location.longitude_deg + angle) - eq_time;
        Some((utc_minutes + offset).round() as i32)
    };
    SolarEvents {
        sunrise_minutes: event(hour_angle),
        sunset_minutes: event(-hour_angle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOSCOW: Location = Location {
        latitude_deg: 55.7558,
        longitude_deg: 37.6173,
    };
    const MOSCOW_UTC_OFFSET: i16 = 180;
    const SUMMER_SOLSTICE: u16 = 172;
    const WINTER_SOLSTICE: u16 = 355;

    fn assert_within(actual: Option<i32>, expected: i32, tolerance: i32, what: &str) {
        let actual = actual.unwrap_or_else(|| panic!("{what} should happen"));
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: got {actual} min, expected {expected} +/- {tolerance}"
        );
    }

    #[test]
    fn moscow_summer_solstice_matches_the_almanac() {
        let events = solar_events(MOSCOW, SUMMER_SOLSTICE, MOSCOW_UTC_OFFSET);
        assert_within(events.sunrise_minutes, 3 * 60 + 44, 2, "Moscow sunrise 21 Jun");
        assert_within(events.sunset_minutes, 21 * 60 + 18, 2, "Moscow sunset 21 Jun");
    }

    #[test]
    fn moscow_winter_solstice_matches_the_almanac() {
        let events = solar_events(MOSCOW, WINTER_SOLSTICE, MOSCOW_UTC_OFFSET);
        assert_within(events.sunrise_minutes, 8 * 60 + 57, 3, "Moscow sunrise 21 Dec");
        assert_within(events.sunset_minutes, 15 * 60 + 57, 3, "Moscow sunset 21 Dec");
    }

    #[test]
    fn greenwich_equinox_sunrise_is_around_six() {
        let greenwich = Location {
            latitude_deg: 51.4779,
            longitude_deg: 0.0,
        };
        let events = solar_events(greenwich, 79, 0);
        assert_within(events.sunrise_minutes, 6 * 60 + 2, 3, "Greenwich sunrise");
        assert_within(events.sunset_minutes, 18 * 60 + 12, 3, "Greenwich sunset");
    }

    #[test]
    fn the_polar_summer_reports_no_sunrise_rather_than_a_guess() {
        let longyearbyen = Location {
            latitude_deg: 78.22,
            longitude_deg: 15.65,
        };
        let midnight_sun = solar_events(longyearbyen, SUMMER_SOLSTICE, 60);
        assert_eq!(midnight_sun.sunrise_minutes, None);
        assert_eq!(midnight_sun.sunset_minutes, None);

        let polar_night = solar_events(longyearbyen, WINTER_SOLSTICE, 60);
        assert_eq!(polar_night.sunrise_minutes, None, "the sun never rises");
    }

    #[test]
    fn the_southern_hemisphere_has_its_seasons_the_other_way_round() {
        let sydney = Location {
            latitude_deg: -33.8688,
            longitude_deg: 151.2093,
        };
        let june = solar_events(sydney, SUMMER_SOLSTICE, 600);
        let december = solar_events(sydney, WINTER_SOLSTICE, 600);
        let day_length = |events: SolarEvents| {
            events.sunset_minutes.expect("sunset") - events.sunrise_minutes.expect("sunrise")
        };
        assert!(
            day_length(june) < day_length(december),
            "June is the short day in Sydney: {} vs {}",
            day_length(june),
            day_length(december)
        );
    }

    #[test]
    fn location_needs_both_coordinates() {
        assert!(Location::from_microdeg(Some(55_755_800), None).is_none());
        assert!(Location::from_microdeg(None, Some(37_617_300)).is_none());
        let both = Location::from_microdeg(Some(55_755_800), Some(37_617_300)).expect("location");
        assert!((both.latitude_deg - 55.7558).abs() < 1e-9);
    }
}
