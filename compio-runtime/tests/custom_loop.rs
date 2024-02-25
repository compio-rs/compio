#[cfg(target_os = "macos")]
#[test]
fn cf_run_loop() {
    use std::{os::raw::c_void, time::Duration};

    use compio_driver::AsRawFd;
    use compio_runtime::Runtime;
    use core_foundation::{
        base::TCFType,
        filedescriptor::{kCFFileDescriptorReadCallBack, CFFileDescriptor, CFFileDescriptorRef},
        runloop::{kCFRunLoopDefaultMode, CFRunLoop},
    };
    use futures_util::Future;

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

    let res = runtime.block_on(async {
        compio_runtime::time::sleep(Duration::from_secs(1)).await;
        1
    });
    assert_eq!(res, 1);
}
