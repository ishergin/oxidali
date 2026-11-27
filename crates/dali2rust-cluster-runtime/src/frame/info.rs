use core::iter::Peekable;

use serde::{Deserialize, Serialize};

use super::codec::{put, write_head, DatagramBuffer, EncodeError, FrameKind, Rejection};
use super::command::{is_record_ref, RecordRef};
use super::header::Header;
use super::limits::{is_adapter, is_group, is_node_scene, MAX_NAME_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageMeta {
    pub last: bool,
    pub page: u8,
    pub set: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartFlags {
    pub send: bool,
    pub accept: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record<'a> {
    pub id: RecordRef,
    pub name: &'a str,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPart {
    pub adapter: u8,
    pub channel: RecordRef,
    pub group: u8,
    pub taken: bool,
    pub flags: PartFlags,
    pub revision: u32,
    pub desired_lamps: u64,
    pub written_lamps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePart {
    pub adapter: u8,
    pub scene: RecordRef,
    pub node_scene: u8,
    pub flags: PartFlags,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMap {
    pub adapter: u8,
    pub occupied: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalName<'a> {
    pub adapter: u8,
    pub number: u8,
    pub name: &'a str,
}

impl LocalName<'_> {
    const fn in_range(&self, number_in_range: bool) -> bool {
        number_in_range && is_adapter(self.adapter) && self.name.len() <= MAX_NAME_BYTES
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfoElement<'a> {
    ChannelRecord(#[serde(borrow)] Record<'a>),
    SceneRecord(#[serde(borrow)] Record<'a>),
    ChannelPart(ChannelPart),
    ScenePart(ScenePart),
    GroupMap(GroupMap),
    GroupName(#[serde(borrow)] LocalName<'a>),
    SceneName(#[serde(borrow)] LocalName<'a>),
}

impl InfoElement<'_> {
    pub(crate) const fn in_range(&self) -> bool {
        match self {
            Self::ChannelRecord(record) | Self::SceneRecord(record) => {
                is_record_ref(&record.id) && record.name.len() <= MAX_NAME_BYTES
            }
            Self::ChannelPart(part) => {
                is_adapter(part.adapter) && is_record_ref(&part.channel) && is_group(part.group)
            }
            Self::ScenePart(part) => {
                is_adapter(part.adapter) && is_record_ref(&part.scene) && is_node_scene(part.node_scene)
            }
            Self::GroupMap(map) => is_adapter(map.adapter),
            Self::GroupName(name) => name.in_range(is_group(name.number)),
            Self::SceneName(name) => name.in_range(is_node_scene(name.number)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoPage<'a> {
    meta: PageMeta,
    elements: &'a [u8],
}

impl<'a> InfoPage<'a> {
    pub const fn meta(&self) -> PageMeta {
        self.meta
    }

    pub const fn elements(&self) -> InfoElements<'a> {
        InfoElements { rest: self.elements }
    }

    pub(crate) fn decode(body: &'a [u8]) -> Result<Self, Rejection> {
        let (meta, elements) =
            postcard::take_from_bytes::<PageMeta>(body).map_err(|_| Rejection::Malformed)?;
        let mut rest = elements;
        while !rest.is_empty() {
            let (element, tail) = postcard::take_from_bytes::<InfoElement<'_>>(rest)
                .map_err(|_| Rejection::Malformed)?;
            if !element.in_range() {
                return Err(Rejection::Malformed);
            }
            rest = tail;
        }
        Ok(Self { meta, elements })
    }

    pub(crate) fn write(&self, out: &mut [u8]) -> Result<usize, EncodeError> {
        let at = put(&self.meta, out)?;
        let end = at + self.elements.len();
        out.get_mut(at..end)
            .ok_or(EncodeError::TooLarge)?
            .copy_from_slice(self.elements);
        Ok(end)
    }
}

pub struct InfoElements<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for InfoElements<'a> {
    type Item = InfoElement<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (element, rest) = postcard::take_from_bytes::<InfoElement<'a>>(self.rest).ok()?;
        self.rest = rest;
        Some(element)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    Full,
    OutOfRange,
}

pub struct InfoPageWriter<'b> {
    buffer: &'b mut DatagramBuffer,
    meta_at: usize,
    len: usize,
}

impl<'b> InfoPageWriter<'b> {
    pub fn start(
        buffer: &'b mut DatagramBuffer,
        header: &Header,
        set: u32,
        page: u8,
    ) -> Result<Self, EncodeError> {
        if !header.in_range() {
            return Err(EncodeError::OutOfRange);
        }
        let meta_at = write_head(buffer, FrameKind::ChannelSceneInfo, header)?;
        let meta = PageMeta { last: false, page, set };
        let len = meta_at + put(&meta, &mut buffer[meta_at..])?;
        Ok(Self { buffer, meta_at, len })
    }

    pub fn push(&mut self, element: &InfoElement<'_>) -> Result<(), PushError> {
        if !element.in_range() {
            return Err(PushError::OutOfRange);
        }
        let written = postcard::to_slice(element, &mut self.buffer[self.len..])
            .map_err(|_| PushError::Full)?
            .len();
        self.len += written;
        Ok(())
    }

    pub fn finish(self, last: bool) -> usize {
        self.buffer[self.meta_at] = u8::from(last);
        self.len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    set: u32,
    page: u8,
    taken: usize,
    finished: bool,
}

impl PageCursor {
    pub const fn new(set: u32) -> Self {
        Self { set, page: 0, taken: 0, finished: false }
    }

    pub const fn set(&self) -> u32 {
        self.set
    }

    pub const fn finished(&self) -> bool {
        self.finished
    }

    pub fn write_next<'e>(
        &mut self,
        buffer: &mut DatagramBuffer,
        header: &Header,
        elements: impl IntoIterator<Item = InfoElement<'e>>,
    ) -> Result<Option<usize>, EncodeError> {
        if self.finished {
            return Ok(None);
        }
        let mut rest = elements.into_iter().skip(self.taken).peekable();
        let written = write_page(buffer, header, self.set, self.page, &mut rest)?;
        self.taken += written.elements;
        self.finished = written.last;
        if !written.last {
            self.page = self.page.checked_add(1).ok_or(EncodeError::TooManyPages)?;
        }
        Ok(Some(written.len))
    }
}

struct WrittenPage {
    len: usize,
    elements: usize,
    last: bool,
}

fn write_page<'e, I: Iterator<Item = InfoElement<'e>>>(
    buffer: &mut DatagramBuffer,
    header: &Header,
    set: u32,
    page: u8,
    elements: &mut Peekable<I>,
) -> Result<WrittenPage, EncodeError> {
    let mut writer = InfoPageWriter::start(buffer, header, set, page)?;
    let mut pushed = 0;
    while let Some(element) = elements.peek() {
        match writer.push(element) {
            Ok(()) => pushed += 1,
            Err(PushError::Full) if pushed > 0 => break,
            Err(PushError::Full) => return Err(EncodeError::TooLarge),
            Err(PushError::OutOfRange) => return Err(EncodeError::OutOfRange),
        }
        elements.next();
    }
    let last = elements.peek().is_none();
    Ok(WrittenPage { len: writer.finish(last), elements: pushed, last })
}
