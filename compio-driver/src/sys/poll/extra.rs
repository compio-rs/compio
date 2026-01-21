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

    pub(crate) fn next_fd(&self) -> Option<RawFd> {
        self.track.iter().find(|t| !t.ready).map(|t| t.arg.fd)
    }

    pub(super) fn reset(&mut self) {
        self.track.iter_mut().for_each(|t| t.ready = false);
    }

    pub(super) fn set_args(&mut self, args: Multi<WaitArg>) {
        self.track = args.into_iter().map(Into::into).collect();
    }

    pub(super) fn handle_event(&mut self, fd: RawFd) -> bool {
        self.track.iter_mut().fold(true, |curr, t| {
            if t.arg.fd == fd {
                t.ready = true;
            }
            curr && t.ready
        })
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
