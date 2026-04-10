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
