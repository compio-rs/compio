#![cfg(unix)]

#[cfg(not(any(freebsd, solarish, linux_all)))]
pub use imp::pipe2;
#[cfg(any(freebsd, solarish, linux_all))]
pub use nix::unistd::pipe2;

#[cfg(not(any(freebsd, solarish, linux_all)))]
mod imp {
    use std::os::unix::io::OwnedFd;

    use nix::fcntl::OFlag;

    pub fn pipe2(flags: OFlag) -> nix::Result<(OwnedFd, OwnedFd)> {
        use nix::{
            fcntl::{F_GETFD, F_GETFL, F_SETFD, F_SETFL, FdFlag, fcntl},
            unistd::pipe,
        };

        fn set_cloexec(fd: &OwnedFd) -> nix::Result<()> {
            let flag = FdFlag::from_bits_retain(fcntl(fd, F_GETFD)?);
            fcntl(fd, F_SETFD(flag | FdFlag::FD_CLOEXEC))?;
            Ok(())
        }

        fn set_nonblock(fd: &OwnedFd) -> nix::Result<()> {
            let flag = OFlag::from_bits_retain(fcntl(fd, F_GETFL)?);
            fcntl(fd, F_SETFL(flag | OFlag::O_NONBLOCK))?;
            Ok(())
        }

        let (r, w) = pipe()?;
        if flags.contains(OFlag::O_CLOEXEC) {
            set_cloexec(&r)?;
            set_cloexec(&w)?;
        }
        if flags.contains(OFlag::O_NONBLOCK) {
            set_nonblock(&r)?;
            set_nonblock(&w)?;
        }
        Ok((r, w))
    }
}
