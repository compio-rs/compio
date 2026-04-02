//! Future combinators.

mod cancel;
mod personality;

use std::borrow::Cow;

pub use cancel::*;
use compio_driver::Extra;
pub use personality::*;

use crate::{CancelToken, waker::ExtData};

#[non_exhaustive]
#[derive(Debug, Default)]
pub(crate) struct Ext<'a> {
    personality: Option<u16>,
    cancel: Option<Cow<'a, CancelToken>>,
}

impl<'a> ExtData for Ext<'a> {
    type OwnedExt = Ext<'static>;

    fn to_owned(&self) -> Self::OwnedExt {
        Ext {
            personality: self.personality,
            cancel: self.cancel.clone().map(Cow::into_owned).map(Cow::Owned),
        }
    }

    fn from_owned(owned: &Self::OwnedExt) -> &Self {
        owned
    }
}

impl<'a> Ext<'a> {
    pub fn with_personality(&self, personality: u16) -> Self {
        Self {
            personality: Some(personality),
            cancel: self.cancel.clone(),
        }
    }

    pub fn with_cancel(&self, token: &'a CancelToken) -> Self {
        Self {
            personality: self.personality,
            cancel: Some(Cow::Borrowed(token)),
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
}

/// Extension trait for futures.
///
/// # Implementation
///
/// Extra data are passed down to runtime when the combinators are polled using
/// a custom rolled [`Waker`], and those data are single-threaded. This means
/// - when [`Waker`] are sent to other threads, the data will lost.
/// - when using a "sub-executor" like `FuturesUnordered`, which also creates
///   their own waker, data will lost.
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
    /// If multiple [`CancelToken`]s are set, the inner most one (the one be
    /// polled last) will take precedence.
    fn with_cancel(self, token: CancelToken) -> WithCancel<Self>
    where
        Self: Sized,
    {
        WithCancel::new(self, token)
    }
}

impl<F: Future + ?Sized> FutureExt for F {}
