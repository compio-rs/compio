use std::{
    hint::black_box,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

use compio_buf::{IntoInner, IoBuf};
use compio_io::{AsyncReadAt, AsyncWriteAt};
use criterion::{Bencher, Criterion, Throughput, criterion_group, criterion_main};
use futures_util::{StreamExt, stream::FuturesUnordered};
use rand::{Rng, RngCore, rng};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[cfg(target_os = "linux")]
mod monoio_wrap;
#[cfg(target_os = "linux")]
use monoio_wrap::MonoioRuntime;

criterion_group!(fs, read, write);
criterion_main!(fs);

const BUFFER_SIZE: usize = 4096;

fn read_std(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let file = std::fs::File::open(path).unwrap();
    b.iter(|| {
        let mut buffer = [0u8; BUFFER_SIZE];
        for &offset in *offsets {
            #[cfg(windows)]
            {
                use std::os::windows::fs::FileExt;
                file.seek_read(&mut buffer, offset).unwrap();
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                file.read_at(&mut buffer, offset).unwrap();
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
    b.to_async(&runtime).iter_custom(|iter| async move {
        let mut file = tokio::fs::File::open(path).await.unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let mut buffer = [0u8; BUFFER_SIZE];
            for &offset in *offsets {
                file.seek(SeekFrom::Start(offset)).await.unwrap();
                _ = file.read(&mut buffer).await.unwrap();
            }
            black_box(buffer);
        }
        start.elapsed()
    })
}

fn read_tokio_std(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = Arc::new(std::fs::File::open(path).unwrap());

        let start = Instant::now();
        for _i in 0..iter {
            let mut buffer = [0u8; BUFFER_SIZE];
            for &offset in *offsets {
                let file = file.clone();
                buffer = tokio::task::spawn_blocking(move || {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::FileExt;
                        file.seek_read(&mut buffer, offset).unwrap();
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::FileExt;
                        file.read_at(&mut buffer, offset).unwrap();
                    }
                    buffer
                })
                .await
                .unwrap();
            }
            black_box(buffer);
        }
        start.elapsed()
    })
}

fn read_compio(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = compio::fs::File::open(path).await.unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let mut buffer = Box::new([0u8; BUFFER_SIZE]);
            for &offset in *offsets {
                (_, buffer) = file.read_at(buffer, offset).await.unwrap();
            }
            black_box(buffer);
        }
        start.elapsed()
    })
}

fn read_compio_join(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = compio::fs::File::open(path).await.unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let res = offsets
                .iter()
                .map(|offset| async {
                    let buffer = Box::new([0u8; BUFFER_SIZE]);
                    let (_, buffer) = file.read_at(buffer, *offset).await.unwrap();
                    buffer
                })
                .collect::<FuturesUnordered<_>>()
                .collect::<Vec<_>>()
                .await;
            black_box(res);
        }
        start.elapsed()
    })
}

#[cfg(target_os = "linux")]
fn read_monoio(b: &mut Bencher, (path, offsets): &(&Path, &[u64])) {
    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = monoio::fs::File::open(path).await.unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let mut buffer = Box::new([0u8; BUFFER_SIZE]);
            for &offset in *offsets {
                let res;
                (res, buffer) = file.read_at(buffer, offset).await;
                res.unwrap();
            }
            black_box(buffer);
        }
        start.elapsed()
    })
}

fn read_all_std(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let mut file = std::fs::File::open(path).unwrap();
    b.iter(|| {
        let mut buffer = [0u8; BUFFER_SIZE];
        let mut read_len = 0;
        file.seek(SeekFrom::Start(0)).unwrap();
        while read_len < *len {
            let read = file.read(&mut buffer).unwrap();
            read_len += read as u64;
        }
        buffer
    })
}

fn read_all_tokio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let mut file = tokio::fs::File::open(path).await.unwrap();
        let mut buffer = [0u8; BUFFER_SIZE];

        let start = Instant::now();
        for _i in 0..iter {
            let mut read_len = 0;
            file.seek(SeekFrom::Start(0)).await.unwrap();
            while read_len < *len {
                let read = file.read(&mut buffer).await.unwrap();
                read_len += read as u64;
            }
        }
        start.elapsed()
    })
}

fn read_all_tokio_std(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let mut buffer = [0u8; BUFFER_SIZE];

        let start = Instant::now();
        for _i in 0..iter {
            let mut file = std::fs::File::open(path).unwrap();
            let len = *len;
            buffer = tokio::task::spawn_blocking(move || {
                let mut read_len = 0;
                file.seek(SeekFrom::Start(0)).unwrap();
                while read_len < len {
                    let read = file.read(&mut buffer).unwrap();
                    read_len += read as u64;
                }
                buffer
            })
            .await
            .unwrap();
        }
        start.elapsed()
    })
}

fn read_all_compio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = compio::fs::File::open(path).await.unwrap();
        let mut buffer = Box::new([0u8; BUFFER_SIZE]);

        let start = Instant::now();
        for _i in 0..iter {
            let mut read_len = 0;
            while read_len < *len {
                let read;
                (read, buffer) = file.read_at(buffer, read_len).await.unwrap();
                read_len += read as u64;
            }
        }
        start.elapsed()
    })
}

#[cfg(target_os = "linux")]
fn read_all_monoio(b: &mut Bencher, (path, len): &(&Path, u64)) {
    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = monoio::fs::File::open(path).await.unwrap();
        let mut buffer = Box::new([0u8; BUFFER_SIZE]);

        let start = Instant::now();
        for _i in 0..iter {
            let mut read_len = 0;
            while read_len < *len {
                let read;
                (read, buffer) = file.read_at(buffer, read_len).await;
                read_len += read.unwrap() as u64;
            }
        }
        start.elapsed()
    })
}

fn read(c: &mut Criterion) {
    const FILE_SIZE: u64 = 1024;

    let mut rng = rng();

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
        let offset = rng.random_range(0u64..FILE_SIZE) * BUFFER_SIZE as u64;
        offsets.push(offset);
    }

    let mut group = c.benchmark_group("read-random");

    group.bench_with_input::<_, _, (&Path, &[u64])>("std", &(&path, &offsets), read_std);
    group.bench_with_input::<_, _, (&Path, &[u64])>("tokio", &(&path, &offsets), read_tokio);
    group.bench_with_input::<_, _, (&Path, &[u64])>(
        "tokio_std",
        &(&path, &offsets),
        read_tokio_std,
    );
    group.bench_with_input::<_, _, (&Path, &[u64])>("compio", &(&path, &offsets), read_compio);
    group.bench_with_input::<_, _, (&Path, &[u64])>(
        "compio-join",
        &(&path, &offsets),
        read_compio_join,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64])>("monoio", &(&path, &offsets), read_monoio);

    group.finish();

    let mut group = c.benchmark_group("read-all");
    group.throughput(Throughput::Bytes(FILE_SIZE * BUFFER_SIZE as u64));

    group.bench_with_input::<_, _, (&Path, u64)>("std", &(&path, FILE_SIZE), read_all_std);
    group.bench_with_input::<_, _, (&Path, u64)>("tokio", &(&path, FILE_SIZE), read_all_tokio);
    group.bench_with_input::<_, _, (&Path, u64)>(
        "tokio_std",
        &(&path, FILE_SIZE),
        read_all_tokio_std,
    );
    group.bench_with_input::<_, _, (&Path, u64)>("compio", &(&path, FILE_SIZE), read_all_compio);
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, u64)>("monoio", &(&path, FILE_SIZE), read_all_monoio);

    group.finish();
}

fn write_std(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    b.iter(|| {
        for &offset in *offsets {
            #[cfg(windows)]
            {
                use std::os::windows::fs::FileExt;
                file.seek_write(content, offset).unwrap();
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt;
                file.write_at(content, offset).unwrap();
            }
        }
    })
}

fn write_tokio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await
            .unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            for &offset in *offsets {
                file.seek(SeekFrom::Start(offset)).await.unwrap();
                _ = file.write(content).await.unwrap();
            }
        }
        start.elapsed()
    })
}

fn write_tokio_std(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let file = Arc::new(std::fs::OpenOptions::new().write(true).open(path).unwrap());
        let offsets = Arc::new(offsets.to_vec());
        let content = Arc::new(content.to_vec());

        let start = Instant::now();
        for _i in 0..iter {
            let file = file.clone();
            let offsets = offsets.clone();
            let content = content.clone();

            tokio::task::spawn_blocking(move || {
                for offset in offsets.iter() {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::FileExt;
                        file.seek_write(content, offset).unwrap();
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::FileExt;
                        file.write_at(&content, *offset).unwrap();
                    }
                }
            })
            .await
            .unwrap();
        }
        start.elapsed()
    })
}

fn write_compio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let mut file = compio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                for &offset in *offsets {
                    (_, content) = file.write_at(content, offset).await.unwrap();
                }
            }
            start.elapsed()
        }
    })
}

fn write_compio_join(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            let file = compio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let res = offsets
                    .iter()
                    .map(|offset| {
                        let mut file = &file;
                        let content = content.clone();
                        async move { file.write_at(content, *offset).await.unwrap() }
                    })
                    .collect::<FuturesUnordered<_>>()
                    .collect::<Vec<_>>()
                    .await;
                black_box(res);
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn write_monoio(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = MonoioRuntime::new();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let file = monoio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                for &offset in *offsets {
                    let res;
                    (res, content) = file.write_at(content, offset).await;
                    res.unwrap();
                }
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn write_monoio_join(b: &mut Bencher, (path, offsets, content): &(&Path, &[u64], &[u8])) {
    let runtime = MonoioRuntime::new();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            let file = monoio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let res = offsets
                    .iter()
                    .map(|offset| {
                        let file = &file;
                        let content = content.clone();
                        async move {
                            let (res, content) = file.write_at(content, *offset).await;
                            res.unwrap();
                            content
                        }
                    })
                    .collect::<FuturesUnordered<_>>()
                    .collect::<Vec<_>>()
                    .await;
                black_box(res);
            }
            start.elapsed()
        }
    })
}

fn write_all_std(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let mut file = std::fs::File::create(path).unwrap();
    b.iter(|| {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut write_len = 0;
        let total_len = content.len();
        while write_len < total_len {
            let write = file
                .write(&content[write_len..(write_len + BUFFER_SIZE).min(total_len)])
                .unwrap();
            write_len += write;
        }
    })
}

fn write_all_tokio(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let mut file = tokio::fs::File::create(path).await.unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            file.seek(SeekFrom::Start(0)).await.unwrap();
            let mut write_len = 0;
            let total_len = content.len();
            while write_len < total_len {
                let write = file
                    .write(&content[write_len..(write_len + BUFFER_SIZE).min(total_len)])
                    .await
                    .unwrap();
                write_len += write;
            }
        }
        start.elapsed()
    })
}

fn write_all_tokio_std(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let content = Arc::new(content.to_vec());
        let mut file = std::fs::File::create(path).unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let content = content.clone();
            file = tokio::task::spawn_blocking(move || {
                file.seek(SeekFrom::Start(0)).unwrap();
                let mut write_len = 0;
                let total_len = content.len();
                while write_len < total_len {
                    let write = file
                        .write(&content[write_len..(write_len + BUFFER_SIZE).min(total_len)])
                        .unwrap();
                    write_len += write;
                }
                file
            })
            .await
            .unwrap();
        }
        start.elapsed()
    })
}

fn write_all_compio(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = compio::runtime::Runtime::new().unwrap();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let mut file = compio::fs::File::create(path).await.unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let mut write_len = 0;
                let total_len = content.len();
                while write_len < total_len as u64 {
                    let (write, slice) = file
                        .write_at(
                            content.slice(
                                write_len as usize
                                    ..(write_len as usize + BUFFER_SIZE).min(total_len),
                            ),
                            write_len,
                        )
                        .await
                        .unwrap();
                    write_len += write as u64;
                    content = slice.into_inner();
                }
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn write_all_monoio(b: &mut Bencher, (path, content): &(&Path, &[u8])) {
    let runtime = MonoioRuntime::new();
    let content = content.to_vec();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let file = monoio::fs::File::create(path).await.unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let mut write_len = 0;
                let total_len = content.len();
                while write_len < total_len as u64 {
                    let (write, slice) = file
                        .write_at(
                            monoio::buf::IoBuf::slice(
                                content,
                                write_len as usize
                                    ..(write_len as usize + BUFFER_SIZE).min(total_len),
                            ),
                            write_len,
                        )
                        .await;
                    write_len += write.unwrap() as u64;
                    content = slice.into_inner();
                }
            }
            start.elapsed()
        }
    })
}

fn write(c: &mut Criterion) {
    const FILE_SIZE: u64 = 1024;
    const WRITE_FILE_SIZE: u64 = 16;

    let mut rng = rng();

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
        let offset = rng.random_range(0u64..FILE_SIZE) * BUFFER_SIZE as u64;
        offsets.push(offset);
    }

    let mut content = vec![];
    for _i in 0..WRITE_FILE_SIZE {
        let mut buffer = [0u8; BUFFER_SIZE];
        rng.fill_bytes(&mut buffer);
        content.extend_from_slice(&buffer);
    }

    let mut group = c.benchmark_group("write-random");

    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "std",
        &(&path, &offsets, &single_content),
        write_std,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "tokio",
        &(&path, &offsets, &single_content),
        write_tokio,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "tokio_std",
        &(&path, &offsets, &single_content),
        write_tokio_std,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "compio",
        &(&path, &offsets, &single_content),
        write_compio,
    );
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "compio-join",
        &(&path, &offsets, &single_content),
        write_compio_join,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "monoio",
        &(&path, &offsets, &single_content),
        write_monoio,
    );
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u64], &[u8])>(
        "monoio-join",
        &(&path, &offsets, &single_content),
        write_monoio_join,
    );

    group.finish();

    let mut group = c.benchmark_group("write-all");
    group.throughput(Throughput::Bytes(content.len() as _));

    group.bench_with_input::<_, _, (&Path, &[u8])>("std", &(&path, &content), write_all_std);
    group.bench_with_input::<_, _, (&Path, &[u8])>("tokio", &(&path, &content), write_all_tokio);
    group.bench_with_input::<_, _, (&Path, &[u8])>(
        "tokio_std",
        &(&path, &content),
        write_all_tokio_std,
    );
    group.bench_with_input::<_, _, (&Path, &[u8])>("compio", &(&path, &content), write_all_compio);
    #[cfg(target_os = "linux")]
    group.bench_with_input::<_, _, (&Path, &[u8])>("monoio", &(&path, &content), write_all_monoio);

    group.finish()
}
