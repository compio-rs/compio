#[cfg(target_os = "macos")]
#[test]
fn cf_run_loop() {
    use std::{future::Future, os::raw::c_void, time::Duration};

    use block2::{Block, ConcreteBlock};
    use compio_driver::AsRawFd;
    use compio_runtime::{event::Event, Runtime};
    use core_foundation::{
        base::TCFType,
        filedescriptor::{kCFFileDescriptorReadCallBack, CFFileDescriptor, CFFileDescriptorRef},
        runloop::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRef},
        string::CFStringRef,
    };

    struct CFRunLoopRuntime {
        runtime: Runtime,
    }

    impl CFRunLoopRuntime {
        pub fn new() -> Self {
            let runtime = Runtime::new().unwrap();

            extern "C" fn callback(
                fdref: CFFileDescriptorRef,
                _callback_types: usize,
                _info: *mut c_void,
            ) {
                let fd = unsafe { CFFileDescriptor::wrap_under_get_rule(fdref) };
                fd.enable_callbacks(kCFFileDescriptorReadCallBack);
            }

            let source = CFFileDescriptor::new(runtime.as_raw_fd(), false, callback, None).unwrap();
            source.enable_callbacks(kCFFileDescriptorReadCallBack);
            let source = source.to_run_loop_source(0).unwrap();

            CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopDefaultMode });

            Self { runtime }
        }

        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            let _guard = self.runtime.enter();
            let mut result = None;
            unsafe {
                self.runtime
                    .spawn_unchecked(async { result = Some(future.await) })
            }
            .detach();
            loop {
                self.runtime.run();
                if let Some(result) = result.take() {
                    break result;
                }
                self.runtime.poll_with(|driver, timeout, entries| {
                    match driver.poll(Some(Duration::ZERO), entries) {
                        Ok(()) => {
                            if !entries.is_empty() {
                                return Ok(());
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(e) => return Err(e),
                    }
                    CFRunLoop::run_in_mode(
                        unsafe { kCFRunLoopDefaultMode },
                        timeout.unwrap_or(Duration::MAX),
                        true,
                    );
                    Ok(())
                });
            }
        }
    }

    let runtime = CFRunLoopRuntime::new();

    runtime.block_on(async {
        let event = Event::new();
        let block = ConcreteBlock::new(|| {
            event.handle().notify();
        });
        extern "C" {
            fn CFRunLoopPerformBlock(rl: CFRunLoopRef, mode: CFStringRef, block: &Block<(), ()>);
        }
        let run_loop = CFRunLoop::get_current();
        unsafe {
            CFRunLoopPerformBlock(
                run_loop.as_concrete_TypeRef(),
                kCFRunLoopDefaultMode,
                &block,
            );
        }
        event.wait().await;
    });
}

#[cfg(windows)]
#[test]
fn message_queue() {
    use std::{future::Future, mem::MaybeUninit, sync::Mutex, time::Duration};

    use compio_driver::AsRawFd;
    use compio_runtime::{
        event::{Event, EventHandle},
        Runtime,
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE, HWND, WAIT_FAILED},
        System::Threading::INFINITE,
        UI::WindowsAndMessaging::{
            DispatchMessageW, KillTimer, MsgWaitForMultipleObjectsEx, PeekMessageW, SetTimer,
            TranslateMessage, MWMO_ALERTABLE, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT,
        },
    };

    struct MQRuntime {
        runtime: Runtime,
    }

    impl MQRuntime {
        pub fn new() -> Self {
            Self {
                runtime: Runtime::new().unwrap(),
            }
        }

        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            let _guard = self.runtime.enter();
            let mut result = None;
            unsafe {
                self.runtime
                    .spawn_unchecked(async { result = Some(future.await) })
            }
            .detach();
            loop {
                self.runtime.run();
                if let Some(result) = result.take() {
                    break result;
                }
                self.runtime.poll_with(|driver, timeout, entries| {
                    match driver.poll(Some(Duration::ZERO), entries) {
                        Ok(()) => {
                            if !entries.is_empty() {
                                return Ok(());
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(e) => return Err(e),
                    }

                    let timeout = match timeout {
                        Some(timeout) => timeout.as_millis() as u32,
                        None => INFINITE,
                    };
                    let handle = driver.as_raw_fd() as HANDLE;
                    let res = unsafe {
                        MsgWaitForMultipleObjectsEx(
                            1,
                            &handle,
                            timeout,
                            QS_ALLINPUT,
                            MWMO_ALERTABLE | MWMO_INPUTAVAILABLE,
                        )
                    };
                    if res == WAIT_FAILED {
                        return Err(std::io::Error::last_os_error());
                    }

                    let mut msg = MaybeUninit::uninit();
                    let res = unsafe { PeekMessageW(msg.as_mut_ptr(), 0, 0, 0, PM_REMOVE) };
                    if res != 0 {
                        let msg = unsafe { msg.assume_init() };
                        unsafe {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }

                    Ok(())
                });
            }
        }
    }

    let runtime = MQRuntime::new();

    runtime.block_on(async {
        static GLOBAL_EVENT: Mutex<Option<EventHandle>> = Mutex::new(None);

        let event = Event::new();
        *GLOBAL_EVENT.lock().unwrap() = Some(event.handle());

        unsafe extern "system" fn timer_callback(hwnd: HWND, _msg: u32, id: usize, _dwtime: u32) {
            let handle = GLOBAL_EVENT.lock().unwrap().take().unwrap();
            handle.notify();
            KillTimer(hwnd, id);
        }

        unsafe {
            SetTimer(0, 0, 1, Some(timer_callback));
        }

        event.wait().await;
    });
}
