use std::num::NonZeroUsize;

/// Routing policy used by a process group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    /// Routes each message to the member after the previous selection.
    #[default]
    RoundRobin,
}

impl Strategy {
    pub(super) fn select(&self, cursor: &mut usize, members: NonZeroUsize) -> usize {
        match self {
            Self::RoundRobin => {
                let selected = *cursor % members.get();
                *cursor = cursor.wrapping_add(1);
                selected
            }
        }
    }
}
