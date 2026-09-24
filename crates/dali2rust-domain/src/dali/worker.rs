use crate::dali::frame::{BackwardFrame, ForwardFrame, FrameError};

#[derive(Debug, Clone)]
pub struct DaliRequest {
    pub token: u64,
    pub forward_frame: ForwardFrame,
    pub expects_backward: bool,
}

#[derive(Debug, Clone)]
pub struct DaliResult {
    pub token: u64,
    pub backward_frame: Option<BackwardFrame>,
    pub error: Option<FrameError>,
}

pub trait DaliWorkerPort {
    fn execute(&mut self, request: DaliRequest) -> DaliResult;
}

pub trait DaliWorkerPortExt: DaliWorkerPort {
    fn send(&mut self, token: u64, frame: ForwardFrame) -> DaliResult {
        self.execute(DaliRequest {
            token,
            forward_frame: frame,
            expects_backward: false,
        })
    }

    fn query(&mut self, token: u64, frame: ForwardFrame) -> DaliResult {
        self.execute(DaliRequest {
            token,
            forward_frame: frame,
            expects_backward: true,
        })
    }
}

impl<T: DaliWorkerPort> DaliWorkerPortExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubWorker;

    impl DaliWorkerPort for StubWorker {
        fn execute(&mut self, request: DaliRequest) -> DaliResult {
            DaliResult {
                token: request.token,
                backward_frame: request.expects_backward.then(|| BackwardFrame::new(0xAA)),
                error: None,
            }
        }
    }

    #[test]
    fn stub_send_produces_no_backward() {
        let mut w = StubWorker;
        let result = w.send(42, ForwardFrame::new(0x02, 0xFE));
        assert_eq!(result.token, 42);
        assert!(result.backward_frame.is_none());
        assert!(result.error.is_none());
    }

    #[test]
    fn stub_query_produces_backward() {
        let mut w = StubWorker;
        let result = w.query(99, ForwardFrame::new(0x03, 0x90));
        assert_eq!(result.token, 99);
        assert_eq!(result.backward_frame.unwrap().raw(), 0xAA);
    }
}
