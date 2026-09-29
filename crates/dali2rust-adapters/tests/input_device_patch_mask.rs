use dali2rust_api::http::handlers::input_devices as api;

#[test]
fn the_two_copies_of_the_feedback_mask_agree() {
    use dali2rust_dali_runtime::dev103_feedback_mask as fb;
    assert_eq!(api::FB_PATCH_TIMING, fb::FB_PATCH_TIMING);
    assert_eq!(api::FB_PATCH_ACTIVE_BRIGHTNESS, fb::FB_PATCH_ACTIVE_BRIGHTNESS);
    assert_eq!(api::FB_PATCH_ACTIVE_COLOUR, fb::FB_PATCH_ACTIVE_COLOUR);
    assert_eq!(api::FB_PATCH_INACTIVE_BRIGHTNESS, fb::FB_PATCH_INACTIVE_BRIGHTNESS);
    assert_eq!(api::FB_PATCH_INACTIVE_COLOUR, fb::FB_PATCH_INACTIVE_COLOUR);
}
