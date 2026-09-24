#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishResult {
    Queued,
    DroppedIngressFull,
    RejectedFrameTooLarge,
    RejectedKindMismatch,
}
