//! Bounded owned metadata parsing; no tile IO or Document/native admission.
use super::{FiniteRasterLimits, FiniteRasterM};
use crate::{FormatError, Result, manifest::TileRef};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::backing::{
    BackingError, NativePlaneDescriptor, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use serde::{
    Deserialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use std::{cell::Cell, fmt, io::Read};

/// Physical parsed metadata and its persistent ledger owner; borrowed input and
/// job callbacks are not retained. A native reader must still authenticate objects.
pub struct OwnedFiniteRasterMetadata {
    metadata: FiniteRasterM,
    descriptor: NativePlaneDescriptor,
    quota: BackingQuota,
    _index: QuotaLease,
    _owner: QuotaLease,
}
impl OwnedFiniteRasterMetadata {
    /// Input bytes remain caller-owned. Prepay bounded parser scratch and the
    /// maximum index/string payload before decoding. Strings and vectors use
    /// try_reserve_exact; no Vec<Value>, unbounded string, or ambient read route.
    /// Reservations are capacity accounting, not measured allocator/RSS claims.
    pub fn parse_with(bytes: &[u8], max_bytes: usize, limits: FiniteRasterLimits, quota: BackingQuota, control: &ReadControl<'_>) -> Result<Self> {
        super::poll(control, 1)?;
        if bytes.len() > max_bytes.min(1 << 20) {
            return Err(FormatError::LimitExceeded("finite metadata JSON bytes".into()));
        }
        let owner = quota.reserve(QuotaKind::Resident, (std::mem::size_of::<Self>() + 128) as u64).map_err(quota_error)?;
        let max_tiles = limits.max_tiles.min(4096);
        let envelope = max_tiles
            .checked_mul(2 * (std::mem::size_of::<TileRef>() + 64 + 128))
            .and_then(|n| n.checked_add(416))
            .ok_or_else(|| FormatError::LimitExceeded("finite metadata index overflow".into()))?;
        let mut index = quota.reserve(QuotaKind::Index, envelope as u64).map_err(quota_error)?;
        let temporary =
            bytes.len().checked_mul(2).and_then(|n| n.checked_add(8192)).ok_or_else(|| FormatError::LimitExceeded("finite parser scratch overflow".into()))?;
        let parser = quota.reserve(QuotaKind::Resident, temporary as u64).map_err(quota_error)?;
        let context = Context { control, failure: Cell::new(None) };
        let reader = ControlledInput { bytes, position: 0, context: &context };
        let mut deserializer = serde_json::Deserializer::from_reader(reader);
        let result = MetadataSeed { context: &context, max_tiles }.deserialize(&mut deserializer);
        let metadata = match result {
            Ok(value) => {
                if let Err(error) = deserializer.end() {
                    return Err(context.error(error));
                }
                value
            }
            Err(error) => return Err(context.error(error)),
        };
        let descriptor = metadata.descriptor_with(limits, control)?;
        let mut capacity = metadata
            .tiles
            .capacity()
            .checked_mul(std::mem::size_of::<TileRef>())
            .and_then(|n| n.checked_add(metadata.default.capacity()))
            .and_then(|n| n.checked_add(128))
            .ok_or_else(|| FormatError::LimitExceeded("finite retained metadata overflow".into()))?;
        for tile in &metadata.tiles {
            super::poll(control, 1)?;
            capacity = capacity
                .checked_add(tile.hash.capacity())
                .and_then(|n| n.checked_add(128))
                .ok_or_else(|| FormatError::LimitExceeded("finite retained hash overflow".into()))?;
        }
        index.reconcile_capacity(capacity as u64).map_err(quota_error)?;
        super::poll(control, 1)?;
        // Destroy decoder scratch before refunding its prepaid reservation.
        drop(deserializer);
        drop(parser);
        Ok(Self { metadata, descriptor, quota, _index: index, _owner: owner })
    }
    pub fn metadata(&self) -> &FiniteRasterM {
        &self.metadata
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    pub fn quota(&self) -> &BackingQuota {
        &self.quota
    }
}

fn quota_error(error: photocraft_raster::backing::quota::QuotaError) -> FormatError {
    FormatError::LimitExceeded(format!("finite metadata quota: {error}"))
}
struct Context<'c, 'a> {
    control: &'c ReadControl<'a>,
    failure: Cell<Option<BackingError>>,
}
impl Context<'_, '_> {
    fn poll<E: de::Error>(&self, n: u64) -> std::result::Result<(), E> {
        self.control.check(n).map_err(|error| {
            self.failure.set(Some(error));
            E::custom("finite metadata control refused")
        })
    }
    fn error(&self, error: serde_json::Error) -> FormatError {
        match self.failure.get() {
            Some(BackingError::Cancelled) => FormatError::Cancelled,
            Some(error) => FormatError::LimitExceeded(format!("finite metadata control: {error}")),
            None => FormatError::Json(error),
        }
    }
}
struct ControlledInput<'b, 'c, 'a> {
    bytes: &'b [u8],
    position: usize,
    context: &'c Context<'c, 'a>,
}
impl Read for ControlledInput<'_, '_, '_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let count = out.len().min(64).min(self.bytes.len().saturating_sub(self.position));
        if let Err(error) = self.context.control.check(count as u64) {
            self.context.failure.set(Some(error));
            return Err(std::io::Error::other("finite metadata control refused"));
        }
        let end = self.position.checked_add(count).ok_or_else(|| std::io::Error::other("finite input offset"))?;
        out.get_mut(..count)
            .ok_or_else(|| std::io::Error::other("finite input output"))?
            .copy_from_slice(self.bytes.get(self.position..end).ok_or_else(|| std::io::Error::other("finite input bounds"))?);
        self.position = end;
        Ok(count)
    }
}
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum Field {
    Format,
    Bounds,
    Default,
    Tiles,
}
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum TileField {
    Tx,
    Ty,
    Hash,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictFormat {
    mode: ColorMode,
    sample: SampleType,
    alpha: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictBounds {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}
struct MetadataSeed<'c, 'a> {
    context: &'c Context<'c, 'a>,
    max_tiles: usize,
}
impl<'de> DeserializeSeed<'de> for MetadataSeed<'_, '_> {
    type Value = FiniteRasterM;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for MetadataSeed<'_, '_> {
    type Value = FiniteRasterM;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("finite raster metadata object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<Self::Value, M::Error> {
        let (mut format, mut bounds, mut default, mut tiles) = (None, None, None, None);
        while let Some(field) = map.next_key::<Field>()? {
            self.context.poll(1)?;
            match field {
                Field::Format if format.is_none() => {
                    let f: StrictFormat = map.next_value()?;
                    format = Some(PixelFormat::new(f.mode, f.sample, f.alpha));
                }
                Field::Bounds if bounds.is_none() => {
                    let b: StrictBounds = map.next_value()?;
                    bounds = Some(Rect::new(b.x0, b.y0, b.x1, b.y1));
                }
                Field::Default if default.is_none() => default = Some(map.next_value_seed(HexSeed { context: self.context, maximum: 80, exact: None })?),
                Field::Tiles if tiles.is_none() => tiles = Some(map.next_value_seed(TilesSeed { context: self.context, maximum: self.max_tiles })?),
                _ => return Err(de::Error::custom("duplicate finite metadata field")),
            }
        }
        Ok(FiniteRasterM {
            format: format.ok_or_else(|| de::Error::missing_field("format"))?,
            bounds: bounds.ok_or_else(|| de::Error::missing_field("bounds"))?,
            default: default.ok_or_else(|| de::Error::missing_field("default"))?,
            tiles: tiles.ok_or_else(|| de::Error::missing_field("tiles"))?,
        })
    }
}
struct HexSeed<'c, 'a> {
    context: &'c Context<'c, 'a>,
    maximum: usize,
    exact: Option<usize>,
}
impl<'de> DeserializeSeed<'de> for HexSeed<'_, '_> {
    type Value = String;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<String, D::Error> {
        d.deserialize_str(self)
    }
}
impl Visitor<'_> for HexSeed<'_, '_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded canonical hexadecimal")
    }
    fn visit_str<E: de::Error>(self, text: &str) -> std::result::Result<String, E> {
        self.context.poll(1)?;
        if text.len() > self.maximum || self.exact.is_some_and(|n| text.len() != n) {
            return Err(E::custom("finite hexadecimal length"));
        }
        for byte in text.bytes() {
            self.context.poll(1)?;
            super::nibble(byte).map_err(|_| E::custom("finite hexadecimal character"))?;
        }
        let mut result = String::new();
        result.try_reserve_exact(text.len()).map_err(|_| E::custom("finite hexadecimal allocation"))?;
        if result.capacity() > self.maximum.checked_mul(2).and_then(|n| n.checked_add(128)).ok_or_else(|| E::custom("finite string envelope"))? {
            return Err(E::custom("finite string capacity"));
        }
        result.push_str(text);
        self.context.poll(text.len() as u64)?;
        Ok(result)
    }
}
struct TilesSeed<'c, 'a> {
    context: &'c Context<'c, 'a>,
    maximum: usize,
}
impl<'de> DeserializeSeed<'de> for TilesSeed<'_, '_> {
    type Value = Vec<TileRef>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for TilesSeed<'_, '_> {
    type Value = Vec<TileRef>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded finite tile references")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> std::result::Result<Self::Value, S::Error> {
        self.context.poll(1)?;
        let mut values = Vec::new();
        values.try_reserve_exact(self.maximum).map_err(|_| de::Error::custom("finite tile index allocation"))?;
        if values.capacity() > self.maximum.checked_mul(2).ok_or_else(|| de::Error::custom("finite index envelope"))? {
            return Err(de::Error::custom("finite tile index capacity"));
        }
        while values.len() < self.maximum {
            self.context.poll(1)?;
            match seq.next_element_seed(TileSeed { context: self.context })? {
                Some(value) => values.push(value),
                None => return Ok(values),
            }
        }
        if seq.next_element_seed(RejectExtra)?.is_some() {
            return Err(de::Error::custom("finite tile reference limit"));
        }
        Ok(values)
    }
}
struct RejectExtra;
impl<'de> DeserializeSeed<'de> for RejectExtra {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, _d: D) -> std::result::Result<(), D::Error> {
        Err(de::Error::custom("finite tile reference limit"))
    }
}
struct TileSeed<'c, 'a> {
    context: &'c Context<'c, 'a>,
}
impl<'de> DeserializeSeed<'de> for TileSeed<'_, '_> {
    type Value = TileRef;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for TileSeed<'_, '_> {
    type Value = TileRef;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("finite tile reference")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<Self::Value, M::Error> {
        let (mut tx, mut ty, mut hash) = (None, None, None);
        while let Some(field) = map.next_key::<TileField>()? {
            self.context.poll(1)?;
            match field {
                TileField::Tx if tx.is_none() => tx = Some(map.next_value()?),
                TileField::Ty if ty.is_none() => ty = Some(map.next_value()?),
                TileField::Hash if hash.is_none() => hash = Some(map.next_value_seed(HexSeed { context: self.context, maximum: 64, exact: Some(64) })?),
                _ => return Err(de::Error::custom("duplicate finite tile field")),
            }
        }
        Ok(TileRef {
            tx: tx.ok_or_else(|| de::Error::missing_field("tx"))?,
            ty: ty.ok_or_else(|| de::Error::missing_field("ty"))?,
            hash: hash.ok_or_else(|| de::Error::missing_field("hash"))?,
        })
    }
}
