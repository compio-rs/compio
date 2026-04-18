use rustix::{fs::*, io::*, net::*, pipe::pipe};

use super::*;

pub const SOCKET_FLAG: SocketFlags = SocketFlags::empty();

pub fn mk_pipe() -> io::Result<[Option<OwnedFd>; 2]> {
    let (a, b) = pipe()?;

    fn set_cloexec(fd: &OwnedFd) -> Result<()> {
        fcntl_setfd(fd, fcntl_getfd(fd)? | FdFlags::CLOEXEC)
    }

    fn set_nonblock(fd: &OwnedFd) -> Result<()> {
        fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)
    }

    set_cloexec(&a)?;
    set_cloexec(&b)?;
    set_nonblock(&a)?;
    set_nonblock(&b)?;

    Ok([Some(a), Some(b)])
}
