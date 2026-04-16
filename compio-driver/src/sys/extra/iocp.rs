use crate::sys::pal::*;

/// Extra data attached for IOCP.
#[repr(C)]
#[derive(Debug)]
pub(in crate::sys) struct Extra {
    overlapped: Overlapped,
}

pub(in crate::sys) use Extra as IocpExtra;

impl Default for Extra {
    fn default() -> Self {
        Self {
            overlapped: Overlapped::new(std::ptr::null_mut()),
        }
    }
}

impl Extra {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self {
            overlapped: Overlapped::new(driver),
        }
    }
}

impl super::Extra {
    pub(crate) fn optr(&mut self) -> *mut Overlapped {
        &mut self.0.overlapped as _
    }
}
