use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

trait LenientScalar: Sized {
    fn on_u64(v: u64) -> Self;
    fn on_i64(v: i64) -> Self;
    fn on_f64(v: f64) -> Self;
    fn on_bool(v: bool) -> Self;
    fn on_str(v: &str) -> Self;
    fn on_other() -> Self;
}

struct LenientScalarVisitor<T>(PhantomData<T>);

impl<'de, T: LenientScalar> Visitor<'de> for LenientScalarVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_u64<E>(self, v: u64) -> Result<T, E> {
        Ok(T::on_u64(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<T, E> {
        Ok(T::on_i64(v))
    }
    fn visit_u128<E>(self, _v: u128) -> Result<T, E> {
        Ok(T::on_other())
    }
    fn visit_i128<E>(self, _v: i128) -> Result<T, E> {
        Ok(T::on_other())
    }
    fn visit_f64<E>(self, v: f64) -> Result<T, E> {
        Ok(T::on_f64(v))
    }
    fn visit_bool<E>(self, v: bool) -> Result<T, E> {
        Ok(T::on_bool(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<T, E> {
        Ok(T::on_str(v))
    }
    fn visit_unit<E>(self) -> Result<T, E> {
        Ok(T::on_other())
    }
    fn visit_none<E>(self) -> Result<T, E> {
        Ok(T::on_other())
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        IgnoredAny::deserialize(MapAccessDeserializer::new(map))?;
        Ok(T::on_other())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<T, A::Error> {
        IgnoredAny::deserialize(SeqAccessDeserializer::new(seq))?;
        Ok(T::on_other())
    }
}

fn deserialize_scalar<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: LenientScalar,
{
    deserializer.deserialize_any(LenientScalarVisitor(PhantomData))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaybeU64 {
    Valid(u64),
    Invalid,
}

impl MaybeU64 {
    pub fn valid(&self) -> Option<u64> {
        match self {
            MaybeU64::Valid(v) => Some(*v),
            MaybeU64::Invalid => None,
        }
    }
}

impl LenientScalar for MaybeU64 {
    fn on_u64(v: u64) -> Self {
        MaybeU64::Valid(v)
    }
    fn on_i64(_v: i64) -> Self {
        MaybeU64::Invalid
    }
    fn on_f64(_v: f64) -> Self {
        MaybeU64::Invalid
    }
    fn on_bool(_v: bool) -> Self {
        MaybeU64::Invalid
    }
    fn on_str(_v: &str) -> Self {
        MaybeU64::Invalid
    }
    fn on_other() -> Self {
        MaybeU64::Invalid
    }
}

impl<'de> Deserialize<'de> for MaybeU64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_scalar(d)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaybeF64 {
    Valid(f64),
    Invalid,
}

impl MaybeF64 {
    pub fn valid(&self) -> Option<f64> {
        match self {
            MaybeF64::Valid(v) => Some(*v),
            MaybeF64::Invalid => None,
        }
    }
}

impl LenientScalar for MaybeF64 {
    fn on_u64(v: u64) -> Self {
        MaybeF64::Valid(v as f64)
    }
    fn on_i64(v: i64) -> Self {
        MaybeF64::Valid(v as f64)
    }
    fn on_f64(v: f64) -> Self {
        MaybeF64::Valid(v)
    }
    fn on_bool(_v: bool) -> Self {
        MaybeF64::Invalid
    }
    fn on_str(_v: &str) -> Self {
        MaybeF64::Invalid
    }
    fn on_other() -> Self {
        MaybeF64::Invalid
    }
}

impl<'de> Deserialize<'de> for MaybeF64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_scalar(d)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaybeString {
    Valid(String),
    Invalid,
}

impl MaybeString {
    pub fn valid(&self) -> Option<&str> {
        match self {
            MaybeString::Valid(v) => Some(v.as_str()),
            MaybeString::Invalid => None,
        }
    }
}

impl LenientScalar for MaybeString {
    fn on_u64(_v: u64) -> Self {
        MaybeString::Invalid
    }
    fn on_i64(_v: i64) -> Self {
        MaybeString::Invalid
    }
    fn on_f64(_v: f64) -> Self {
        MaybeString::Invalid
    }
    fn on_bool(_v: bool) -> Self {
        MaybeString::Invalid
    }
    fn on_str(v: &str) -> Self {
        MaybeString::Valid(v.to_string())
    }
    fn on_other() -> Self {
        MaybeString::Invalid
    }
}

impl<'de> Deserialize<'de> for MaybeString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_scalar(d)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaybeBool {
    Valid(bool),
    Invalid,
}

impl MaybeBool {
    pub fn valid(&self) -> Option<bool> {
        match self {
            MaybeBool::Valid(v) => Some(*v),
            MaybeBool::Invalid => None,
        }
    }
}

impl LenientScalar for MaybeBool {
    fn on_u64(_v: u64) -> Self {
        MaybeBool::Invalid
    }
    fn on_i64(_v: i64) -> Self {
        MaybeBool::Invalid
    }
    fn on_f64(_v: f64) -> Self {
        MaybeBool::Invalid
    }
    fn on_bool(v: bool) -> Self {
        MaybeBool::Valid(v)
    }
    fn on_str(_v: &str) -> Self {
        MaybeBool::Invalid
    }
    fn on_other() -> Self {
        MaybeBool::Invalid
    }
}

impl<'de> Deserialize<'de> for MaybeBool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_scalar(d)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaybeObj<T> {
    Valid(T),
    Invalid,
}

impl<T> MaybeObj<T> {
    pub fn valid(&self) -> Option<&T> {
        match self {
            MaybeObj::Valid(v) => Some(v),
            MaybeObj::Invalid => None,
        }
    }
}

struct MaybeObjVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for MaybeObjVisitor<T> {
    type Value = MaybeObj<T>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        Ok(MaybeObj::Valid(T::deserialize(MapAccessDeserializer::new(map))?))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        IgnoredAny::deserialize(SeqAccessDeserializer::new(seq))?;
        Ok(MaybeObj::Invalid)
    }
    fn visit_u64<E>(self, _v: u64) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_i64<E>(self, _v: i64) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_f64<E>(self, _v: f64) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_bool<E>(self, _v: bool) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(MaybeObj::Invalid)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for MaybeObj<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(MaybeObjVisitor(PhantomData))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaybeSeq<T> {
    Valid(Vec<T>),
    Invalid,
}

impl<T> MaybeSeq<T> {
    pub fn valid(&self) -> Option<&[T]> {
        match self {
            MaybeSeq::Valid(v) => Some(v.as_slice()),
            MaybeSeq::Invalid => None,
        }
    }
}

struct MaybeSeqVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for MaybeSeqVisitor<T> {
    type Value = MaybeSeq<T>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        Ok(MaybeSeq::Valid(Vec::deserialize(SeqAccessDeserializer::new(
            seq,
        ))?))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        IgnoredAny::deserialize(MapAccessDeserializer::new(map))?;
        Ok(MaybeSeq::Invalid)
    }
    fn visit_u64<E>(self, _v: u64) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_i64<E>(self, _v: i64) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_f64<E>(self, _v: f64) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_bool<E>(self, _v: bool) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(MaybeSeq::Invalid)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for MaybeSeq<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(MaybeSeqVisitor(PhantomData))
    }
}
