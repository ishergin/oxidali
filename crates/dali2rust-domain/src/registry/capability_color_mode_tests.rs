use super::{
    capability_accepts_color_mode, capability_supports_color_mode,
    seed_capability_from_color_mode, CapabilityFlagsView,
};
use dali2rust_contracts::msg::ColorMode;

fn caps(cct: bool, xy: bool, rgb: bool) -> CapabilityFlagsView {
    CapabilityFlagsView {
        cct,
        xy,
        rgb,
        ..CapabilityFlagsView::default()
    }
}

#[test]
fn supports_matches_the_specific_mode_only() {
    let c = caps(true, false, false);
    assert!(capability_supports_color_mode(c, ColorMode::Cct));
    assert!(!capability_supports_color_mode(c, ColorMode::Xy));
    assert!(!capability_supports_color_mode(c, ColorMode::Rgb));
}

#[test]
fn accepts_fails_open_when_no_colour_capability_is_known() {
    let unknown = CapabilityFlagsView::default();
    assert!(capability_accepts_color_mode(unknown, ColorMode::Rgb));
    assert!(capability_accepts_color_mode(unknown, ColorMode::Cct));
}

#[test]
fn accepts_filters_a_confirmed_mismatch() {
    let cct_only = caps(true, false, false);
    assert!(capability_accepts_color_mode(cct_only, ColorMode::Cct));
    assert!(!capability_accepts_color_mode(cct_only, ColorMode::Rgb));
}

#[test]
fn accepts_allows_either_mode_for_a_dual_capable_member() {
    let dual = caps(true, false, true);
    assert!(capability_accepts_color_mode(dual, ColorMode::Cct));
    assert!(capability_accepts_color_mode(dual, ColorMode::Rgb));
}

#[test]
fn a_six_channel_only_classification_still_counts_as_known() {
    let six_only = CapabilityFlagsView {
        rgbwaf: true,
        ..CapabilityFlagsView::default()
    };
    assert!(capability_accepts_color_mode(six_only, ColorMode::Rgbwaf));
    assert!(!capability_accepts_color_mode(six_only, ColorMode::Cct));
}

#[test]
fn seed_adds_the_declared_mode_without_clearing_evidence() {
    let mut c = caps(true, false, false);
    seed_capability_from_color_mode(&mut c, ColorMode::Rgb);
    assert!(c.rgb, "the declared mode asserts its own capability");
    assert!(c.cct, "scan evidence survives the assertion");
    assert!(!c.xy);
}

#[test]
fn seed_is_a_no_op_for_non_colour_modes() {
    let before = caps(true, false, false);
    for mode in [ColorMode::Brightness, ColorMode::None, ColorMode::Unknown] {
        let mut c = before;
        seed_capability_from_color_mode(&mut c, mode);
        assert_eq!(c, before, "{mode:?} must not seed a colour capability");
    }
}
