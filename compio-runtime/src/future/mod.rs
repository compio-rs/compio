//! Future combinators.

mod cancel;
mod personality;

pub use cancel::*;
use compio_driver::Extra;
pub use personality::*;

use crate::CancelToken;

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub(crate) struct Ext {
    personality: Option<u16>,
    cancel: Option<CancelToken>,
}

impl Ext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_personality(self, personality: u16) -> Self {
        Self {
            personality: Some(personality),
            ..self
        }
    }

    pub fn set_personality(&mut self, personality: u16) {
        self.personality = Some(personality);
    }

    pub fn get_cancel(&self) -> Option<&CancelToken> {
        self.cancel.as_ref()
    }

    pub fn with_cancel(mut self, token: &CancelToken) -> Self {
        self.set_cancel(token);
        self
    }

    pub fn set_cancel(&mut self, cancel: &CancelToken) {
        // to avoid unnecessary clones
        if self.cancel.as_ref().is_some_and(|c| c == cancel) {
            return;
        }
        self.cancel = Some(cancel.clone());
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
pub trait FutureExt {
    /// Sets the personality for this future.
    fn with_personality(self, personality: u16) -> WithPersonality<Self>
    where
        Self: Sized,
    {
        WithPersonality::new(self, personality)
    }

    /// Sets the cancel token for this future.
    fn with_cancel(self, token: CancelToken) -> WithCancel<Self>
    where
        Self: Sized,
    {
        WithCancel::new(self, token)
    }
}

impl<F: Future + ?Sized> FutureExt for F {}
