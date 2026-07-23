use std::num::NonZeroU16;

use compio_buf::BufResult;
use compio_driver::{
    AsRawFd, OpCode, Proactor, ProactorBuilder, PushEntry, ResultTakeBuffer, SharedFd,
    op::ReadManagedAt,
};

#[cfg(not(unix))]
fn build_proactor(num_bufs: u16, buf_len: usize) -> Proactor {
    ProactorBuilder::new()
        .buffer_pool_size(NonZeroU16::new(num_bufs).unwrap())
        .buffer_pool_buffer_len(buf_len)
        .build()
        .unwrap()
}

#[cfg(unix)]
fn build_proactor(num_bufs: u16, buf_len: usize) -> Proactor {
    use std::{mem::MaybeUninit, num::NonZeroUsize, ptr::NonNull};

    use compio_driver::{BufferAllocator, DriverType};
    use nix::sys::mman::{self, MapFlags, ProtFlags};

    struct MmapAllocator;

    impl BufferAllocator for MmapAllocator {
        fn allocate(len: u32) -> NonNull<MaybeUninit<u8>> {
            let size = NonZeroUsize::new(len as usize).expect("Cannot allocate zero-sized buffer");
            let prot = ProtFlags::PROT_READ | ProtFlags::PROT_WRITE;
            let flags = MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS;
            unsafe { mman::mmap_anonymous(None, size, prot, flags) }
                .expect("mmap failed")
                .cast::<MaybeUninit<u8>>()
        }

        unsafe fn deallocate(ptr: NonNull<MaybeUninit<u8>>, len: u32) {
            unsafe { mman::munmap(ptr.cast(), len as usize) }.expect("munmap failed");
        }
    }

    ProactorBuilder::new()
        .driver_type(DriverType::Poll)
        .buffer_pool_allocator::<MmapAllocator>()
        .buffer_pool_size(NonZeroU16::new(num_bufs).unwrap())
        .buffer_pool_buffer_len(buf_len)
        .build()
        .unwrap()
}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> BufResult<usize, O> {
    match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn buffer_pool_pop_and_use() {
    use compio_buf::{IoBuf, IoBufMut, SetLen};

    let mut driver = build_proactor(4, 4096);

    let pool = driver.buffer_pool().unwrap();
    let mut buf = pool.pop().unwrap();

    let data = b"hello compio";
    let uninit = buf.as_uninit();
    uninit[..data.len()]
        .copy_from_slice(unsafe { std::slice::from_raw_parts(data.as_ptr().cast(), data.len()) });
    unsafe { buf.set_len(data.len()) };

    assert_eq!(buf.as_init(), data);
}

#[test]
fn buffer_pool_multiple_buffers() {
    use compio_buf::IoBufMut;

    let mut driver = build_proactor(4, 4096);

    if driver.driver_type().is_iouring() {
        return;
    }

    let pool = driver.buffer_pool().unwrap();

    let mut buf1 = pool.pop().unwrap();
    let mut buf2 = pool.pop().unwrap();

    let p1 = buf1.as_uninit().as_ptr();
    let p2 = buf2.as_uninit().as_ptr();
    assert_ne!(p1, p2);

    drop(buf1);
    drop(buf2);

    let _buf3 = pool.pop().unwrap();
}

#[test]
fn buffer_pool_managed_read() {
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

    let mut driver = build_proactor(4, 8192);

    #[cfg(not(windows))]
    let file = std::fs::File::open("Cargo.toml").unwrap();
    #[cfg(windows)]
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED)
        .open("Cargo.toml")
        .unwrap();

    let fd = SharedFd::new(file);
    driver.attach(fd.as_raw_fd()).unwrap();

    let pool = driver.buffer_pool().unwrap();

    let op = ReadManagedAt::new(fd.clone(), 0, &pool, 1024).unwrap();
    let res = push_and_wait(&mut driver, op);

    let buffer = unsafe { res.take_buffer() }.unwrap().unwrap();
    let content = std::str::from_utf8(&buffer).unwrap();
    assert!(
        content.starts_with("[package]"),
        "unexpected content: {content:?}"
    );

    drop(buffer);
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn buffer_pool_buffer_capacity() {
    use compio_buf::IoBufMut;

    let mut driver = build_proactor(2, 8192);

    let pool = driver.buffer_pool().unwrap();

    let mut buf = pool.pop().unwrap().with_capacity(128);
    assert_eq!(buf.as_uninit().len(), 128);
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn buffer_pool_recycle() {
    let mut driver = build_proactor(1, 4096);

    let pool = driver.buffer_pool().unwrap();

    let buf = pool.pop().unwrap();
    assert!(pool.pop().is_err());
    drop(buf);

    let buf = pool.pop().unwrap();
    drop(driver);
    drop(pool);
    drop(buf);
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn create_buffer_pool_multiple() {
    use compio_buf::IoBufMut;
    use compio_driver::BoxAllocator;

    let mut driver = build_proactor(2, 4096);

    // Default pool
    let pool = driver.buffer_pool().unwrap();

    // Second pool
    let pool2 = driver
        .create_buffer_pool::<BoxAllocator>(2, 4096, 0)
        .unwrap();

    // Default pool still works
    let _buf = pool.pop().unwrap();

    // Second pool also works
    let _buf2 = pool2.pop().unwrap();

    // Third pool with different size
    let pool3 = driver
        .create_buffer_pool::<BoxAllocator>(4, 1024, 0)
        .unwrap();
    let mut buf3 = pool3.pop().unwrap();
    assert_eq!(buf3.as_uninit().len(), 1024);
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn create_buffer_pool_independent() {
    use compio_buf::IoBufMut;
    use compio_driver::BoxAllocator;

    let mut driver = build_proactor(2, 4096);

    let pool_a = driver
        .create_buffer_pool::<BoxAllocator>(2, 4096, 0)
        .unwrap();
    let pool_b = driver
        .create_buffer_pool::<BoxAllocator>(2, 4096, 0)
        .unwrap();

    // Each pool has its own buffers
    let mut buf_a1 = pool_a.pop().unwrap();
    let mut buf_a2 = pool_a.pop().unwrap();
    let mut buf_b1 = pool_b.pop().unwrap();
    let mut buf_b2 = pool_b.pop().unwrap();

    // Verify all pointers are distinct
    let p_a1 = buf_a1.as_uninit().as_ptr();
    let p_a2 = buf_a2.as_uninit().as_ptr();
    let p_b1 = buf_b1.as_uninit().as_ptr();
    let p_b2 = buf_b2.as_uninit().as_ptr();

    assert_ne!(p_a1, p_a2);
    assert_ne!(p_b1, p_b2);
    // Buffers from different pools may have the same address (reallocation),
    // but within the same pool they must be distinct
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn extra_buffer_pool_released_after_last_pool_drop() {
    use std::{
        mem::MaybeUninit,
        ptr::NonNull,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compio_driver::{BoxAllocator, BufferAllocator};

    static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

    struct CountingAllocator;

    impl BufferAllocator for CountingAllocator {
        fn allocate(len: u32) -> NonNull<MaybeUninit<u8>> {
            BoxAllocator::allocate(len)
        }

        unsafe fn deallocate(ptr: NonNull<MaybeUninit<u8>>, len: u32) {
            DEALLOCS.fetch_add(1, Ordering::SeqCst);
            unsafe { BoxAllocator::deallocate(ptr, len) };
        }
    }

    DEALLOCS.store(0, Ordering::SeqCst);

    let mut driver = build_proactor(1, 4096);
    let pool = driver
        .create_buffer_pool::<CountingAllocator>(2, 1024, 0)
        .unwrap();

    drop(pool);
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), 0);

    _ = driver.poll(Some(Duration::ZERO));
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), 2);
}

#[cfg(any(not(target_os = "linux"), feature = "polling"))]
#[test]
fn extra_buffer_pool_waits_for_live_buffer_ref() {
    use std::{
        mem::MaybeUninit,
        ptr::NonNull,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compio_driver::{BoxAllocator, BufferAllocator};

    static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

    struct CountingAllocator;

    impl BufferAllocator for CountingAllocator {
        fn allocate(len: u32) -> NonNull<MaybeUninit<u8>> {
            BoxAllocator::allocate(len)
        }

        unsafe fn deallocate(ptr: NonNull<MaybeUninit<u8>>, len: u32) {
            DEALLOCS.fetch_add(1, Ordering::SeqCst);
            unsafe { BoxAllocator::deallocate(ptr, len) };
        }
    }

    DEALLOCS.store(0, Ordering::SeqCst);

    let mut driver = build_proactor(1, 4096);
    let pool = driver
        .create_buffer_pool::<CountingAllocator>(1, 1024, 0)
        .unwrap();
    let buf = pool.pop().unwrap();

    drop(pool);
    _ = driver.poll(Some(Duration::ZERO));
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), 0);

    drop(buf);
    _ = driver.poll(Some(Duration::ZERO));
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), 1);
}

#[cfg(io_uring)]
#[test]
fn create_buffer_pool_iouring_multiple_groups() {
    use compio_buf::IoBuf;
    use compio_driver::{BoxAllocator, DriverType};

    let mut driver = ProactorBuilder::new()
        .driver_type(DriverType::IoUring)
        .build()
        .unwrap();
    if !driver.driver_type().is_iouring() {
        return;
    }

    let file = std::fs::File::open("Cargo.toml").unwrap();
    let fd = SharedFd::new(file);
    driver.attach(fd.as_raw_fd()).unwrap();

    let pool_a = driver
        .create_buffer_pool::<BoxAllocator>(2, 128, 0)
        .unwrap();
    let pool_b = driver
        .create_buffer_pool::<BoxAllocator>(2, 256, 0)
        .unwrap();

    let op = ReadManagedAt::new(fd.clone(), 0, &pool_a, 32).unwrap();
    let res = push_and_wait(&mut driver, op);
    let buffer = unsafe { res.take_buffer() }.unwrap().unwrap();
    assert!(buffer.as_init().starts_with(b"[package]"));
    drop(buffer);

    let op = ReadManagedAt::new(fd, 0, &pool_b, 32).unwrap();
    let res = push_and_wait(&mut driver, op);
    let buffer = unsafe { res.take_buffer() }.unwrap().unwrap();
    assert!(buffer.as_init().starts_with(b"[package]"));
}
