use dali2rust_dali_phy::{backward_window::BACKWARD_DATA_BITS, PhyIsrCore};
use dali2rust_platform::arbitration::ArbitrationReflex;

#[cfg(target_os = "espidf")]
pub(crate) fn answer_published_frame(
    isr: &PhyIsrCore,
    reflex: &ArbitrationReflex,
    rx_epoch: u8,
    frame: [u8; 3],
    now_ms: u32,
) -> bool {
    let Some(answer) = reflex.answer_for(frame, now_ms) else {
        return false;
    };
    let buffer =
        dali2rust_dali_codec::codec::encode_to_half_bits(u32::from(answer), BACKWARD_DATA_BITS);
    if !isr.submit_answer(rx_epoch, &buffer) {
        reflex.note_cell_busy();
        return false;
    }
    true
}
