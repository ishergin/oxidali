use crate::cursor::Cursor;
use crate::lexer::TokenKind;
use dali2rust_rules_model::{CompileError, DaySet, SolarEvent, TimeBound, TimeOfDay, Weekday};

pub fn time_of_day(c: &mut Cursor<'_>) -> Result<TimeOfDay, CompileError> {
    let pos = c.here();
    match c.advance() {
        Some(t) => match t.kind {
            TokenKind::Time { hour, minute } => Ok(TimeOfDay { hour, minute }),
            _ => Err(pos.err("expected HH:MM")),
        },
        None => Err(pos.err("expected HH:MM")),
    }
}

pub fn all_days() -> DaySet {
    DaySet::ALL
}

fn weekday(c: &mut Cursor<'_>) -> Result<Weekday, CompileError> {
    let (word, pos) = c.expect_ident("a day (mon..sun)")?;
    Weekday::from_name(&word).ok_or_else(|| pos.err(format!("unknown day \"{word}\"")))
}

pub fn day_set(c: &mut Cursor<'_>) -> Result<DaySet, CompileError> {
    let mut days = DaySet::empty();
    loop {
        let from = weekday(c)?;
        if c.accept(&TokenKind::DotDot) {
            let to = weekday(c)?;
            days.insert_range(from, to);
        } else {
            days.insert(from);
        }
        if !c.accept(&TokenKind::Comma) {
            return Ok(days);
        }
    }
}

pub fn solar_with_offset(c: &mut Cursor<'_>) -> Result<(SolarEvent, i32), CompileError> {
    let (word, pos) = c.expect_ident("HH:MM, sunrise or sunset")?;
    let event = match word.as_str() {
        "sunrise" => SolarEvent::Sunrise,
        "sunset" => SolarEvent::Sunset,
        _ => return Err(pos.err(format!("expected HH:MM, sunrise or sunset, got \"{word}\""))),
    };
    let sign: i64 = if c.accept(&TokenKind::Plus) {
        1
    } else if c.accept(&TokenKind::Minus) {
        -1
    } else {
        return Ok((event, 0));
    };
    let (ms, dur_pos) = c.expect_duration("solar offset")?;
    let offset = i32::try_from(sign * i64::from(ms))
        .map_err(|_| dur_pos.err("solar offset too large"))?;
    Ok((event, offset))
}

pub fn time_bound(c: &mut Cursor<'_>) -> Result<TimeBound, CompileError> {
    if let Some(TokenKind::Time { .. }) = c.peek().map(|t| &t.kind) {
        return Ok(TimeBound::Clock(time_of_day(c)?));
    }
    let (event, offset_ms) = solar_with_offset(c)?;
    Ok(TimeBound::Solar { event, offset_ms })
}
