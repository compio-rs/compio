use std::{
    fmt::Debug,
    io,
    ptr::{null, null_mut},
    sync::Arc,
    time::Duration,
};

use windows_sys::Win32::System::Threading::{
    CloseThreadpool, CloseThreadpoolCleanupGroup, CloseThreadpoolCleanupGroupMembers,
    CreateThreadpool, CreateThreadpoolCleanupGroup, PTP_CALLBACK_INSTANCE,
    SetThreadpoolThreadMaximum, TP_CALLBACK_ENVIRON_V3, TP_CALLBACK_PRIORITY_NORMAL,
    TrySubmitThreadpoolCallback,
};

use super::{DispatchError, Dispatchable};
use crate::syscall;

struct PoolEnv(TP_CALLBACK_ENVIRON_V3);

impl PoolEnv {
    fn as_ref(&self) -> &TP_CALLBACK_ENVIRON_V3 {
        &self.0
    }
}

unsafe impl Send for PoolEnv {}
unsafe impl Sync for PoolEnv {}

impl Drop for PoolEnv {
    fn drop(&mut self) {
        unsafe {
            let pool = self.0.Pool;
            let group = self.0.CleanupGroup;
            CloseThreadpoolCleanupGroupMembers(group, 1, null_mut());
            CloseThreadpoolCleanupGroup(group);
            CloseThreadpool(pool);
        }
    }
}

#[derive(Clone)]
pub struct AsyncifyPool {
    inner: Option<Arc<PoolEnv>>,
}

impl Debug for AsyncifyPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncifyPool").finish_non_exhaustive()
    }
}

impl AsyncifyPool {
    pub fn new(thread_limit: usize, _recv_timeout: Duration) -> io::Result<Self> {
        if thread_limit == 0 {
            Ok(Self { inner: None })
        } else {
            let pool = syscall!(BOOL, CreateThreadpool(null()))?;
            let group = syscall!(BOOL, CreateThreadpoolCleanupGroup())?;
            let inner = TP_CALLBACK_ENVIRON_V3 {
                Version: 3,
                Pool: pool,
                CleanupGroup: group,
                CallbackPriority: TP_CALLBACK_PRIORITY_NORMAL,
                Size: size_of::<TP_CALLBACK_ENVIRON_V3>() as u32,
                ..Default::default()
            };
            unsafe {
                SetThreadpoolThreadMaximum(pool, thread_limit as _);
            }
            Ok(Self {
                inner: Some(Arc::new(PoolEnv(inner))),
            })
        }
    }

    pub fn dispatch<D: Dispatchable>(&self, f: D) -> Result<(), DispatchError<D>> {
        unsafe extern "system" fn callback<F: Dispatchable>(
            _: PTP_CALLBACK_INSTANCE,
            callback: *mut std::ffi::c_void,
        ) {
            unsafe {
                Box::from_raw(callback as *mut F).run();
            }
        }

        if let Some(inner) = &self.inner {
            let f = Box::new(f);
            let ptr = Box::into_raw(f);
            let res = syscall!(
                BOOL,
                TrySubmitThreadpoolCallback(
                    Some(callback::<D>),
                    ptr.cast(),
                    inner.as_ref().as_ref(),
                )
            );
            match res {
                Ok(_) => Ok(()),
                Err(_) => Err(DispatchError(*unsafe { Box::from_raw(ptr) })),
            }
        } else {
            panic!("the thread pool is needed but no worker thread is running");
        }
    }
}
