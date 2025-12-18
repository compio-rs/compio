#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

use std::{net::Ipv4Addr, panic::resume_unwind, time::Duration};

use compio::{
    buf::*,
    fs::File,
    io::{AsyncReadAt, AsyncReadExt, AsyncWriteAt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use compio_driver::ProactorBuilder;
use tempfile::NamedTempFile;

#[compio_macros::test]
#[cfg(feature = "fd-sync")]
async fn multi_threading() {
    const DATA: &str = "Hello world!";

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (mut tx, (mut rx, _)) =
        futures_util::try_join!(TcpStream::connect(&addr), listener.accept()).unwrap();

    tx.write_all(DATA).await.0.unwrap();
    tx.write_all(DATA).await.0.unwrap();

    let ((), buffer) = rx.read_exact(Vec::with_capacity(DATA.len())).await.unwrap();
    assert_eq!(DATA, String::from_utf8(buffer).unwrap());

    compio::runtime::spawn_blocking(move || {
        compio::runtime::Runtime::new().unwrap().block_on(async {
            let ((), buffer) = rx.read_exact(Vec::with_capacity(DATA.len())).await.unwrap();
            assert_eq!(DATA, String::from_utf8(buffer).unwrap());
        });
    })
    .await
    .unwrap_or_else(|e| resume_unwind(e))
}

#[compio_macros::test]
async fn drop_on_complete() {
    use std::sync::Arc;

    struct MyBuf {
        data: Vec<u8>,
        _ref_cnt: Arc<()>,
    }

    impl IoBuf for MyBuf {
        fn as_slice(&self) -> &[u8] {
            self.data.as_slice()
        }
    }

    impl IoBufMut for MyBuf {
        fn as_uninit(&mut self) -> &mut [std::mem::MaybeUninit<u8>] {
            self.data.as_uninit()
        }
    }

    impl SetLen for MyBuf {
        unsafe fn set_len(&mut self, pos: usize) {
            unsafe { self.data.set_len(pos) }
        }
    }

    // Used to test if the buffer dropped.
    let ref_cnt = Arc::new(());

    let tempfile = tempfile();

    let vec = vec![0; 50 * 1024 * 1024];
    let mut file = std::fs::File::create(tempfile.path()).unwrap();
    std::io::Write::write_all(&mut file, &vec).unwrap();

    let file = {
        let file = File::open(tempfile.path()).await.unwrap();
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
    };

    assert_eq!(Arc::strong_count(&ref_cnt), 1);

    drop(file);
}

#[test]
fn too_many_submissions() {
    let mut proactor_builder = ProactorBuilder::new();
    proactor_builder.capacity(1).thread_pool_limit(1);
    compio_runtime::Runtime::builder()
        .with_proactor(proactor_builder)
        .build()
        .unwrap()
        .block_on(async move {
            let tempfile = tempfile();
            let mut file = File::create(tempfile.path()).await.unwrap();
            for _ in 0..600 {
                poll_once(async {
                    file.write_at("hello world", 0).await.0.unwrap();
                })
                .await;
            }
        })
}

#[cfg(feature = "allocator_api")]
#[compio_macros::test]
async fn arena() {
    use std::{
        alloc::{AllocError, Allocator, Layout},
        ptr::NonNull,
    };

    use compio::io::AsyncReadAtExt;

    thread_local! {
        static ALLOCATOR: bumpalo::Bump = bumpalo::Bump::new();
    }

    struct ArenaAllocator;

    unsafe impl Allocator for ArenaAllocator {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            ALLOCATOR.with(|alloc| {
                let ptr = alloc.alloc_layout(layout);
                let slice = NonNull::slice_from_raw_parts(ptr, layout.size());
                Ok(slice)
            })
        }

        unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {}
    }

    let file = File::open("Cargo.toml").await.unwrap();
    let (read, buffer) = file
        .read_to_end_at(Vec::new_in(ArenaAllocator), 0)
        .await
        .unwrap();
    assert_eq!(buffer.len(), read);
}

#[compio_macros::test]
async fn wake_cross_thread() {
    let (tx, rx) = futures_channel::oneshot::channel::<()>();
    let thread = std::thread::spawn(move || compio::runtime::Runtime::new().unwrap().block_on(rx));
    // Sleep and wait for the waker passed into the channel.
    std::thread::sleep(Duration::from_secs(1));
    tx.send(()).unwrap();
    thread.join().unwrap().unwrap();
}

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}

async fn poll_once(future: impl std::future::Future) {
    use std::{future::poll_fn, pin::pin, task::Poll};

    let mut future = pin!(future);

    poll_fn(|cx| {
        let _ = future.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
}
