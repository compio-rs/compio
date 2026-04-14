use crate::{
    Key, OpCode,
    key::{ErasedKey, WeakKey},
};

/// A type-erased cancel token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cancel(WeakKey);

impl Cancel {
    /// Check if this cancel token cancels the given key.
    pub fn cancels<T: OpCode>(&self, key: &Key<T>) -> bool {
        self.0.as_ptr() as usize == key.as_raw()
    }

    pub(crate) fn new<T: OpCode>(key: &Key<T>) -> Self {
        Self(key.downgrade())
    }

    pub(crate) fn upgrade(&self) -> Option<ErasedKey> {
        self.0.upgrade()
    }
}
