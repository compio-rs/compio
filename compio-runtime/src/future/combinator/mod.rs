//! Future combinators.

mod cancel;
mod notify_always;
mod personality;

use std::{
    borrow::Cow,
    sync::atomic::{AtomicI8, Ordering},
};

pub use cancel::*;
use compio_driver::Extra;
pub use notify_always::*;
pub use personality::*;

use crate::CancelToken;

#[non_exhaustive]
#[derive(Debug, Default)]
pub(crate) struct Ext<'a> {
    personality: Option<u16>,
    cancel: Option<Cow<'a, CancelToken>>,
    notify_always: AtomicI8,
}

const NOTIFY_UNSET: i8 = 0;
const NOTIFY_FALSE: i8 = -1;
const NOTIFY_TRUE: i8 = 1;

impl<'a> Ext<'a> {
    pub fn to_owned(&self) -> Ext<'static> {
        Ext {
            personality: self.personality,
            cancel: self
                .cancel
                .as_ref()
                .map(|x| Cow::Owned(x.clone().into_owned())),
            notify_always: AtomicI8::new(self.notify_always.load(Ordering::Acquire)),
        }
    }
}

impl<'a> Ext<'a> {
    pub fn with_personality(&self, personality: u16) -> Self {
        Self {
            personality: Some(personality),
            cancel: self.cancel.clone(),
            notify_always: AtomicI8::new(self.notify_always.load(Ordering::Acquire)),
        }
    }

    pub fn with_cancel(&self, token: &'a CancelToken) -> Self {
        Self {
            personality: self.personality,
            cancel: Some(Cow::Borrowed(token)),
            notify_always: AtomicI8::new(self.notify_always.load(Ordering::Acquire)),
        }
    }

    pub fn get_cancel(&self) -> Option<&CancelToken> {
        self.cancel.as_deref()
    }

    pub fn set_extra(&self, extra: &mut Extra) -> bool {
        let mut changed = false;
        if let Some(personality) = self.personality {
            extra.set_personality(personality);
            changed = true;
        }
        changed
    }

    pub fn set_notify_always(&self, notify: bool) {
        self.notify_always
            .compare_exchange(
                NOTIFY_UNSET,
                if notify { NOTIFY_TRUE } else { NOTIFY_FALSE },
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok();
    }

    pub fn should_notify_always(&self) -> bool {
        self.notify_always.load(Ordering::Acquire) == NOTIFY_TRUE
    }
}

/// Extension trait for futures.
///
/// # Implementation
///
/// Extra data are passed down to runtime when the combinators are polled using
/// a custom [`Waker`], and those data are single-threaded. This means
/// - when [`Waker`]s are sent to other threads, the data will be lost.
/// - when using a "sub-executor" like `FuturesUnordered`, which also creates
///   its own waker, the data will be lost.
///
/// So try to keep the path from the wrapped future to runtime clean, something
/// like this will generally work:
///
/// ```rust,ignore
/// use std::vec::Vec;
///
/// use compio::runtime::{FutureExt, CancelToken};
/// use compio::fs::File;
///
/// let file = File::open("/tmp/file");
/// let cancel = CancelToken::new();
/// file.read(Vec::with_capacity(1024)).with_cancel(cancel.clone()).await;
/// ```
///
/// [`Waker`]: std::task::Waker
pub trait FutureExt {
    /// Sets the personality for this future.
    ///
    /// This only takes effect on io-uring drivers and will be ignored on other
    /// ones.
    fn with_personality(self, personality: u16) -> WithPersonality<Self>
    where
        Self: Sized,
    {
        WithPersonality::new(self, personality)
    }

    /// Sets the cancel token for this future.
    ///
    /// If multiple [`CancelToken`]s are set, the innermost one (the one being
    /// polled last) will take precedence.
    fn with_cancel(self, token: CancelToken) -> WithCancel<Self>
    where
        Self: Sized,
    {
        WithCancel::new(self, token)
    }

    /// Sets the notify-always flag for this future, which will make the runtime
    /// always notify the waker even if it is on the same thread.
    fn with_notify_always(self, notify: bool) -> WithNotifyAlways<Self>
    where
        Self: Sized,
    {
        WithNotifyAlways::new(self, notify)
    }
}

impl<F: Future + ?Sized> FutureExt for F {}
