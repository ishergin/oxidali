use dali2rust_api::http::handlers::input_devices as api;
use dali2rust_dali_runtime::dev103_patch_mask as worker;

#[test]
fn the_two_copies_of_the_patch_mask_agree() {
    assert_eq!(api::PATCH_EVENT_SCHEME, worker::PATCH_EVENT_SCHEME);
    assert_eq!(api::PATCH_EVENT_FILTER, worker::PATCH_EVENT_FILTER);
    assert_eq!(api::PATCH_EVENT_PRIORITY, worker::PATCH_EVENT_PRIORITY);
    assert_eq!(api::PATCH_INSTANCE_GROUP_0, worker::PATCH_INSTANCE_GROUP_0);
    assert_eq!(api::PATCH_INSTANCE_GROUP_0 << 1, worker::PATCH_INSTANCE_GROUP_1);
    assert_eq!(api::PATCH_INSTANCE_GROUP_0 << 2, worker::PATCH_INSTANCE_GROUP_2);
    assert_eq!(api::TIMER_PATCH_BITS, worker::TIMER_PATCH_BITS);
    assert_eq!(api::PATCH_INSTANCE_ENABLED, worker::PATCH_INSTANCE_ENABLED);
}

const PATCH_BIT_COUNT: u32 = 11;

#[test]
fn every_bit_of_the_patch_mask_is_held_by_this_file() {
    let held = [
        api::PATCH_EVENT_SCHEME,
        api::PATCH_EVENT_FILTER,
        api::PATCH_EVENT_PRIORITY,
        api::PATCH_INSTANCE_GROUP_0,
        api::PATCH_INSTANCE_GROUP_0 << 1,
        api::PATCH_INSTANCE_GROUP_0 << 2,
        api::PATCH_INSTANCE_ENABLED,
    ]
    .into_iter()
    .chain(api::TIMER_PATCH_BITS)
    .fold(0u16, |acc, bit| acc | bit);
    assert_eq!(held, worker::ALL_PATCH_BITS, "a patch bit has no row above");
    assert_eq!(
        worker::ALL_PATCH_BITS.count_ones(),
        PATCH_BIT_COUNT,
        "the mask grew: add the new bit to `held` above and raise the frozen \
         count, so the two lists cannot both be satisfied by forgetting"
    );
    assert_eq!(
        worker::ALL_PATCH_BITS,
        (1u16 << PATCH_BIT_COUNT) - 1,
        "patch bits are allocated contiguously from bit 0; a gap means a \
         declared bit never reached `ALL_PATCH_BITS`"
    );
}

#[test]
fn the_two_copies_of_the_feedback_mask_agree() {
    use dali2rust_dali_runtime::dev103_feedback_mask as fb;
    assert_eq!(api::FB_PATCH_TIMING, fb::FB_PATCH_TIMING);
    assert_eq!(api::FB_PATCH_ACTIVE_BRIGHTNESS, fb::FB_PATCH_ACTIVE_BRIGHTNESS);
    assert_eq!(api::FB_PATCH_ACTIVE_COLOUR, fb::FB_PATCH_ACTIVE_COLOUR);
    assert_eq!(api::FB_PATCH_INACTIVE_BRIGHTNESS, fb::FB_PATCH_INACTIVE_BRIGHTNESS);
    assert_eq!(api::FB_PATCH_INACTIVE_COLOUR, fb::FB_PATCH_INACTIVE_COLOUR);
}
