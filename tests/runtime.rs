#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

use std::net::Ipv4Addr;

use compio::{
    buf::*,
    fs::File,
    net::{TcpListener, TcpStream},
};
use tempfile::NamedTempFile;

#[test]
fn multi_threading() {
    const DATA: &str = "Hello world!";

    compio::task::block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, (rx, _)) =
            futures_util::try_join!(TcpStream::connect(&addr), listener.accept()).unwrap();

        tx.send_all(DATA).await.0.unwrap();

        let rx = SendWrapper(rx);
        if let Err(e) = std::thread::spawn(move || {
            compio::task::block_on(async {
                let buffer = Vec::with_capacity(DATA.len());
                let (n, buffer) = rx.recv_exact(buffer).await;
                assert_eq!(n.unwrap(), buffer.len());
                assert_eq!(DATA, String::from_utf8(buffer).unwrap());
            });
        })
        .join()
        {
            std::panic::resume_unwind(e)
        }
    });
}

#[test]
fn try_clone() {
    const DATA: &str = "Hello world!";

    compio::task::block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, (rx, _)) =
            futures_util::try_join!(TcpStream::connect(&addr), listener.accept()).unwrap();

        let tx = tx.try_clone().unwrap();
        tx.send_all(DATA).await.0.unwrap();

        let rx = SendWrapper(rx.try_clone().unwrap());
        if let Err(e) = std::thread::spawn(move || {
            compio::task::block_on(async {
                let buffer = Vec::with_capacity(DATA.len());
                let (n, buffer) = rx.recv_exact(buffer).await;
                assert_eq!(n.unwrap(), buffer.len());
                assert_eq!(DATA, String::from_utf8(buffer).unwrap());
            });
        })
        .join()
        {
            std::panic::resume_unwind(e)
        }
    });
}

#[test]
fn drop_on_complete() {
    use std::sync::Arc;

    struct MyBuf {
        data: Vec<u8>,
        _ref_cnt: Arc<()>,
    }

    unsafe impl IoBuf<'static> for MyBuf {
        fn as_buf_ptr(&self) -> *const u8 {
            self.data.as_buf_ptr()
        }

        fn buf_len(&self) -> usize {
            self.data.buf_len()
        }

        fn buf_capacity(&self) -> usize {
            self.data.buf_capacity()
        }
    }

    unsafe impl IoBufMut<'static> for MyBuf {
        fn as_buf_mut_ptr(&mut self) -> *mut u8 {
            self.data.as_buf_mut_ptr()
        }

        fn set_buf_init(&mut self, pos: usize) {
            self.data.set_buf_init(pos);
        }
    }

    // Used to test if the buffer dropped.
    let ref_cnt = Arc::new(());

    let tempfile = tempfile();

    let vec = vec![0; 50 * 1024 * 1024];
    let mut file = std::fs::File::create(tempfile.path()).unwrap();
    std::io::Write::write_all(&mut file, &vec).unwrap();

    let file = compio::task::block_on(async {
        let file = File::open(tempfile.path()).unwrap();
        file.read_at(
            MyBuf {
                data: Vec::with_capacity(64 * 1024),
                _ref_cnt: ref_cnt.clone(),
            },
            25 * 1024 * 1024,
        )
        .await
        .0
        .unwrap();
        file
    });

    assert_eq!(Arc::strong_count(&ref_cnt), 1);

    drop(file);
}

#[test]
fn too_many_submissions() {
    let tempfile = tempfile();

    compio::task::block_on(async {
        let file = File::create(tempfile.path()).unwrap();
        for _ in 0..600 {
            poll_once(async {
                file.write_at("hello world", 0).await.0.unwrap();
            })
            .await;
        }
    });
}

#[test]
#[cfg(feature = "allocator_api")]
fn arena() {
    use std::{
        alloc::{AllocError, Allocator, Layout},
        ptr::NonNull,
    };

    thread_local! {
        static ALLOCATOR: bumpalo::Bump = bumpalo::Bump::new();
    }

    struct ArenaAllocator;

    unsafe impl Allocator for ArenaAllocator {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            ALLOCATOR.with(|alloc| alloc.allocate(layout))
        }

        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            ALLOCATOR.with(|alloc| alloc.deallocate(ptr, layout))
        }
    }

    compio::task::block_on(async {
        let file = File::open("Cargo.toml").unwrap();
        let (read, buffer) = file.read_to_end_at(Vec::new_in(ArenaAllocator), 0).await;
        let read = read.unwrap();
        assert_eq!(buffer.len(), read);
    })
}

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}

async fn poll_once(future: impl std::future::Future) {
    use std::{future::poll_fn, pin::pin, task::Poll};

    let mut future = pin!(future);

    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

struct SendWrapper<T>(pub T);

unsafe impl<T> Send for SendWrapper<T> {}

impl<T> std::ops::Deref for SendWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
