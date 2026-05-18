use super::*;

cfg_select! {
    apple => {
        mod_use![apple];
    }
    _ => {
        use rustix::{net::*, pipe::{PipeFlags, pipe_with}};

        pub const SOCKET_FLAG: SocketFlags = SocketFlags::NONBLOCK.union(SocketFlags::CLOEXEC);

        pub fn mk_pipe() -> io::Result<[Option<OwnedFd>; 2]> {
            let (a,b) = pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK)?;

            Ok([Some(a), Some(b)])
        }
    }
}
