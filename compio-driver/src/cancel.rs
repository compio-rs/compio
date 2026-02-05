use std::collections::HashSet;

use crate::{Key, key::ErasedKey};

pub(crate) struct CancelRegistry {
    cancellers: HashSet<ErasedKey>,
}

/// A type-erased cancel token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cancel(usize);

impl CancelRegistry {
    pub fn new() -> Self {
        Self {
            cancellers: HashSet::new(),
        }
    }

    pub fn register<T>(&mut self, key: Key<T>) -> Cancel {
        let key_num = key.as_raw();
        self.cancellers.insert(key.erase());
        Cancel(key_num)
    }

    pub fn take(&mut self, token: Cancel) -> Option<ErasedKey> {
        self.cancellers.take(&token.0)
    }

    pub fn remove(&mut self, key: &ErasedKey) -> bool {
        self.cancellers.remove(key)
    }
}
