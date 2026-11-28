use core::iter::Peekable;
use core::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::codec::{put, write_head, DatagramBuffer, EncodeError, FrameKind, Rejection};
use super::header::Header;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageMeta {
    pub last: bool,
    pub page: u8,
    pub set: u32,
}

pub trait InRange {
    fn in_range(&self) -> bool;
}

impl InRange for () {
    fn in_range(&self) -> bool {
        true
    }
}

pub trait PageKind {
    const KIND: FrameKind;
    type Scope: Copy + Serialize + DeserializeOwned + InRange;
    type Element<'a>: Serialize + Deserialize<'a> + InRange;
}

pub struct Page<'a, K: PageKind> {
    meta: PageMeta,
    scope: K::Scope,
    elements: &'a [u8],
}

impl<K: PageKind> Clone for Page<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: PageKind> Copy for Page<'_, K> {}

impl<K: PageKind> PartialEq for Page<'_, K>
where
    K::Scope: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.meta == other.meta && self.scope == other.scope && self.elements == other.elements
    }
}

impl<K: PageKind> Eq for Page<'_, K> where K::Scope: Eq {}

impl<K: PageKind> core::fmt::Debug for Page<'_, K>
where
    K::Scope: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Page").field("meta", &self.meta).field("scope", &self.scope).finish_non_exhaustive()
    }
}

impl<'a, K: PageKind> Page<'a, K> {
    pub const fn meta(&self) -> PageMeta {
        self.meta
    }

    pub const fn scope(&self) -> K::Scope {
        self.scope
    }

    pub const fn elements(&self) -> PageElements<'a, K> {
        PageElements { rest: self.elements, kind: PhantomData }
    }

    pub(crate) fn decode(body: &'a [u8]) -> Result<Self, Rejection> {
        let (meta, rest) = postcard::take_from_bytes::<PageMeta>(body).map_err(|_| Rejection::Malformed)?;
        let (scope, elements) = postcard::take_from_bytes::<K::Scope>(rest).map_err(|_| Rejection::Malformed)?;
        if !scope.in_range() {
            return Err(Rejection::Malformed);
        }
        let mut rest = elements;
        while !rest.is_empty() {
            let (element, tail) =
                postcard::take_from_bytes::<K::Element<'_>>(rest).map_err(|_| Rejection::Malformed)?;
            if !element.in_range() {
                return Err(Rejection::Malformed);
            }
            rest = tail;
        }
        Ok(Self { meta, scope, elements })
    }

    pub(crate) fn write(&self, out: &mut [u8]) -> Result<usize, EncodeError> {
        if !self.scope.in_range() {
            return Err(EncodeError::OutOfRange);
        }
        let at = put(&self.meta, out)?;
        let at = at + put(&self.scope, out.get_mut(at..).ok_or(EncodeError::TooLarge)?)?;
        let end = at + self.elements.len();
        out.get_mut(at..end).ok_or(EncodeError::TooLarge)?.copy_from_slice(self.elements);
        Ok(end)
    }
}

pub struct PageElements<'a, K: PageKind> {
    rest: &'a [u8],
    kind: PhantomData<K>,
}

impl<'a, K: PageKind> Iterator for PageElements<'a, K> {
    type Item = K::Element<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (element, rest) = postcard::take_from_bytes::<K::Element<'a>>(self.rest).ok()?;
        self.rest = rest;
        Some(element)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    Full,
    OutOfRange,
}

pub struct PageWriter<'b, K: PageKind> {
    buffer: &'b mut DatagramBuffer,
    meta_at: usize,
    len: usize,
    kind: PhantomData<K>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PagePlace<S> {
    pub scope: S,
    pub set: u32,
    pub page: u8,
}

impl<'b, K: PageKind> PageWriter<'b, K> {
    pub fn open(buffer: &'b mut DatagramBuffer, header: &Header, place: PagePlace<K::Scope>) -> Result<Self, EncodeError> {
        if !header.in_range() || !place.scope.in_range() {
            return Err(EncodeError::OutOfRange);
        }
        let meta_at = write_head(buffer, K::KIND, header)?;
        let meta = PageMeta { last: false, page: place.page, set: place.set };
        let at = meta_at + put(&meta, &mut buffer[meta_at..])?;
        let len = at + put(&place.scope, &mut buffer[at..])?;
        Ok(Self { buffer, meta_at, len, kind: PhantomData })
    }

    pub fn push(&mut self, element: &K::Element<'_>) -> Result<(), PushError> {
        if !element.in_range() {
            return Err(PushError::OutOfRange);
        }
        let written = postcard::to_slice(element, &mut self.buffer[self.len..]).map_err(|_| PushError::Full)?.len();
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

    pub fn write_next_of<'e, K: PageKind>(
        &mut self,
        buffer: &mut DatagramBuffer,
        header: &Header,
        scope: K::Scope,
        elements: impl IntoIterator<Item = K::Element<'e>>,
    ) -> Result<Option<usize>, EncodeError> {
        if self.finished {
            return Ok(None);
        }
        let mut rest = elements.into_iter().skip(self.taken).peekable();
        let place = PagePlace { scope, set: self.set, page: self.page };
        let written = write_page::<K, _>(buffer, header, place, &mut rest)?;
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

fn write_page<'e, K: PageKind, I: Iterator<Item = K::Element<'e>>>(
    buffer: &mut DatagramBuffer,
    header: &Header,
    place: PagePlace<K::Scope>,
    elements: &mut Peekable<I>,
) -> Result<WrittenPage, EncodeError> {
    let mut writer = PageWriter::<K>::open(buffer, header, place)?;
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
