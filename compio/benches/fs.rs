#[cfg(windows)]
use std::io::Seek;
use std::{
    io::{Read, SeekFrom, Write},
    path::Path,
};

use compio_buf::{IntoInner, IoBuf};
use compio_io::{AsyncReadAtExt, AsyncWriteAtExt};
use criterion::{criterion_group, criterion_main, Bencher, Criterion};
use futures_util::{stream::FuturesUnordered, StreamExt};
use rand::{thread_rng, Rng, RngCore};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[cfg(target_os = "linux")]
struct MonoioRuntime(std::cell::RefCell<monoio::Runtime<monoio::IoUringDriver>>);

#[cfg(target_os = "linux")]
impl criterion::async_executor::AsyncExecutor for MonoioRuntime {
    fn block_on<T>(&self, future: impl futures_util::Future<Output = T>) -> T {
        self.0.borrow_mut().block_on(future)
    }
}

#[cfg(target_os = "linux")]
impl criterion::async_executor::AsyncExecutor for &MonoioRuntime {
    fn block_on<T>(&self, future: impl futures_util::Future<Output = T>) -> T {
        self.0.borrow_mut().block_on(future)
    }
}

criterion_group!(fs, read, write);
criterion_main!(fs);

const BUFFER_SIZE: usize = 4096;

fn read_std(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    b.iter(|| {
        #[allow(unused_mut)]
        let mut file = std::fs::File::open(path).unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        for &offset in *offsets {
            #[cfg(windows)]
            {
                file.seek(SeekFrom::Start(offset)).unwrap();
                file.read_exact(&mut buffer).unwrap();
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                file.read_exact_at(&mut buffer, offset).unwrap();
            }
        }
        buffer
    })
}

fn read_tokio(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter(|| async {
        let mut file = tokio::fs::File::open(path).await.unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        for &offset in *offsets {
            file.seek(SeekFrom::Start(offset)).await.unwrap();
            file.read_exact(&mut buffer).await.unwrap();
        }
        buffer
    })
}

fn read_compio(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter(|| async {
        let file = compio::fs::File::open(path).await.unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        for &offset in *offsets {
            (_, buffer) = file.read_exact_at(buffer, offset).await.unwrap();
        }
        buffer
    })
}

fn read_compio_join(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter(|| async {
        let file = compio::fs::File::open(path).await.unwrap();
        offsets
            .iter()
            .map(|offset| async {
                let buffer = [0u8; BUFFER_SIZE];
                let (_, buffer) = file.read_exact_at(buffer, *offset).await.unwrap();
                buffer
            })
            .collect::<FuturesUnordered<_>>()
            .collect::<Vec<_>>()
            .await
    })
}

#[cfg(target_os = "linux")]
fn read_monoio(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = MonoioRuntime(std::cell::RefCell::new(
        monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .unwrap(),
    ));
    b.to_async(&runtime).iter(|| async {
        let file = monoio::fs::File::open(path).await.unwrap();
        let mut buffer = Box::new([0u8; BUFFER_SIZE]);
        for &offset in *offsets {
            let res;
            (res, buffer) = file.read_exact_at(buffer, offset).await;
            res.unwrap();
        }
        buffer
    })
}

fn read_all_std(b: &mut Bencher, (path, len): &(&Path, u64)) {
    b.iter(|| {
        let mut file = std::fs::File::open(path).unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut read_len = 0;
        while read_len < *len {
            file.read_exact(&mut buffer).unwrap();
            read_len += BUFFER_SIZE as u64;
        }
        buffer
    })
}

fn read_all_tokio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter(|| async {
        let mut file = tokio::fs::File::open(path).await.unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut read_len = 0;
        while read_len < *len {
            file.read_exact(&mut buffer).await.unwrap();
            read_len += BUFFER_SIZE as u64;
        }
        buffer
    })
}

fn read_all_compio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter(|| async {
        let file = compio::fs::File::open(path).await.unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut read_len = 0;
        while read_len < *len {
            (_, buffer) = file.read_exact_at(buffer, read_len).await.unwrap();
            read_len += BUFFER_SIZE as u64;
        }
        buffer
    })
}

fn read_all_compio_join(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter(|| async {
        let file = compio::fs::File::open(path).await.unwrap();
        let tasks = len / BUFFER_SIZE as u64;
        (0..tasks)
            .map(|offset| {
                let file = &file;
                async move {
                    let buffer = [0u8; BUFFER_SIZE];
                    let (_, buffer) = file.read_exact_at(buffer, offset).await.unwrap();
                    buffer
                }
            })
            .collect::<FuturesUnordered<_>>()
            .collect::<Vec<_>>()
            .await
    })
}

#[cfg(target_os = "linux")]
fn read_all_monoio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = MonoioRuntime(std::cell::RefCell::new(
        monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .unwrap(),
    ));
    b.to_async(&runtime).iter(|| async {
        let file = monoio::fs::File::open(path).await.unwrap();
        let mut buffer = Box::new([0u8; BUFFER_SIZE]);
        let mut read_len = 0;
        while read_len < *len {
            let res;
            (res, buffer) = file.read_exact_at(buffer, read_len).await;
            res.unwrap();
            read_len += BUFFER_SIZE as u64;
        }
        buffer
    })
}

fn read(c: &mut Criterion) {
    const FILE_SIZE: u64 = 1024;

    let mut rng = thread_rng();

    let mut file = NamedTempFile::new().unwrap();
    for _i in 0..FILE_SIZE {
        let mut buffer = [0u8; BUFFER_SIZE];
        rng.fill_bytes(&mut buffer);
        file.write_all(&buffer).unwrap();
    }
    file.flush().unwrap();
    let path = file.into_temp_path();

    let mut offsets = vec![];
    for _i in 0..64 {
        let offset = rng.gen_range(0u64..FILE_SIZE) * BUFFER_SIZE as u64;
        offsets.push(offset);
    }

    let mut group = c.benchmark_group("read");

    group.bench_with_input::<_, _, (&Path, &[u64])>("std-random", &(&path, &offsets), read_std);
    group.bench_with_input::<_, _, (&Path, &[u64])>("tokio-random", &(&path, &offsets), read_tokio);
    group.bench_with_input::<_, _, (&Path, &[u64])>(
        "compio-random",
        &(&path, &offsets),
        read_compio,
    );
    group.bench_with_input::<_, _, (&Path, &[u64])>(
        "compio-random-join",
        &(&path, &offsets),
        read_compio_join,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64])>(
        "monoio-random",
        &(&path, &offsets),
        read_monoio,
    );

    group.bench_with_input::<_, _, (&Path, u64)>("std-all", &(&path, FILE_SIZE), read_all_std);
    group.bench_with_input::<_, _, (&Path, u64)>("tokio-all", &(&path, FILE_SIZE), read_all_tokio);
    group.bench_with_input::<_, _, (&Path, u64)>(
        "compio-all",
        &(&path, FILE_SIZE),
        read_all_compio,
    );
    group.bench_with_input::<_, _, (&Path, u64)>(
        "compio-all-join",
        &(&path, FILE_SIZE),
        read_all_compio_join,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, u64)>(
        "monoio-all",
        &(&path, FILE_SIZE),
        read_all_monoio,
    );

    group.finish();
}

fn write_std(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    b.iter(|| {
        #[allow(unused_mut)]
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        for &offset in *offsets {
            #[cfg(windows)]
            {
                file.seek(SeekFrom::Start(offset)).unwrap();
                file.write_all(content).unwrap();
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                file.write_all_at(content, offset).unwrap();
            }
        }
    })
}

fn write_tokio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter(|| async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await
            .unwrap();
        for &offset in *offsets {
            file.seek(SeekFrom::Start(offset)).await.unwrap();
            file.write_all(content).await.unwrap();
        }
    })
}

fn write_compio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| {
        let mut content = content.clone();
        async {
            let mut file = compio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();
            for &offset in *offsets {
                (_, content) = file.write_all_at(content, offset).await.unwrap();
            }
        }
    })
}

fn write_compio_join(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| async {
        let file = compio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await
            .unwrap();
        offsets
            .iter()
            .map(|offset| {
                let mut file = &file;
                let content = content.clone();
                async move { file.write_all_at(content, *offset).await.unwrap() }
            })
            .collect::<FuturesUnordered<_>>()
            .collect::<Vec<_>>()
            .await
    })
}

#[cfg(target_os = "linux")]
fn write_monoio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = MonoioRuntime(std::cell::RefCell::new(
        monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .unwrap(),
    ));
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| {
        let mut content = content.clone();
        async {
            let file = monoio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();
            for &offset in *offsets {
                let res;
                (res, content) = file.write_all_at(content, offset).await;
                res.unwrap();
            }
        }
    })
}

#[cfg(target_os = "linux")]
fn write_monoio_join(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = MonoioRuntime(std::cell::RefCell::new(
        monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .unwrap(),
    ));
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| {
        let content = content.clone();
        async move {
            let file = monoio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();
            offsets
                .iter()
                .map(|offset| {
                    let file = &file;
                    let content = content.clone();
                    async move {
                        let (res, content) = file.write_all_at(content, *offset).await;
                        res.unwrap();
                        content
                    }
                })
                .collect::<FuturesUnordered<_>>()
                .collect::<Vec<_>>()
                .await
        }
    })
}

fn write_all_std(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    b.iter(|| {
        let mut file = std::fs::File::create(path).unwrap();
        for buffer in content.windows(BUFFER_SIZE) {
            file.write_all(buffer).unwrap();
        }
    })
}

fn write_all_compio(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| {
        let mut content = content.clone();
        async {
            let mut file = compio::fs::File::create(path).await.unwrap();
            let mut write_len = 0;
            while write_len < content.len() as u64 {
                let (_, slice) = file
                    .write_all_at(
                        content.slice(write_len as usize..write_len as usize + BUFFER_SIZE),
                        write_len,
                    )
                    .await
                    .unwrap();
                write_len += BUFFER_SIZE as u64;
                content = slice.into_inner();
            }
        }
    })
}

#[cfg(target_os = "linux")]
fn write_all_monoio(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = MonoioRuntime(std::cell::RefCell::new(
        monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .build()
            .unwrap(),
    ));
    let content = content.to_vec();
    b.to_async(&runtime).iter(|| {
        let mut content = content.clone();
        async {
            let file = monoio::fs::File::create(path).await.unwrap();
            let mut write_len = 0;
            while write_len < content.len() as u64 {
                let (res, slice) = file
                    .write_all_at(
                        monoio::buf::IoBuf::slice(
                            content,
                            write_len as usize..write_len as usize + BUFFER_SIZE,
                        ),
                        write_len,
                    )
                    .await;
                res.unwrap();
                write_len += BUFFER_SIZE as u64;
                content = slice.into_inner();
            }
        }
    })
}

fn write(c: &mut Criterion) {
    const FILE_SIZE: u64 = 1024;
    const WRITE_FILE_SIZE: u64 = 16;

    let mut rng = thread_rng();

    let mut file = NamedTempFile::new().unwrap();
    for _i in 0..FILE_SIZE {
        let mut buffer = [0u8; BUFFER_SIZE];
        rng.fill_bytes(&mut buffer);
        file.write_all(&buffer).unwrap();
    }
    file.flush().unwrap();
    let path = file.into_temp_path();

    let mut single_content = [0u8; BUFFER_SIZE];
    rng.fill_bytes(&mut single_content);

    let mut offsets = vec![];
    for _i in 0..64 {
        let offset = rng.gen_range(0u64..FILE_SIZE) * BUFFER_SIZE as u64;
        offsets.push(offset);
    }

    let mut content = vec![];
    for _i in 0..WRITE_FILE_SIZE {
        let mut buffer = [0u8; BUFFER_SIZE];
        rng.fill_bytes(&mut buffer);
        content.extend_from_slice(&buffer);
    }

    let mut group = c.benchmark_group("write");

    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "std-random",
        &(&path, &offsets, &single_content),
        write_std,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "tokio-random",
        &(&path, &offsets, &single_content),
        write_tokio,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "compio-random",
        &(&path, &offsets, &single_content),
        write_compio,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "compio-random-join",
        &(&path, &offsets, &single_content),
        write_compio_join,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "monoio-random",
        &(&path, &offsets, &single_content),
        write_monoio,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "monoio-random-join",
        &(&path, &offsets, &single_content),
        write_monoio_join,
    );

    group.bench_with_input::<_, _, (&Path, &[u8])>("std-all", &(&path, &content), write_all_std);
    group.bench_with_input::<_, _, (&Path, &[u8])>(
        "compio-all",
        &(&path, &content),
        write_all_compio,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u8])>(
        "monoio-all",
        &(&path, &content),
        write_all_monoio,
    );

    group.finish()
}
