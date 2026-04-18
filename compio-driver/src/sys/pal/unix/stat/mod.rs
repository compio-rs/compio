use super::*;

cfg_if! {
    if #[cfg(linux_all)] {
        mod_use![linux];
    } else {
        pub fn stat<Fd: AsFd>(dirfd: Fd, path: &CStr, follow_symlink: bool) -> io::Result<Stat> {
            use rustix::fs::{fstat, statat};

            if path.is_empty() {
                Ok(fstat(dirfd)?)
            } else {
                let flags = if follow_symlink {
                    AtFlags::empty()
                } else {
                    AtFlags::SYMLINK_NOFOLLOW
                };
                Ok(statat(dirfd, path, flags)?)
            }
        }
    }
}
