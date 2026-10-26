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

const MAX_COPIED_VIEW_BYTES: usize = 2_048;

const COPY_BUILDING_BLOCKS: &[ViewTraits] = &[
    view_traits!(GearDiagnosticsView),
    view_traits!(SourceDiagnosticsView),
    view_traits!(LuminaireMaintenanceView),
    view_traits!(EnergyBankView),
    view_traits!(ConditionView),
];

#[test]
fn a_copy_view_is_small_enough_to_land_on_any_stack() {
    for view in SECTION_VIEWS.iter().chain(COPY_BUILDING_BLOCKS) {
        assert!(
            !view.is_copy || view.size <= MAX_COPIED_VIEW_BYTES,
            "{} is Copy at {} B: any reader can copy it onto its stack without a word. \
             Drop Copy and box it in place through BoxedInPlace instead",
            view.name,
            view.size
        );
    }
}

#[test]
fn a_section_view_leaves_the_registry_without_a_large_stack_copy() {
    for view in SECTION_VIEWS {
        assert!(
            view.in_place || view.size <= MAX_COPIED_VIEW_BYTES,
            "{} is {} B and neither Copy nor BoxedInPlace: the registry clones it on the httpd \
             stack, which costs up to twice that in one frame. Implement BoxedInPlace for it",
            view.name,
            view.size
        );
    }
}

#[test]
fn the_section_view_list_covers_every_variant() {
    assert_eq!(
        SECTION_VIEWS.len(),
        AttributeSectionKind::ALL.len(),
        "a section kind has no entry in the view list"
    );
}
