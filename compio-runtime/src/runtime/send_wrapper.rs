// Copyright 2017 Thomas Keh.
// Copyright 2024 compio-rs
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[cfg(feature = "current_thread_id")]
use std::thread::current_id;
use std::{
    mem::{self, ManuallyDrop},
    thread::{self, ThreadId},
};

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

/// A wrapper that copied from `send_wrapper` crate, with our own optimizations.
pub struct SendWrapper<T> {
    data: ManuallyDrop<T>,
    thread_id: ThreadId,
}

impl<T> SendWrapper<T> {
    /// Create a `SendWrapper<T>` wrapper around a value of type `T`.
    /// The wrapper takes ownership of the value.
    #[inline]
    pub fn new(data: T) -> SendWrapper<T> {
        SendWrapper {
            data: ManuallyDrop::new(data),
            thread_id: current_id(),
        }
    }

    /// Returns `true` if the value can be safely accessed from within the
    /// current thread.
    #[inline]
    pub fn valid(&self) -> bool {
        self.thread_id == current_id()
    }

    /// Returns a reference to the contained value.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    #[inline]
    pub unsafe fn get_unchecked(&self) -> &T {
        &self.data
    }

    /// Returns a reference to the contained value, if valid.
    #[inline]
    #[allow(dead_code)]
    pub fn get(&self) -> Option<&T> {
        if self.valid() { Some(&self.data) } else { None }
    }

    /// Returns a tracker that can be used to check if the current thread is
    /// the same as the creator thread.
    #[inline]
    pub fn tracker(&self) -> SendWrapper<()> {
        SendWrapper {
            data: ManuallyDrop::new(()),
            thread_id: self.thread_id,
        }
    }
}

unsafe impl<T> Send for SendWrapper<T> {}
unsafe impl<T> Sync for SendWrapper<T> {}

impl<T> Drop for SendWrapper<T> {
    /// Drops the contained value.
    ///
    /// # Panics
    ///
    /// Dropping panics if it is done from a different thread than the one the
    /// `SendWrapper<T>` instance has been created with.
    ///
    /// Exceptions:
    /// - There is no extra panic if the thread is already panicking/unwinding.
    ///   This is because otherwise there would be double panics (usually
    ///   resulting in an abort) when dereferencing from a wrong thread.
    /// - If `T` has a trivial drop ([`needs_drop::<T>()`] is false) then this
    ///   method never panics.
    ///
    /// [`needs_drop::<T>()`]: std::mem::needs_drop
    #[track_caller]
    fn drop(&mut self) {
        // If the drop is trivial (`needs_drop` = false), then dropping `T` can't access
        // it and so it can be safely dropped on any thread.
        if !mem::needs_drop::<T>() || self.valid() {
            unsafe {
                // Drop the inner value
                //
                // SAFETY:
                // - We've just checked that it's valid to drop `T` on this thread
                // - We only move out from `self.data` here and in drop, so `self.data` is
                //   present
                ManuallyDrop::drop(&mut self.data);
            }
        } else {
            invalid_drop()
        }
    }
}

#[cold]
#[inline(never)]
#[track_caller]
fn invalid_drop() {
    const DROP_ERROR: &str = "Dropped SendWrapper<T> variable from a thread different to the one \
                              it has been created with.";

    if !thread::panicking() {
        // panic because of dropping from wrong thread
        // only do this while not unwinding (could be caused by deref from wrong thread)
        panic!("{}", DROP_ERROR)
    }
}
