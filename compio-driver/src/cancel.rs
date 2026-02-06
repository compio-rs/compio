use std::collections::HashSet;

use crate::{Key, OpCode, key::ErasedKey};

pub(crate) struct CancelRegistry {
    cancellers: HashSet<ErasedKey>,
}

/// A type-erased cancel token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cancel(usize);

impl Cancel {
    /// Check if this cancel token cancels the given key.
    pub fn cancels<T: OpCode>(&self, key: &Key<T>) -> bool {
        self.0 == key.as_raw()
    }
}

impl CancelRegistry {
    pub fn new() -> Self {
        Self {
            cancellers: HashSet::new(),
        }
    }

    pub fn register<T>(&mut self, key: &Key<T>) -> Cancel {
        let raw = key.as_raw();
        if self.cancellers.contains(&raw) {
            return Cancel(raw);
        }
        self.cancellers.insert(key.clone().erase());
        Cancel(raw)
    }

    pub fn take(&mut self, token: Cancel) -> Option<ErasedKey> {
        self.cancellers.take(&token.0)
    }

    pub fn remove(&mut self, key: &ErasedKey) -> bool {
        self.cancellers.remove(key)
    }
}
