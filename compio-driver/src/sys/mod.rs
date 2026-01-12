cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod iocp;
        use iocp as imp;
    } else if #[cfg(fusion)] {
        mod fusion;
        mod poll;
        mod iour;

        use fusion as imp;
    } else if #[cfg(io_uring)] {
        mod iour;
        use iour as imp;
    } else if #[cfg(all(target_os = "linux", not(feature = "polling")))] {
        mod stub;
        use stub as imp;
    } else if #[cfg(unix)] {
        mod poll;
        use poll as imp;
    }
}

#[cfg(unix)]
mod unix_op;

mod extra;

pub use extra::Extra;
pub use imp::*;

#[cfg(aio)]
pub(crate) mod aio {
    pub use libc::aiocb;

    pub fn new_aiocb() -> aiocb {
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(not(aio))]
pub(crate) mod aio {
    #[allow(non_camel_case_types)]
    pub type aiocb = ();

    pub fn new_aiocb() -> aiocb {}
}
