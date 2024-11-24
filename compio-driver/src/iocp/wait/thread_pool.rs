use std::{ffi::c_void, io, ptr::null};

use windows_sys::Win32::{
    Foundation::{ERROR_IO_PENDING, ERROR_TIMEOUT, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        CloseThreadpoolWait, CreateThreadpoolWait, PTP_CALLBACK_INSTANCE, PTP_WAIT,
        SetThreadpoolWait, WaitForThreadpoolWaitCallbacks,
    },
};

use crate::{Key, OpCode, RawFd, sys::cp, syscall};

pub struct Wait {
    wait: PTP_WAIT,
    // For memory safety.
    #[allow(dead_code)]
    context: Box<WinThreadpoolWaitContext>,
}

impl Wait {
    pub fn new(port: &cp::Port, event: RawFd, op: &mut Key<dyn OpCode>) -> io::Result<Self> {
        let port = port.handle();
        let mut context = Box::new(WinThreadpoolWaitContext {
            port,
            user_data: op.user_data(),
        });
        let wait = syscall!(
            BOOL,
            CreateThreadpoolWait(
                Some(Self::wait_callback),
                (&mut *context) as *mut WinThreadpoolWaitContext as _,
                null()
            )
        )?;
        unsafe {
            SetThreadpoolWait(wait, event as _, null());
        }
        Ok(Self { wait, context })
    }

    unsafe extern "system" fn wait_callback(
        _instance: PTP_CALLBACK_INSTANCE,
        context: *mut c_void,
        _wait: PTP_WAIT,
        result: u32,
    ) {
        let context = &*(context as *mut WinThreadpoolWaitContext);
        let res = match result {
            WAIT_OBJECT_0 => Ok(0),
            WAIT_TIMEOUT => Err(io::Error::from_raw_os_error(ERROR_TIMEOUT as _)),
            _ => Err(io::Error::from_raw_os_error(result as _)),
        };
        let mut op = unsafe { Key::<dyn OpCode>::new_unchecked(context.user_data) };
        context.port.post(res, op.as_mut_ptr()).ok();
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        // Try to cancel it, but don't know whether it is successfully cancelled.
        unsafe {
            SetThreadpoolWait(self.wait, 0, null());
        }
        Err(io::Error::from_raw_os_error(ERROR_IO_PENDING as _))
    }

    pub fn is_cancelled(&self) -> bool {
        false
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        unsafe {
            SetThreadpoolWait(self.wait, 0, null());
            WaitForThreadpoolWaitCallbacks(self.wait, 1);
            CloseThreadpoolWait(self.wait);
        }
    }
}

struct WinThreadpoolWaitContext {
    port: cp::PortHandle,
    user_data: usize,
}
