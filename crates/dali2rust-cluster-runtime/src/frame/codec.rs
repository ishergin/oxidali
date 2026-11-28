use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use serde::{Deserialize, Serialize};

use super::command::Command;
use super::header::Header;
use super::info::InfoPage;
use super::note::WireNote;
use super::presence::{Hello, Pulse};

pub const PROTOCOL_TAG: [u8; 4] = *b"D2RC";

pub const PROTOCOL_VERSION: u8 = 1;

pub const PREAMBLE_BYTES: usize = PROTOCOL_TAG.len() + 2;

const VERSION_AT: usize = PROTOCOL_TAG.len();

const KIND_AT: usize = VERSION_AT + 1;

pub type DatagramBuffer = [u8; MAX_CLUSTER_DATAGRAM_BYTES];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    Command = 0,
    Pulse = 1,
    Hello = 2,
    ChannelSceneInfo = 3,
    WireNote = 4,
}

impl FrameKind {
    const ALL: [Self; 5] = [Self::Command, Self::Pulse, Self::Hello, Self::ChannelSceneInfo, Self::WireNote];

    fn from_byte(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| *kind as u8 == byte)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame<'a> {
    Command(Command),
    Pulse(Pulse<'a>),
    Hello(Hello),
    ChannelSceneInfo(InfoPage<'a>),
    WireNote(WireNote),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded<'a> {
    pub header: Header,
    pub frame: Frame<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    NotCluster,
    UnknownVersion(u8),
    UnknownKind(u8),
    TooLarge,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    TooLarge,
    OutOfRange,
    TooManyPages,
}

impl<'a> Frame<'a> {
    const fn kind(&self) -> FrameKind {
        match self {
            Self::Command(_) => FrameKind::Command,
            Self::Pulse(_) => FrameKind::Pulse,
            Self::Hello(_) => FrameKind::Hello,
            Self::ChannelSceneInfo(_) => FrameKind::ChannelSceneInfo,
            Self::WireNote(_) => FrameKind::WireNote,
        }
    }

    fn in_range(&self) -> bool {
        match self {
            Self::Command(command) => command.in_range(),
            Self::WireNote(note) => note.in_range(),
            Self::Pulse(pulse) => pulse.in_range(),
            Self::Hello(_) | Self::ChannelSceneInfo(_) => true,
        }
    }

    fn write_body(&self, out: &mut [u8]) -> Result<usize, EncodeError> {
        match self {
            Self::Command(command) => put(command, out),
            Self::Pulse(pulse) => put(pulse, out),
            Self::Hello(hello) => put(hello, out),
            Self::ChannelSceneInfo(page) => page.write(out),
            Self::WireNote(note) => put(note, out),
        }
    }

    fn read_body(kind: FrameKind, body: &'a [u8]) -> Result<Self, Rejection> {
        let frame = match kind {
            FrameKind::Command => Self::Command(whole(body)?),
            FrameKind::Pulse => Self::Pulse(whole(body)?),
            FrameKind::Hello => Self::Hello(whole(body)?),
            FrameKind::ChannelSceneInfo => Self::ChannelSceneInfo(InfoPage::decode(body)?),
            FrameKind::WireNote => Self::WireNote(whole(body)?),
        };
        if frame.in_range() {
            Ok(frame)
        } else {
            Err(Rejection::Malformed)
        }
    }
}

pub fn encode(
    header: &Header,
    frame: &Frame<'_>,
    buffer: &mut DatagramBuffer,
) -> Result<usize, EncodeError> {
    if !header.in_range() || !frame.in_range() {
        return Err(EncodeError::OutOfRange);
    }
    let at = write_head(buffer, frame.kind(), header)?;
    Ok(at + frame.write_body(&mut buffer[at..])?)
}

pub fn decode(datagram: &[u8]) -> Result<Decoded<'_>, Rejection> {
    if datagram.len() > MAX_CLUSTER_DATAGRAM_BYTES {
        return Err(Rejection::TooLarge);
    }
    let (kind, rest) = read_preamble(datagram)?;
    let (header, body) =
        postcard::take_from_bytes::<Header>(rest).map_err(|_| Rejection::Malformed)?;
    if !header.in_range() {
        return Err(Rejection::Malformed);
    }
    Ok(Decoded { header, frame: Frame::read_body(kind, body)? })
}

fn read_preamble(datagram: &[u8]) -> Result<(FrameKind, &[u8]), Rejection> {
    let Some((preamble, rest)) = datagram.split_first_chunk::<PREAMBLE_BYTES>() else {
        return Err(Rejection::NotCluster);
    };
    if !preamble.starts_with(&PROTOCOL_TAG) {
        return Err(Rejection::NotCluster);
    }
    let version = preamble[VERSION_AT];
    if version != PROTOCOL_VERSION {
        return Err(Rejection::UnknownVersion(version));
    }
    let kind_byte = preamble[KIND_AT];
    let kind = FrameKind::from_byte(kind_byte).ok_or(Rejection::UnknownKind(kind_byte))?;
    Ok((kind, rest))
}

pub(crate) fn write_head(
    buffer: &mut DatagramBuffer,
    kind: FrameKind,
    header: &Header,
) -> Result<usize, EncodeError> {
    buffer[..VERSION_AT].copy_from_slice(&PROTOCOL_TAG);
    buffer[VERSION_AT] = PROTOCOL_VERSION;
    buffer[KIND_AT] = kind as u8;
    Ok(PREAMBLE_BYTES + put(header, &mut buffer[PREAMBLE_BYTES..])?)
}

pub(crate) fn put<T: Serialize + ?Sized>(value: &T, out: &mut [u8]) -> Result<usize, EncodeError> {
    postcard::to_slice(value, out)
        .map(|written| written.len())
        .map_err(|_| EncodeError::TooLarge)
}

fn whole<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, Rejection> {
    match postcard::take_from_bytes::<T>(body) {
        Ok((value, [])) => Ok(value),
        _ => Err(Rejection::Malformed),
    }
}
