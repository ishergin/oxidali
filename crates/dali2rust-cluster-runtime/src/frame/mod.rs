mod codec;
mod command;
mod header;
mod info;
mod inputs;
mod lamps;
mod limits;
mod note;
mod page;
mod presence;

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod lamp_tests;
#[cfg(test)]
mod tests;

pub use codec::{
    decode, encode, DatagramBuffer, Decoded, EncodeError, Frame, FrameKind, Rejection,
    PREAMBLE_BYTES, PROTOCOL_TAG, PROTOCOL_VERSION,
};
pub use command::{
    action_in_range, Action, BirthMark, Command, LampAddress, LightVerb, RecallScope, RecordRef, SourceClass,
    Target,
};
pub use header::{cluster_tag, Header};
pub use info::{
    ChannelPart, GroupMap, InfoElement, InfoElements, InfoKind, InfoPage, InfoPageWriter, LocalName, PartFlags,
    Record, ScenePart,
};
pub use inputs::{
    InputAddress, InputDevice, InputEvent, InputInfoElement, InputInfoElements, InputInfoKind, InputInfoPage,
    InputInfoWriter, InputInstance, InputOwner, InputValue, INPUT_FIELD_MASK, MAX_INPUT_PREFIX_BYTES, MAX_INPUT_SCHEME,
};
pub use lamps::{
    AdapterScope, HaIdentity, KelvinRange, LampCapabilities, LampColour, LampInfo, LampInfoElement,
    LampInfoElements, LampInfoKind, LampInfoPage, LampInfoWriter, LampState, LampStateKind, LampStatePage,
    LampStateWriter, PartState, StateElement, StateElements, MAX_DISCOVERY_PREFIX_BYTES,
};
pub use page::{InRange, Page, PageCursor, PageElements, PageKind, PageMeta, PagePlace, PageWriter, PushError};
pub use limits::{
    ADAPTER_COUNT, MAX_CLUSTER_NAME_BYTES, MAX_INSTALLATION_BYTES, MAX_NAME_BYTES, NODE_COUNT,
    NUMBERS_PER_NODE, RECORD_NUMBER_COUNT,
};
pub use note::{NoteDeed, NoteReach, WireNote};
pub use presence::{Hello, NodeClaim, Pulse};
