use super::*;

const HTTP_VIEW_CEILINGS: &[(&str, usize, usize, &str)] = &[
    (
        "PhysicalDeviceSummaryView",
        std::mem::size_of::<PhysicalDeviceSummaryView>(),
        512,
        "the list builds one of these per device, back to back",
    ),
    (
        "PhysicalDeviceCoreView",
        std::mem::size_of::<PhysicalDeviceCoreView>(),
        512,
        "GET/PATCH/PUT on one device",
    ),
    (
        "AttributeSectionView",
        std::mem::size_of::<AttributeSectionView>(),
        32,
        "every variant is boxed, so a failure here means a variant lost its Box",
    ),
];

#[test]
fn http_reachable_views_stay_within_the_httpd_stack_budget() {
    for (name, actual, ceiling, why) in HTTP_VIEW_CEILINGS {
        assert!(
            actual <= ceiling,
            "{name} is {actual} B, ceiling {ceiling} B ({why}). \
             Box the new field rather than raising this: the view is built on the one httpd task's stack."
        );
    }
}

#[test]
fn the_registry_whole_device_snapshot_stays_bounded() {
    const CEILING: usize = 8_192;
    let actual = std::mem::size_of::<PhysicalDeviceView>();
    assert!(
        actual <= CEILING,
        "PhysicalDeviceView is {actual} B, ceiling {CEILING} B."
    );
}

#[test]
fn every_section_kind_round_trips_through_its_wire_name() {
    for kind in AttributeSectionKind::ALL.iter().copied() {
        assert_eq!(
            AttributeSectionKind::from_wire_name(kind.wire_name()),
            Some(kind),
            "{} does not round-trip",
            kind.wire_name()
        );
    }
    let mut names: Vec<&str> = AttributeSectionKind::ALL
        .iter()
        .map(|k| k.wire_name())
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), AttributeSectionKind::ALL.len());
}

#[test]
fn no_boxed_section_temporary_exceeds_the_budget() {
    const MAX_SECTION_TEMPORARY_BYTES: usize = 4_096;
    for (name, size) in BOXED_SECTION_TEMPORARIES {
        assert!(
            *size <= MAX_SECTION_TEMPORARY_BYTES,
            "{name} is {size} B: `Box::new(section.clone())` puts that on the \
             httpd stack under the read lock. Clone into the allocation \
             instead — the mirror of `default_in_place` — rather than raising \
             this number"
        );
    }
}

#[test]
fn the_section_temporary_list_covers_every_variant() {
    assert_eq!(
        BOXED_SECTION_TEMPORARIES.len(),
        AttributeSectionKind::ALL.len(),
        "a section kind has no entry in the temporary budget"
    );
}
