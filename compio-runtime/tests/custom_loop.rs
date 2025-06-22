#[cfg(target_os = "macos")]
#[test]
fn cf_run_loop() {
    use std::{
        future::Future,
        os::raw::c_void,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use block2::{Block, StackBlock};
    use compio_driver::AsRawFd;
    use compio_runtime::{Runtime, event::Event};
    use core_foundation::{
        base::TCFType,
        filedescriptor::{CFFileDescriptor, CFFileDescriptorRef, kCFFileDescriptorReadCallBack},
        runloop::{CFRunLoop, CFRunLoopRef, CFRunLoopStop, kCFRunLoopDefaultMode},
        string::CFStringRef,
    };

    struct CFRunLoopRuntime {
        runtime: Runtime,
        fd_source: CFFileDescriptor,
    }

    impl CFRunLoopRuntime {
        pub fn new() -> Self {
            let runtime = Runtime::new().unwrap();

            extern "C" fn callback(
                _fdref: CFFileDescriptorRef,
                _callback_types: usize,
                _info: *mut c_void,
            ) {
            }

            let fd_source =
                CFFileDescriptor::new(runtime.as_raw_fd(), false, callback, None).unwrap();
            let source = fd_source.to_run_loop_source(0).unwrap();

            CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopDefaultMode });

            Self { runtime, fd_source }
        }

        pub fn block_on<F: Future>(&self, future: F) -> F::Output {
            self.runtime.enter(|| {
                let mut result = None;
                unsafe {
                    self.runtime
                        .spawn_unchecked(async { result = Some(future.await) })
                }
                .detach();
                loop {
                    self.runtime.poll_with(Some(Duration::ZERO));

                    let remaining_tasks = self.runtime.run();
                    if let Some(result) = result.take() {
                        break result;
                    }

                    let timeout = if remaining_tasks {
                        Some(Duration::ZERO)
                    } else {
                        self.runtime.current_timeout()
                    };
                    self.fd_source
                        .enable_callbacks(kCFFileDescriptorReadCallBack);
                    CFRunLoop::run_in_mode(
                        unsafe { kCFRunLoopDefaultMode },
                        timeout.unwrap_or(Duration::MAX),
                        true,
                    );
                }
            })
        }
    }

    let runtime = CFRunLoopRuntime::new();

    runtime.block_on(async {
        compio_runtime::time::sleep(Duration::from_secs(1)).await;

        let event = Event::new();
        let handle = Arc::new(Mutex::new(Some(event.handle())));
        let run_loop = CFRunLoop::get_current();
        let block = StackBlock::new(move || {
            handle.lock().unwrap().take().unwrap().notify();
            unsafe {
                CFRunLoopStop(run_loop.as_concrete_TypeRef());
            }
        });
        extern "C" {
            fn CFRunLoopPerformBlock(rl: CFRunLoopRef, mode: CFStringRef, block: &Block<dyn Fn()>);
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
        Runtime,
        event::{Event, EventHandle},
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE, HWND, WAIT_FAILED},
        System::Threading::INFINITE,
        UI::WindowsAndMessaging::{
            DispatchMessageW, KillTimer, MWMO_ALERTABLE, MWMO_INPUTAVAILABLE,
            MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT, SetTimer,
            TranslateMessage,
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
            self.runtime.enter(|| {
                let mut result = None;
                unsafe {
                    self.runtime
                        .spawn_unchecked(async { result = Some(future.await) })
                }
                .detach();
                loop {
                    self.runtime.poll_with(Some(Duration::ZERO));

                    let remaining_tasks = self.runtime.run();
                    if let Some(result) = result.take() {
                        break result;
                    }

                    let timeout = if remaining_tasks {
                        Some(Duration::ZERO)
                    } else {
                        self.runtime.current_timeout()
                    };
                    let timeout = match timeout {
                        Some(timeout) => timeout.as_millis() as u32,
                        None => INFINITE,
                    };
                    let handle = self.runtime.as_raw_fd() as HANDLE;
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
                        panic!("{:?}", std::io::Error::last_os_error());
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
                }
            })
        }
    }

    let runtime = MQRuntime::new();

    runtime.block_on(async {
        compio_runtime::time::sleep(Duration::from_secs(1)).await;

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
