use super::*;

cfg_select! {
     linux_all => {
        mod_use![linux];
    }
    _ => {
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
