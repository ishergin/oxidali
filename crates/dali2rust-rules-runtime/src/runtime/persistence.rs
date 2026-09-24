use dali2rust_contracts::msg::MAX_RULES_SOURCE_BYTES;
use dali2rust_platform::slice_store::SliceKey;
use serde::{Deserialize, Serialize};

pub const RULES_MANIFEST_VERSION: u32 = 1;

pub const RULES_BANK_BYTES: usize = 4080;
pub const RULES_TEXT_BANKS: u8 = 3;

#[must_use]
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleManifestEntry {
    pub name_hash: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesManifest {
    pub version: u32,
    pub lang_id: u8,
    pub total_len: u16,
    pub source_hash: u32,
    pub revision: u32,
    pub entries: Vec<RuleManifestEntry>,
}

pub fn text_banks(source: &[u8]) -> Vec<(SliceKey, Vec<u8>)> {
    let mut banks = Vec::new();
    for bank in 0..RULES_TEXT_BANKS {
        let start = usize::from(bank) * RULES_BANK_BYTES;
        let end = (start + RULES_BANK_BYTES).min(source.len());
        let payload = if start >= source.len() {
            Vec::new()
        } else {
            source[start..end].to_vec()
        };
        banks.push((SliceKey::Rules { bank }, payload));
    }
    banks
}

#[must_use]
pub fn reassemble(manifest: &RulesManifest, banks: &[Option<Vec<u8>>]) -> Option<Vec<u8>> {
    let total = usize::from(manifest.total_len);
    if total > MAX_RULES_SOURCE_BYTES {
        return None;
    }
    let mut source = Vec::with_capacity(total);
    for bank in banks.iter().flatten() {
        source.extend_from_slice(bank);
    }
    if source.len() < total {
        return None;
    }
    source.truncate(total);
    if fnv1a32(&source) != manifest.source_hash {
        return None;
    }
    Some(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_for(source: &[u8]) -> RulesManifest {
        RulesManifest {
            version: RULES_MANIFEST_VERSION,
            lang_id: 1,
            total_len: u16::try_from(source.len()).expect("test source fits"),
            source_hash: fnv1a32(source),
            revision: 7,
            entries: Vec::new(),
        }
    }

    fn banks_after(previous: &[u8], source: &[u8]) -> Vec<Option<Vec<u8>>> {
        let mut held: Vec<Option<Vec<u8>>> = text_banks(previous)
            .into_iter()
            .map(|(_, bytes)| (!bytes.is_empty()).then_some(bytes))
            .collect();
        for (index, (_, bytes)) in text_banks(source).into_iter().enumerate() {
            held[index] = (!bytes.is_empty()).then_some(bytes);
        }
        held
    }

    #[test]
    fn a_document_that_shrinks_past_a_bank_boundary_comes_back_whole() {
        let long = vec![b'a'; RULES_BANK_BYTES + 100];
        let short = b"rule \"x\" {}\n".to_vec();

        let banks = banks_after(&long, &short);

        assert_eq!(reassemble(&manifest_for(&short), &banks), Some(short));
    }

    #[test]
    fn a_stale_higher_bank_left_by_an_older_writer_is_still_survivable() {
        let short = b"rule \"x\" {}\n".to_vec();
        let banks = vec![
            Some(short.clone()),
            Some(vec![b'z'; 1_129]),
            None,
        ];

        assert_eq!(reassemble(&manifest_for(&short), &banks), Some(short));
    }

    #[test]
    fn a_torn_commit_is_still_refused() {
        let mut new_doc = vec![b'n'; RULES_BANK_BYTES];
        new_doc.extend_from_slice(&[b'n'; 200]);
        let torn = vec![
            Some(vec![b'n'; RULES_BANK_BYTES]),
            Some(vec![b'o'; 200]),
            None,
        ];

        assert_eq!(reassemble(&manifest_for(&new_doc), &torn), None);
    }

    #[test]
    fn banks_that_fall_short_of_the_document_are_refused() {
        let doc = vec![b'a'; RULES_BANK_BYTES + 100];
        let missing_tail = vec![Some(vec![b'a'; RULES_BANK_BYTES]), None, None];

        assert_eq!(reassemble(&manifest_for(&doc), &missing_tail), None);
    }

    #[test]
    fn a_manifest_claiming_more_than_the_maximum_is_refused() {
        let mut manifest = manifest_for(b"x");
        manifest.total_len = u16::MAX;

        assert_eq!(reassemble(&manifest, &[Some(vec![b'x'; 64])]), None);
    }

    #[test]
    fn every_text_bank_is_named_even_by_a_one_line_document() {
        let banks = text_banks(b"rule \"x\" {}\n");

        assert_eq!(banks.len(), usize::from(RULES_TEXT_BANKS));
        assert!(!banks[0].1.is_empty());
        assert!(banks[1..].iter().all(|(_, bytes)| bytes.is_empty()),
                "the banks past the end must be named as empty, so the writer \
                 can clear what a longer document left there");
    }

    #[test]
    fn a_document_spanning_three_banks_splits_in_order() {
        let source: Vec<u8> = (0..RULES_BANK_BYTES * 2 + 7)
            .map(|i| u8::try_from(i % 251).expect("modulo fits"))
            .collect();

        let banks = text_banks(&source);
        let rejoined: Vec<u8> = banks.iter().flat_map(|(_, b)| b.clone()).collect();

        assert_eq!(banks[0].1.len(), RULES_BANK_BYTES);
        assert_eq!(banks[1].1.len(), RULES_BANK_BYTES);
        assert_eq!(banks[2].1.len(), 7);
        assert_eq!(rejoined, source);
    }
}
