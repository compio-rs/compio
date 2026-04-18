mod buffer_pool;
mod driver;
mod extra;
mod pal;
mod sys_slice;

// Publicly visible items
pub mod op;
pub use driver::*;
pub use extra::Extra;
pub use op::{ResultTakeBuffer, TakeBuffer};
#[allow(unused_imports)]
pub use pal::reexport::*;
pub use pal::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

// Crate-internal items
#[allow(unused_imports)]
pub(crate) use self::buffer_pool::BufControl;
#[allow(unused_imports)]
#[cfg(io_uring)]
pub(crate) use self::pal::is_op_supported;

/// Internal prelude module that includes all necessary utils for sys module
#[allow(unused_imports)]
mod prelude {
    pub(super) use std::{
        collections::VecDeque,
        ffi::{CStr, CString},
        io,
        marker::PhantomData,
        mem::ManuallyDrop,
        ptr::{NonNull, null, null_mut, read_unaligned},
        task::{Poll, Wake, Waker},
        time::Duration,
    };

    #[cfg(any(windows, io_uring))]
    cfg_if! {
        if #[cfg(feature = "once_cell_try")] {
            pub(super) use std::sync::OnceLock;
        } else {
            pub(super) use once_cell::sync::OnceCell as OnceLock;
        }
    }

    pub(super) use cfg_if::cfg_if;
    pub(super) use compio_buf::*;
    pub(super) use compio_log::*;
    pub(super) use mod_use::mod_use;
    pub(super) use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

    pub(super) use crate::{
        BufferPool, BufferRef, DriverType, ProactorBuilder, SharedFd, ToSharedFd,
        control::Carrier,
        key::ErasedKey,
        sys::{
            extra::Extra,
            pal::{self, *},
            sys_slice::*,
        },
        syscall,
    };
}
