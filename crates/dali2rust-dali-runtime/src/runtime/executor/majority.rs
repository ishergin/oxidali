pub fn majority_byte_from_u32_samples(samples: &[u32], shift: u32) -> Option<u8> {
    let mut counts = [0u8; 256];
    for &sample in samples {
        let value = ((sample >> shift) & 0xFF) as usize;
        counts[value] = counts[value].saturating_add(1);
    }
    unique_majority_byte(&counts)
}

pub fn majority_optional_byte(samples: &[Option<u8>]) -> Option<Option<u8>> {
    let mut counts = [0u8; 256];
    let mut none_count = 0u8;
    for sample in samples {
        match sample {
            Some(value) => {
                counts[*value as usize] = counts[*value as usize].saturating_add(1);
            }
            None => none_count = none_count.saturating_add(1),
        }
    }

    let byte_majority = best_unique_byte(&counts);
    let best_byte_count = counts.iter().copied().max().unwrap_or(0);
    match byte_majority {
        Some((value, count)) if count > none_count => Some(Some(value)),
        Some((_, count)) if count == none_count && none_count >= 2 => None,
        Some((value, _)) if none_count < 2 => Some(Some(value)),
        _ if none_count >= 2 && none_count > best_byte_count => Some(None),
        _ => None,
    }
}

pub fn majority_optional<T: Copy + Eq>(samples: &[Option<T>]) -> Option<Option<T>> {
    let mut tally: Vec<(T, u8)> = Vec::new();
    let mut none_count = 0u8;
    for sample in samples {
        match sample {
            Some(value) => match tally.iter_mut().find(|(seen, _)| seen == value) {
                Some((_, count)) => *count = count.saturating_add(1),
                None => tally.push((*value, 1)),
            },
            None => none_count = none_count.saturating_add(1),
        }
    }

    let best_count = tally.iter().map(|(_, c)| *c).max().unwrap_or(0);
    let tied = tally.iter().filter(|(_, c)| *c == best_count).count() > 1;
    let winner = (best_count >= 2 && !tied)
        .then(|| tally.iter().find(|(_, c)| *c == best_count))
        .flatten()
        .map(|(value, count)| (*value, *count));

    match winner {
        Some((value, count)) if count > none_count => Some(Some(value)),
        Some((_, count)) if count == none_count && none_count >= 2 => None,
        Some((value, _)) if none_count < 2 => Some(Some(value)),
        _ if none_count >= 2 && none_count > best_count => Some(None),
        _ => None,
    }
}

pub fn unique_majority_byte(counts: &[u8; 256]) -> Option<u8> {
    best_unique_byte(counts).map(|(value, _)| value)
}

fn best_unique_byte(counts: &[u8; 256]) -> Option<(u8, u8)> {
    let mut best = None;
    let mut tied = false;
    for (value, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        match best {
            None => {
                best = Some((value as u8, count));
                tied = false;
            }
            Some((_, best_count)) if count > best_count => {
                best = Some((value as u8, count));
                tied = false;
            }
            Some((_, best_count)) if count == best_count => tied = true,
            _ => {}
        }
    }
    match best {
        Some((value, count)) if count >= 2 && !tied => Some((value, count)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majority_optional_byte_confirms_value() {
        assert_eq!(
            majority_optional_byte(&[Some(0x11), Some(0x22), Some(0x11)]),
            Some(Some(0x11))
        );
    }

    #[test]
    fn majority_optional_byte_confirms_none() {
        assert_eq!(majority_optional_byte(&[None, Some(0x22), None]), Some(None));
    }

    #[test]
    fn majority_optional_byte_rejects_tie() {
        assert_eq!(majority_optional_byte(&[None, None, Some(0x22), Some(0x22)]), None);
    }

    #[test]
    fn the_generic_tally_agrees_with_the_byte_version() {
        let cases: &[&[Option<u8>]] = &[
            &[Some(0x11), Some(0x22), Some(0x11)],
            &[Some(0x11), Some(0x22)],
            &[None, None],
            &[None, None, Some(0x11)],
            &[Some(0x11), None],
            &[Some(0x11), Some(0x11), None, None],
            &[Some(0x11), Some(0x22), Some(0x33)],
            &[],
            &[Some(0x11)],
        ];
        for case in cases {
            assert_eq!(
                majority_optional(case),
                majority_optional_byte(case),
                "verdicts diverge for {case:?}"
            );
        }
    }

    #[test]
    fn a_wide_value_is_confirmed_by_the_same_rule() {
        assert_eq!(
            majority_optional(&[Some(370u16), Some(29185), Some(370)]),
            Some(Some(370))
        );
        assert_eq!(majority_optional(&[Some(370u16), Some(29185)]), None);
    }
}
