use dali2rust_contracts::msg::LevelTransition;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelContext {
    pub target_level: Option<u8>,
    pub min_level: Option<u8>,
    pub max_level: Option<u8>,
    pub last_active_level: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionOutcome {
    Level(u8),
    Unchanged,
    Unknown,
}

#[must_use]
pub fn resolve(transition: LevelTransition, ctx: &LevelContext) -> TransitionOutcome {
    match transition {
        LevelTransition::RecallMaxLevel => absolute(ctx.max_level, ctx.target_level),
        LevelTransition::RecallMinLevel => absolute(ctx.min_level, ctx.target_level),
        LevelTransition::GoToLastActiveLevel => absolute(ctx.last_active_level, ctx.target_level),
        LevelTransition::StepUp
        | LevelTransition::StepDown
        | LevelTransition::StepDownAndOff
        | LevelTransition::OnAndStepUp => step(transition, ctx),
    }
}

fn absolute(value: Option<u8>, stored: Option<u8>) -> TransitionOutcome {
    match (value, stored) {
        (None, _) => TransitionOutcome::Unknown,
        (Some(level), Some(stored)) if level == stored => TransitionOutcome::Unchanged,
        (Some(level), _) => TransitionOutcome::Level(level),
    }
}

// IEC 62386-102 §11.3.5, §11.3.6, §11.3.9, §11.3.10
fn step(transition: LevelTransition, ctx: &LevelContext) -> TransitionOutcome {
    let (Some(target), Some(min), Some(max)) = (ctx.target_level, ctx.min_level, ctx.max_level)
    else {
        return TransitionOutcome::Unknown;
    };
    if min > max {
        return TransitionOutcome::Unknown;
    }
    match transition {
        LevelTransition::StepUp => match target {
            0 => TransitionOutcome::Unchanged,
            t if t >= max => TransitionOutcome::Unchanged,
            t if t >= min => TransitionOutcome::Level(t.saturating_add(1)),
            _ => TransitionOutcome::Unknown,
        },
        LevelTransition::StepDown => match target {
            0 => TransitionOutcome::Unchanged,
            t if t <= min => TransitionOutcome::Unchanged,
            t if t <= max => TransitionOutcome::Level(t.saturating_sub(1)),
            _ => TransitionOutcome::Unknown,
        },
        LevelTransition::StepDownAndOff => match target {
            0 => TransitionOutcome::Unchanged,
            t if t <= min => TransitionOutcome::Level(0),
            t if t <= max => TransitionOutcome::Level(t.saturating_sub(1)),
            _ => TransitionOutcome::Unknown,
        },
        LevelTransition::OnAndStepUp => match target {
            0 => TransitionOutcome::Level(min),
            t if t >= max => TransitionOutcome::Unchanged,
            t if t >= min => TransitionOutcome::Level(t.saturating_add(1)),
            _ => TransitionOutcome::Unknown,
        },
        LevelTransition::RecallMaxLevel
        | LevelTransition::RecallMinLevel
        | LevelTransition::GoToLastActiveLevel => TransitionOutcome::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(target: u8, min: u8, max: u8) -> LevelContext {
        LevelContext {
            target_level: Some(target),
            min_level: Some(min),
            max_level: Some(max),
            last_active_level: Some(200),
        }
    }

    #[test]
    fn a_step_up_walks_one_and_holds_at_the_ceiling() {
        assert_eq!(
            resolve(LevelTransition::StepUp, &ctx(100, 10, 254)),
            TransitionOutcome::Level(101)
        );
        assert_eq!(
            resolve(LevelTransition::StepUp, &ctx(254, 10, 254)),
            TransitionOutcome::Unchanged
        );
    }

    #[test]
    fn a_step_up_from_dark_stays_dark_and_on_and_step_up_does_not() {
        assert_eq!(
            resolve(LevelTransition::StepUp, &ctx(0, 10, 254)),
            TransitionOutcome::Unchanged
        );
        assert_eq!(
            resolve(LevelTransition::OnAndStepUp, &ctx(0, 10, 254)),
            TransitionOutcome::Level(10)
        );
    }

    #[test]
    fn step_down_holds_at_min_and_step_down_and_off_switches_off() {
        assert_eq!(
            resolve(LevelTransition::StepDown, &ctx(10, 10, 254)),
            TransitionOutcome::Unchanged
        );
        assert_eq!(
            resolve(LevelTransition::StepDownAndOff, &ctx(10, 10, 254)),
            TransitionOutcome::Level(0)
        );
    }

    #[test]
    fn a_step_without_bounds_is_unknown_not_a_guess() {
        let bare = LevelContext {
            target_level: Some(100),
            ..LevelContext::default()
        };
        for verb in [
            LevelTransition::StepUp,
            LevelTransition::StepDown,
            LevelTransition::StepDownAndOff,
            LevelTransition::OnAndStepUp,
        ] {
            assert_eq!(resolve(verb, &bare), TransitionOutcome::Unknown, "{verb:?}");
        }
    }

    #[test]
    fn the_absolute_verbs_read_one_attribute_each() {
        assert_eq!(
            resolve(LevelTransition::RecallMaxLevel, &ctx(100, 10, 254)),
            TransitionOutcome::Level(254)
        );
        assert_eq!(
            resolve(LevelTransition::RecallMinLevel, &ctx(10, 10, 254)),
            TransitionOutcome::Unchanged
        );
        assert_eq!(
            resolve(LevelTransition::GoToLastActiveLevel, &ctx(100, 10, 254)),
            TransitionOutcome::Level(200)
        );
        let never_lit = LevelContext {
            target_level: Some(0),
            ..LevelContext::default()
        };
        assert_eq!(
            resolve(LevelTransition::GoToLastActiveLevel, &never_lit),
            TransitionOutcome::Unknown,
            "a gear never seen lit has no level to recall"
        );
    }
}
