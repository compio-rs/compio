//! Future combinators.

mod personality;
use compio_driver::Extra;
pub use personality::*;

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub(crate) struct Ext {
    personality: Option<u16>,
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
}
