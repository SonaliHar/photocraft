//! Document-scoped identities; source tattoos are provenance, not identifiers.
use crate::LayerId;
use serde::{Deserialize, Serialize};
macro_rules! ids {($($name:ident),*)=>{$(#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord,Hash,Serialize,Deserialize)] pub struct $name(pub u64);)*};}
ids!(ChannelId, MaskId, PathId, FilterId, ItemSetId, ResourceId, GraphNodeId);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemRef {
    Layer(LayerId),
    Channel(ChannelId),
    Mask(MaskId),
    Path(PathId),
}
impl ItemRef {
    pub fn raw(self) -> u64 {
        match self {
            Self::Layer(v) => v.0,
            Self::Channel(v) => v.0,
            Self::Mask(v) => v.0,
            Self::Path(v) => v.0,
        }
    }
}

impl ChannelId {
    /// Fresh canonical identity; independent of names, array positions and source tattoos.
    pub fn fresh() -> Self {
        Self(crate::next_id())
    }
}

impl PathId {
    /// Canonical identity independent of saved name, position and source tattoo.
    pub fn fresh() -> Self {
        Self(crate::next_id())
    }
}

/// Runtime target category; source names and tattoos never determine this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Layer,
    Channel,
    Mask,
    Path,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Zero,
    KindMismatch,
    Missing,
    DuplicateSource,
    TargetCollision,
    Limit,
    Allocation,
}
impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid document identity: {self:?}")
    }
}
impl std::error::Error for IdentityError {}

macro_rules! checked_ids {
    ($($name:ident),*) => {$ (
        impl $name {
            /// Checked canonical identity from external numeric input. Does not allocate
            /// or reserve an identity; uniqueness must be checked in the owning document.
            pub fn checked(raw: u64) -> Result<Self, IdentityError> {
                if raw == 0 { Err(IdentityError::Zero) } else { Ok(Self(raw)) }
            }
        }
    )*};
}
checked_ids!(ChannelId, MaskId, PathId, FilterId, ItemSetId, ResourceId, GraphNodeId);

impl ItemRef {
    pub fn kind(self) -> ItemKind {
        match self {
            Self::Layer(_) => ItemKind::Layer,
            Self::Channel(_) => ItemKind::Channel,
            Self::Mask(_) => ItemKind::Mask,
            Self::Path(_) => ItemKind::Path,
        }
    }

    /// Construct an explicitly typed nonzero target, without a name/index fallback.
    pub fn checked(kind: ItemKind, raw: u64) -> Result<Self, IdentityError> {
        if raw == 0 {
            return Err(IdentityError::Zero);
        }
        Ok(match kind {
            ItemKind::Layer => Self::Layer(LayerId(raw)),
            ItemKind::Channel => Self::Channel(ChannelId(raw)),
            ItemKind::Mask => Self::Mask(MaskId(raw)),
            ItemKind::Path => Self::Path(PathId(raw)),
        })
    }

    /// Validate a command's exact typed target against a bounded canonical inventory.
    /// Repeated identities are rejected, rather than resolved by first array position.
    pub fn validate_target(self, expected: ItemKind, inventory: &[ItemRef]) -> Result<(), IdentityError> {
        if self.raw() == 0 {
            return Err(IdentityError::Zero);
        }
        if self.kind() != expected {
            return Err(IdentityError::KindMismatch);
        }
        if inventory.len() > ItemRemap::MAX_ITEMS {
            return Err(IdentityError::Limit);
        }
        let mut found = false;
        for item in inventory {
            if *item == self {
                if found {
                    return Err(IdentityError::DuplicateSource);
                }
                found = true;
            }
        }
        if found { Ok(()) } else { Err(IdentityError::Missing) }
    }
}

/// Immutable, checked, document-local mapping. Construction completes before a caller
/// mutates any payload. Resolving an omitted identity fails instead of keeping a stale ID.
/// Other documents and untouched destination identities require their own inventory checks.
#[derive(Clone, Debug)]
pub struct ItemRemap {
    entries: Vec<(ItemRef, ItemRef)>,
}
impl ItemRemap {
    pub const MAX_ITEMS: usize = 10_000;

    pub fn new(entries: &[(ItemRef, ItemRef)]) -> Result<Self, IdentityError> {
        if entries.len() > Self::MAX_ITEMS {
            return Err(IdentityError::Limit);
        }
        let mut planned = Vec::new();
        planned.try_reserve_exact(entries.len()).map_err(|_| IdentityError::Allocation)?;
        for &(source, target) in entries {
            if source.raw() == 0 || target.raw() == 0 {
                return Err(IdentityError::Zero);
            }
            if source.kind() != target.kind() {
                return Err(IdentityError::KindMismatch);
            }
            planned.push((source, target));
        }
        planned.sort_unstable_by_key(|entry| entry.0);
        let mut previous = None;
        for &(source, _) in &planned {
            if previous == Some(source) {
                return Err(IdentityError::DuplicateSource);
            }
            previous = Some(source);
        }
        let mut targets = Vec::new();
        targets.try_reserve_exact(planned.len()).map_err(|_| IdentityError::Allocation)?;
        targets.extend(planned.iter().map(|entry| entry.1));
        targets.sort_unstable();
        let mut previous = None;
        for target in targets {
            if previous == Some(target) {
                return Err(IdentityError::TargetCollision);
            }
            previous = Some(target);
        }
        Ok(Self { entries: planned })
    }

    pub fn resolve(&self, source: ItemRef) -> Result<ItemRef, IdentityError> {
        if source.raw() == 0 {
            return Err(IdentityError::Zero);
        }
        let index = self.entries.binary_search_by_key(&source, |entry| entry.0).map_err(|_| IdentityError::Missing)?;
        self.entries.get(index).map(|entry| entry.1).ok_or(IdentityError::Missing)
    }

    /// Prepare a whole selector before publication, preserving order and duplicates.
    pub fn remap_selector(&self, source: &[ItemRef]) -> Result<Vec<ItemRef>, IdentityError> {
        if source.len() > Self::MAX_ITEMS {
            return Err(IdentityError::Limit);
        }
        let mut result = Vec::new();
        result.try_reserve_exact(source.len()).map_err(|_| IdentityError::Allocation)?;
        for &item in source {
            result.push(self.resolve(item)?);
        }
        Ok(result)
    }
}
