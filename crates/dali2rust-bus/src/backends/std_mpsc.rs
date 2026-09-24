use std::sync::mpsc::{self, TrySendError};

use crate::channels::{Receiver, RecvError, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

#[derive(Debug)]
pub struct MpscSender<T>(mpsc::SyncSender<T>);

impl<T> Clone for MpscSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> MpscSender<T> {
    pub fn new(inner: mpsc::SyncSender<T>) -> Self {
        Self(inner)
    }
}

impl<T: Send + 'static> Sender<T> for MpscSender<T> {
    fn try_send(&self, item: T) -> Result<(), T> {
        self.0.try_send(item).map_err(|e| match e {
            TrySendError::Full(item) | TrySendError::Disconnected(item) => item,
        })
    }
}

#[derive(Debug)]
pub struct MpscReceiver<T>(mpsc::Receiver<T>);

impl<T> MpscReceiver<T> {
    pub fn new(inner: mpsc::Receiver<T>) -> Self {
        Self(inner)
    }
}

impl<T: Send + 'static> Receiver<T> for MpscReceiver<T> {
    fn recv(&self) -> Result<T, RecvError> {
        self.0.recv().map_err(|_| RecvError::Disconnected)
    }

    fn recv_timeout(&self, timeout_ms: u32) -> Result<T, RecvTimeoutError> {
        match self
            .0
            .recv_timeout(Duration::from_millis(timeout_ms as u64))
        {
            Ok(item) => Ok(item),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(RecvTimeoutError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        }
    }

    fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.try_recv() {
            Ok(item) => Ok(item),
            Err(mpsc::TryRecvError::Empty) => Err(TryRecvError::Empty),
            Err(mpsc::TryRecvError::Disconnected) => Err(TryRecvError::Disconnected),
        }
    }
}

impl<T: Send + 'static> Receiver<T> for mpsc::Receiver<T> {
    fn recv(&self) -> Result<T, RecvError> {
        mpsc::Receiver::recv(self).map_err(|_| RecvError::Disconnected)
    }

    fn recv_timeout(&self, timeout_ms: u32) -> Result<T, RecvTimeoutError> {
        match mpsc::Receiver::recv_timeout(self, Duration::from_millis(timeout_ms as u64)) {
            Ok(item) => Ok(item),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(RecvTimeoutError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        }
    }

    fn try_recv(&self) -> Result<T, TryRecvError> {
        match mpsc::Receiver::try_recv(self) {
            Ok(item) => Ok(item),
            Err(mpsc::TryRecvError::Empty) => Err(TryRecvError::Empty),
            Err(mpsc::TryRecvError::Disconnected) => Err(TryRecvError::Disconnected),
        }
    }
}

pub fn create_channel<T>(capacity: usize) -> (MpscSender<T>, MpscReceiver<T>) {
    let (tx, rx) = mpsc::sync_channel(capacity);
    (MpscSender::new(tx), MpscReceiver::new(rx))
}
