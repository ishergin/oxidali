use super::*;

impl<T: DaliTransport + Send> DaliController<T> {
    pub fn set_sniffer_tap(&mut self, tap: Arc<SnifferTap>, adapter_id: u8) {
        self.sniffer = Some(tap);
        self.sniffer_adapter_id = adapter_id;
    }

    pub(super) fn tap_exchange(&self, frame: WireFrame, outcome: TransferOutcome, attempt: u8) {
        let Some(tap) = self.sniffer.as_ref() else {
            return;
        };
        if !tap.is_enabled() {
            return;
        }
        let (kind, bytes) = match frame {
            WireFrame::Forward16(f) => {
                let raw = f.raw();
                (ObservedRawFrameKind::Forward16, [(raw >> 8) as u8, raw as u8, 0])
            }
            WireFrame::Forward24(bytes) => (ObservedRawFrameKind::Forward24, bytes),
        };
        tap.record_frame(
            FrameDirection::Tx,
            kind,
            bytes,
            self.sniffer_adapter_id,
            attempt,
        );
        if let TransferOutcome::Answer(byte) = outcome {
            tap.record_frame(
                FrameDirection::Reply,
                ObservedRawFrameKind::Backward8,
                [byte, 0, 0],
                self.sniffer_adapter_id,
                attempt,
            );
        }
    }
}
