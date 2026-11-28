use serde::{Deserialize, Serialize};

use super::codec::{DatagramBuffer, EncodeError, FrameKind};
use super::command::{is_record_ref, RecordRef};
use super::header::Header;
use super::limits::{is_adapter, is_group, is_node_scene, MAX_NAME_BYTES};
use super::page::{InRange, Page, PageCursor, PageElements, PageKind, PagePlace, PageWriter};

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

impl InRange for InfoElement<'_> {
    fn in_range(&self) -> bool {
        Self::in_range(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoKind;

impl PageKind for InfoKind {
    const KIND: FrameKind = FrameKind::ChannelSceneInfo;
    type Scope = ();
    type Element<'a> = InfoElement<'a>;
}

pub type InfoPage<'a> = Page<'a, InfoKind>;

pub type InfoElements<'a> = PageElements<'a, InfoKind>;

pub type InfoPageWriter<'b> = PageWriter<'b, InfoKind>;

impl<'b> PageWriter<'b, InfoKind> {
    pub fn start(buffer: &'b mut DatagramBuffer, header: &Header, set: u32, page: u8) -> Result<Self, EncodeError> {
        Self::open(buffer, header, PagePlace { scope: (), set, page })
    }
}

impl PageCursor {
    pub fn write_next<'e>(
        &mut self,
        buffer: &mut DatagramBuffer,
        header: &Header,
        elements: impl IntoIterator<Item = InfoElement<'e>>,
    ) -> Result<Option<usize>, EncodeError> {
        self.write_next_of::<InfoKind>(buffer, header, (), elements)
    }
}
