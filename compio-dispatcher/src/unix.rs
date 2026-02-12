use std::io;

use nix::sys::signal::{SigSet, SigmaskHow, Signal::*, pthread_sigmask};

pub struct MaskedSignalGuard {
    sigset: SigSet,
}

impl Drop for MaskedSignalGuard {
    fn drop(&mut self) {
        let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.sigset), None);
    }
}

/// Block standard signals before spawning workers.
pub fn mask_signal(block_signals: bool) -> io::Result<Option<MaskedSignalGuard>> {
    if !block_signals {
        return Ok(None);
    }

    let mut set = SigSet::empty();
    set.add(SIGINT);
    set.add(SIGINT);
    set.add(SIGTERM);
    set.add(SIGQUIT);
    set.add(SIGHUP);
    set.add(SIGUSR1);
    set.add(SIGUSR2);
    set.add(SIGPIPE);

    let mut old_set = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&set), Some(&mut old_set))?;

    Ok(Some(MaskedSignalGuard { sigset: old_set }))
}
