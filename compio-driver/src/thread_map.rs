// Copyright (c) 2024-2025 Paulo Villela
// Copyright (c) 2026 compio-rs
//
// MIT License

#[cfg(feature = "current_thread_id")]
use std::thread::current_id;
use std::{cell::UnsafeCell, collections::HashMap, fmt::Debug, thread::ThreadId};

// FIXME: the code is the same as the one in `compio-runtime`.
#[cfg(not(feature = "current_thread_id"))]
mod imp {
    use std::{
        cell::Cell,
        thread::{self, ThreadId},
    };
    thread_local! {
        static THREAD_ID: Cell<ThreadId> = Cell::new(thread::current().id());
    }

    pub fn current_id() -> ThreadId {
        THREAD_ID.get()
    }
}

#[cfg(not(feature = "current_thread_id"))]
use imp::current_id;

/// Wrapper to enable cell to be used as value in `HashMap`.
struct UnsafeSyncCell<V>(UnsafeCell<V>);

/// SAFETY:
/// An instance is only accessed by [`ThreadMap`] through mutable references.
unsafe impl<V> Sync for UnsafeSyncCell<V> {}

impl<V: Debug> Debug for UnsafeSyncCell<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", unsafe { &*self.0.get() }))
    }
}

#[derive(Debug)]
pub struct ThreadMap<V> {
    state: HashMap<ThreadId, UnsafeSyncCell<V>>,
    value_init: fn() -> V,
}

impl<V> ThreadMap<V> {
    pub fn new(value_init: fn() -> V) -> Self {
        Self {
            state: HashMap::new(),
            value_init,
        }
    }

    pub fn get_mut(&mut self) -> &mut V {
        let tid = current_id();
        self.state
            .entry(tid)
            .or_insert_with(|| UnsafeSyncCell(UnsafeCell::new((self.value_init)())))
            .0
            .get_mut()
    }
}

impl<V: Default> Default for ThreadMap<V> {
    fn default() -> Self {
        Self::new(V::default)
    }
}
