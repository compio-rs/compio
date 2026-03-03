use std::{collections::HashSet, mem::ManuallyDrop};

use crate::{Key, OpCode, key::ErasedKey, thread_map::ThreadMap};

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

struct CancelRegistryInner {
    cancellers: ManuallyDrop<HashSet<ErasedKey>>,
}

impl CancelRegistryInner {
    pub fn new() -> Self {
        Self {
            cancellers: ManuallyDrop::new(HashSet::new()),
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

    pub fn clear(&mut self) {
        self.cancellers.clear();
    }
}

impl Drop for CancelRegistryInner {
    fn drop(&mut self) {
        if self.cancellers.is_empty() {
            // SAFETY: No keys remain.
            unsafe { ManuallyDrop::drop(&mut self.cancellers) }
        }
    }
}

pub(crate) struct CancelRegistry {
    inner: ThreadMap<CancelRegistryInner>,
}

// SAFETY: `CancelRegistryInner` is only accessed and dropped on the thread it
// was created on.
unsafe impl Send for CancelRegistry {}

impl CancelRegistry {
    pub fn new() -> Self {
        Self {
            inner: ThreadMap::new(CancelRegistryInner::new),
        }
    }

    pub fn register<T>(&mut self, key: &Key<T>) -> Cancel {
        self.inner.get_mut().register(key)
    }

    pub fn take(&mut self, token: Cancel) -> Option<ErasedKey> {
        self.inner.get_mut().take(token)
    }

    pub fn remove(&mut self, key: &ErasedKey) -> bool {
        self.inner.get_mut().remove(key)
    }

    pub fn clear(&mut self) {
        self.inner.get_mut().clear();
    }
}

impl Drop for CancelRegistry {
    fn drop(&mut self) {
        self.clear();
    }
}
