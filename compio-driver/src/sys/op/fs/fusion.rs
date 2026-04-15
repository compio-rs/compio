use super::{iour, poll};

crate::macros::fuse_op! {
    <S: AsFd> FileStat(fd: S);
    <S: AsFd> PathStat(dirfd: S, path: CString, follow_symlink: bool);
}
