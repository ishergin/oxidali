const HALF_SEQUENCE_SPACE: u32 = 1 << 31;

const NEWEST_ONLY: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    Fresh { skipped: u32 },
    Late,
    Duplicate,
    TooOld,
    Restarted,
}

impl Arrival {
    pub const fn is_new(self) -> bool {
        matches!(self, Self::Fresh { .. } | Self::Late | Self::Restarted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceWindow {
    epoch: u32,
    left: Option<u32>,
    highest: u32,
    seen: u64,
}

impl SequenceWindow {
    pub const fn starting_at(epoch: u32, seq: u32) -> Self {
        Self { epoch, left: None, highest: seq, seen: NEWEST_ONLY }
    }

    pub fn admit(&mut self, epoch: u32, seq: u32) -> Arrival {
        if epoch != self.epoch {
            return self.restart(epoch, seq);
        }
        let ahead = seq.wrapping_sub(self.highest);
        match ahead {
            0 => Arrival::Duplicate,
            1..HALF_SEQUENCE_SPACE => self.advance(seq, ahead),
            _ => self.look_back(self.highest.wrapping_sub(seq)),
        }
    }

    fn restart(&mut self, epoch: u32, seq: u32) -> Arrival {
        if self.left == Some(epoch) {
            return Arrival::TooOld;
        }
        *self = Self { left: Some(self.epoch), ..Self::starting_at(epoch, seq) };
        Arrival::Restarted
    }

    fn advance(&mut self, seq: u32, ahead: u32) -> Arrival {
        self.seen = self.seen.checked_shl(ahead).unwrap_or(0) | NEWEST_ONLY;
        self.highest = seq;
        Arrival::Fresh { skipped: ahead - 1 }
    }

    fn look_back(&mut self, behind: u32) -> Arrival {
        let Some(bit) = NEWEST_ONLY.checked_shl(behind) else {
            return Arrival::TooOld;
        };
        if self.seen & bit != 0 {
            return Arrival::Duplicate;
        }
        self.seen |= bit;
        Arrival::Late
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: u32 = 0x5eed_0001;
    const REBOOTED: u32 = 0x5eed_0002;
    const WINDOW: u32 = u64::BITS;

    #[test]
    fn the_second_copy_of_a_frame_is_a_duplicate() {
        let mut window = SequenceWindow::starting_at(EPOCH, 10);
        assert_eq!(window.admit(EPOCH, 11), Arrival::Fresh { skipped: 0 });
        assert_eq!(window.admit(EPOCH, 11), Arrival::Duplicate);
        assert_eq!(window.admit(EPOCH, 10), Arrival::Duplicate);
    }

    #[test]
    fn a_gap_ahead_counts_the_numbers_that_did_not_arrive() {
        let mut window = SequenceWindow::starting_at(EPOCH, 10);
        assert_eq!(window.admit(EPOCH, 15), Arrival::Fresh { skipped: 4 });
    }

    #[test]
    fn a_number_from_the_gap_arrives_late_once_and_is_a_duplicate_after() {
        let mut window = SequenceWindow::starting_at(EPOCH, 10);
        window.admit(EPOCH, 15);
        assert_eq!(window.admit(EPOCH, 12), Arrival::Late);
        assert_eq!(window.admit(EPOCH, 12), Arrival::Duplicate);
    }

    #[test]
    fn the_window_reaches_sixty_four_back_and_no_further() {
        let mut window = SequenceWindow::starting_at(EPOCH, 1_000);
        assert_eq!(window.admit(EPOCH, 1_000 - (WINDOW - 1)), Arrival::Late);
        assert_eq!(window.admit(EPOCH, 1_000 - WINDOW), Arrival::TooOld);
    }

    #[test]
    fn a_jump_past_the_window_forgets_everything_before_it() {
        let mut window = SequenceWindow::starting_at(EPOCH, 10);
        assert_eq!(window.admit(EPOCH, 10 + WINDOW + 36), Arrival::Fresh { skipped: WINDOW + 35 });
        assert_eq!(window.admit(EPOCH, 10 + WINDOW + 35), Arrival::Late);
        assert_eq!(window.admit(EPOCH, 10), Arrival::TooOld);
        assert_eq!(window.admit(EPOCH, 10 + WINDOW), Arrival::Late);
    }

    #[test]
    fn a_jump_of_exactly_the_window_keeps_nothing_but_the_newest() {
        let mut window = SequenceWindow::starting_at(EPOCH, 10);
        window.admit(EPOCH, 11);
        assert_eq!(window.admit(EPOCH, 11 + WINDOW), Arrival::Fresh { skipped: WINDOW - 1 });
        assert_eq!(window.admit(EPOCH, 11 + WINDOW - 1), Arrival::Late);
        assert_eq!(window.admit(EPOCH, 12), Arrival::Late);
        assert_eq!(window.admit(EPOCH, 11), Arrival::TooOld);
    }

    #[test]
    fn the_sequence_wraps_without_looking_like_an_old_frame() {
        let mut window = SequenceWindow::starting_at(EPOCH, u32::MAX);
        assert_eq!(window.admit(EPOCH, 0), Arrival::Fresh { skipped: 0 });
        assert_eq!(window.admit(EPOCH, u32::MAX), Arrival::Duplicate);
        assert_eq!(window.admit(EPOCH, u32::MAX - 1), Arrival::Late);
    }

    #[test]
    fn a_number_half_the_space_away_is_behind_not_ahead() {
        let mut window = SequenceWindow::starting_at(EPOCH, 0);
        assert_eq!(window.admit(EPOCH, HALF_SEQUENCE_SPACE), Arrival::TooOld);
        assert_eq!(window.admit(EPOCH, HALF_SEQUENCE_SPACE - 1), Arrival::Fresh {
            skipped: HALF_SEQUENCE_SPACE - 2
        });
    }

    #[test]
    fn a_new_boot_epoch_starts_a_new_window_even_at_a_number_already_seen() {
        let mut window = SequenceWindow::starting_at(EPOCH, 500);
        assert_eq!(window.admit(REBOOTED, 1), Arrival::Restarted);
        assert_eq!(window.admit(REBOOTED, 1), Arrival::Duplicate);
        assert_eq!(window.admit(REBOOTED, 2), Arrival::Fresh { skipped: 0 });
    }

    #[test]
    fn a_late_frame_from_the_epoch_just_left_is_too_old_not_a_restart() {
        let mut window = SequenceWindow::starting_at(EPOCH, 5);
        assert_eq!(window.admit(REBOOTED, 1), Arrival::Restarted);
        assert_eq!(window.admit(EPOCH, 5), Arrival::TooOld);
        assert_eq!(window.admit(REBOOTED, 1), Arrival::Duplicate);
    }

    #[test]
    fn only_a_frame_never_seen_counts_as_new() {
        assert!(Arrival::Fresh { skipped: 3 }.is_new());
        assert!(Arrival::Late.is_new());
        assert!(Arrival::Restarted.is_new());
        assert!(!Arrival::Duplicate.is_new());
        assert!(!Arrival::TooOld.is_new());
    }
}
