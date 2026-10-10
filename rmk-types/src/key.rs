//! Stable semantic identities for physical keys.

/// Stable identity of one logical key within a topology revision.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct KeyId(pub u16);
