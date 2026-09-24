use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliTargetScope, HclLevelMode, HclTargetRow, HclTargetScope,
    LightSetpoint, PowerState,
};

use super::curve::DesiredState;

pub const MAX_COMMANDS_PER_TICK: usize = 32;
const DALI_GROUP_COUNT: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetKey {
    pub adapter_id: u8,
    pub scope: HclTargetScope,
    pub group_id: u8,
}

impl TargetKey {
    fn dali_scope(&self) -> DaliTargetScope {
        match self.scope {
            HclTargetScope::Group => DaliTargetScope::Group,
            HclTargetScope::Broadcast => DaliTargetScope::Broadcast,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesiredEntry {
    pub key: TargetKey,
    pub state: DesiredState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlannedCommand {
    TargetState {
        key: TargetKey,
        setpoint: LightSetpoint,
    },
    RecallLastActive { key: TargetKey },
}

impl PlannedCommand {
    pub fn key(&self) -> TargetKey {
        match self {
            PlannedCommand::TargetState { key, .. } | PlannedCommand::RecallLastActive { key } => {
                *key
            }
        }
    }

    pub fn dali_scope(&self) -> DaliTargetScope {
        self.key().dali_scope()
    }
}

pub fn expand_target(row: &HclTargetRow) -> Vec<TargetKey> {
    match row.scope {
        HclTargetScope::Broadcast => vec![TargetKey {
            adapter_id: row.adapter_id,
            scope: HclTargetScope::Broadcast,
            group_id: 0,
        }],
        HclTargetScope::Group => (0..DALI_GROUP_COUNT)
            .filter(|group| row.group_mask & (1u16 << group) != 0)
            .map(|group_id| TargetKey {
                adapter_id: row.adapter_id,
                scope: HclTargetScope::Group,
                group_id,
            })
            .collect(),
    }
}

pub fn coalesce(entries: Vec<DesiredEntry>) -> Vec<DesiredEntry> {
    let mut coalesced: Vec<DesiredEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        match coalesced.iter_mut().find(|seen| seen.key == entry.key) {
            Some(seen) => *seen = entry,
            None => coalesced.push(entry),
        }
    }
    coalesced
}

pub fn commands_for(entry: &DesiredEntry) -> Vec<PlannedCommand> {
    let color = entry
        .state
        .color_temperature_kelvin
        .map(|kelvin| ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: kelvin,
            ..ColorValue::default()
        });
    match entry.state.level_mode {
        HclLevelMode::Absolute => vec![PlannedCommand::TargetState {
            key: entry.key,
            setpoint: absolute_setpoint(entry.state.level.unwrap_or(0), color),
        }],
        HclLevelMode::LastActive => color
            .map(|color| PlannedCommand::TargetState {
                key: entry.key,
                setpoint: color_only_setpoint(color),
            })
            .into_iter()
            .chain(std::iter::once(PlannedCommand::RecallLastActive {
                key: entry.key,
            }))
            .collect(),
        HclLevelMode::None => color
            .map(|color| PlannedCommand::TargetState {
                key: entry.key,
                setpoint: color_only_setpoint(color),
            })
            .into_iter()
            .collect(),
    }
}

fn absolute_setpoint(level: u8, color: Option<ColorValue>) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::for_level(level),
        level,
        color,
    }
}

fn color_only_setpoint(color: ColorValue) -> LightSetpoint {
    LightSetpoint {
        power: PowerState::Unknown,
        level: 0,
        color: Some(color),
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct TickPlan {
    pub commands: Vec<PlannedCommand>,
    pub dropped: usize,
}

pub fn plan(entries: &[DesiredEntry]) -> TickPlan {
    let mut commands: Vec<PlannedCommand> = Vec::new();
    let mut dropped = 0usize;
    for entry in entries.iter().filter(|entry| !entry.state.is_noop()) {
        let unit = commands_for(entry);
        if dropped > 0 || commands.len() + unit.len() > MAX_COMMANDS_PER_TICK {
            dropped += unit.len();
            continue;
        }
        commands.extend(unit);
    }
    TickPlan { commands, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_row(adapter_id: u8, groups: &[u8]) -> HclTargetRow {
        HclTargetRow {
            adapter_id,
            scope: HclTargetScope::Group,
            group_mask: groups.iter().fold(0u16, |mask, g| mask | (1u16 << g)),
        }
    }

    fn broadcast_row(adapter_id: u8) -> HclTargetRow {
        HclTargetRow {
            adapter_id,
            scope: HclTargetScope::Broadcast,
            group_mask: 0,
        }
    }

    fn entry(key: TargetKey, level_mode: HclLevelMode, level: Option<u8>, kelvin: Option<u16>) -> DesiredEntry {
        DesiredEntry {
            key,
            state: DesiredState {
                level_mode,
                level,
                color_temperature_kelvin: kelvin,
            },
        }
    }

    fn group_key(adapter_id: u8, group_id: u8) -> TargetKey {
        TargetKey {
            adapter_id,
            scope: HclTargetScope::Group,
            group_id,
        }
    }

    #[test]
    fn a_group_row_expands_to_one_target_per_group() {
        let keys = expand_target(&group_row(0, &[1, 5]));
        assert_eq!(keys, vec![group_key(0, 1), group_key(0, 5)]);
    }

    #[test]
    fn a_broadcast_row_is_one_target_whatever_the_mask_says() {
        let mut row = broadcast_row(1);
        row.group_mask = u16::MAX;
        assert_eq!(
            expand_target(&row),
            vec![TargetKey {
                adapter_id: 1,
                scope: HclTargetScope::Broadcast,
                group_id: 0,
            }]
        );
    }

    #[test]
    fn duplicate_targets_collapse_to_the_last_offer() {
        let first = entry(group_key(0, 1), HclLevelMode::Absolute, Some(80), Some(2700));
        let second = entry(group_key(0, 1), HclLevelMode::Absolute, Some(200), Some(4000));
        let other = entry(group_key(0, 2), HclLevelMode::Absolute, Some(50), None);
        let coalesced = coalesce(vec![first, second, other]);
        assert_eq!(coalesced.len(), 2, "one command per target");
        assert_eq!(coalesced[0].state.level, Some(200), "the later schedule wins");
    }

    #[test]
    fn broadcast_duplicates_coalesce_too() {
        let key = TargetKey {
            adapter_id: 0,
            scope: HclTargetScope::Broadcast,
            group_id: 0,
        };
        let coalesced = coalesce(vec![
            entry(key, HclLevelMode::None, None, Some(2700)),
            entry(key, HclLevelMode::None, None, Some(4000)),
        ]);
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].state.color_temperature_kelvin, Some(4000));
    }

    #[test]
    fn an_absolute_point_is_one_command_carrying_both_halves() {
        let commands = commands_for(&entry(
            group_key(0, 3),
            HclLevelMode::Absolute,
            Some(180),
            Some(3000),
        ));
        assert_eq!(commands.len(), 1);
        let PlannedCommand::TargetState { setpoint, .. } = &commands[0] else {
            panic!("expected a target-state command");
        };
        assert_eq!(setpoint.level, 180);
        assert_eq!(setpoint.power, PowerState::On);
        let color = setpoint.color.as_ref().expect("colour rides along");
        assert_eq!(color.color_temperature_kelvin, 3000);
    }

    #[test]
    fn an_absolute_zero_turns_the_target_off() {
        let commands = commands_for(&entry(group_key(0, 3), HclLevelMode::Absolute, Some(0), None));
        let PlannedCommand::TargetState { setpoint, .. } = &commands[0] else {
            panic!("expected a target-state command");
        };
        assert_eq!(setpoint.power, PowerState::Off);
    }

    #[test]
    fn last_active_sends_the_colour_first_then_the_recall() {
        let commands = commands_for(&entry(
            group_key(0, 3),
            HclLevelMode::LastActive,
            None,
            Some(2200),
        ));
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0], PlannedCommand::TargetState { .. }));
        assert!(matches!(commands[1], PlannedCommand::RecallLastActive { .. }));
    }

    #[test]
    fn last_active_without_a_colour_is_just_the_recall() {
        let commands = commands_for(&entry(group_key(0, 3), HclLevelMode::LastActive, None, None));
        assert_eq!(commands, vec![PlannedCommand::RecallLastActive { key: group_key(0, 3) }]);
    }

    #[test]
    fn a_colour_only_point_leaves_brightness_alone() {
        let commands = commands_for(&entry(group_key(0, 3), HclLevelMode::None, None, Some(2700)));
        let PlannedCommand::TargetState { setpoint, .. } = &commands[0] else {
            panic!("expected a target-state command");
        };
        assert_eq!(setpoint.power, PowerState::Unknown, "not an on or off order");
        assert_eq!(setpoint.level, 0, "no DAPC for a colour-only point");
    }

    #[test]
    fn a_point_with_nothing_to_say_produces_no_command() {
        let plan = plan(&[entry(group_key(0, 3), HclLevelMode::None, None, None)]);
        assert!(plan.commands.is_empty());
        assert_eq!(plan.dropped, 0);
    }

    #[test]
    fn the_cap_never_splits_an_entrys_pair() {
        let mut entries = vec![entry(group_key(0, 0), HclLevelMode::Absolute, Some(100), None)];
        entries.extend(
            (0..16).map(|i| entry(group_key(1, i), HclLevelMode::LastActive, None, Some(2700))),
        );
        let plan = plan(&entries);
        assert_eq!(
            plan.commands.len(),
            31,
            "the straddling pair is deferred whole, not cut in half"
        );
        assert_eq!(plan.dropped, 2, "both halves count as refused");
        let straddler = group_key(1, 15);
        assert!(
            plan.commands.iter().all(|cmd| cmd.key() != straddler),
            "neither half of the straddling pair is published this tick"
        );
    }

    #[test]
    fn the_tick_cap_truncates_and_counts_the_remainder() {
        let entries: Vec<DesiredEntry> = (0..20)
            .map(|i| {
                entry(
                    group_key(i / 16, i % 16),
                    HclLevelMode::LastActive,
                    None,
                    Some(2700),
                )
            })
            .collect();
        let plan = plan(&entries);
        assert_eq!(plan.commands.len(), MAX_COMMANDS_PER_TICK);
        assert_eq!(plan.dropped, 8);
    }
}
