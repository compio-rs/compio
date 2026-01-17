use super::*;

/// Extra data for RawOp.
pub struct Extra {
    pub(super) track: Multi<Track>,
}

impl Extra {
    pub fn new() -> Self {
        Self {
            track: Multi::new(),
        }
    }

    pub(super) fn reset(&mut self) {
        self.track.iter_mut().for_each(|t| t.ready = false);
    }

    pub(super) fn set_args(&mut self, args: Multi<WaitArg>) {
        self.track = args.into_iter().map(Into::into).collect();
    }

    pub(super) fn handle_event(&mut self, fd: RawFd) -> bool {
        // First pass: mark all tracks matching this fd as ready.
        let mut found = false;
        for track in self.track.iter_mut() {
            if track.arg.fd == fd {
                track.ready = true;
                found = true;
            }
        }

        // If no track corresponds to this fd, the overall operation is not ready.
        if !found {
            return false;
        }

        // Second pass: check if all tracks are ready.
        self.track.iter().all(|track| track.ready)
    }
}

#[allow(dead_code)]
#[cfg(not(fusion))]
impl crate::sys::Extra {
    pub(in crate::sys) fn try_as_poll(&self) -> Option<&Extra> {
        Some(&self.0)
    }

    pub(in crate::sys) fn try_as_poll_mut(&mut self) -> Option<&mut Extra> {
        Some(&mut self.0)
    }
}

#[allow(dead_code)]
impl crate::sys::Extra {
    pub(in crate::sys) fn as_poll(&self) -> &Extra {
        self.try_as_poll().expect("Current driver is not `polling`")
    }

    pub(in crate::sys) fn as_poll_mut(&mut self) -> &mut Extra {
        self.try_as_poll_mut()
            .expect("Current driver is not `polling`")
    }
}
