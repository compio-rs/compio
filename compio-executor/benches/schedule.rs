//! Benchmarks for the executor scheduling hot paths.
//!
//! The key path exercised here is the *local* wake: a task that wakes itself
//! on the same thread as the executor. Every such wake goes through
//! `Local::schedule`, which piggyback-drains the cross-thread "sync" queue. On
//! a single-threaded workload that queue is always empty, so this benchmark
//! measures the cost of the empty-drain fast path (the subject of issue #852).

use std::{
    future::Future,
    hint::black_box,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
    thread,
};

use compio_executor::Executor;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// A future that re-wakes itself `n` times before completing.
///
/// Each self-wake schedules the task again through the local path, so driving
/// it to completion performs exactly `n` local schedules.
struct SelfWake {
    remaining: usize,
}

impl Future for SelfWake {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.remaining == 0 {
            Poll::Ready(())
        } else {
            self.remaining -= 1;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// A future that is woken `n` times from another thread, exercising the remote
/// (cross-thread) schedule path and the piggyback drain on the consumer side.
struct RemoteWake {
    remaining: usize,
}

impl Future for RemoteWake {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.remaining == 0 {
            Poll::Ready(())
        } else {
            self.remaining -= 1;
            let waker = cx.waker().clone();
            thread::spawn(move || waker.wake());
            Poll::Pending
        }
    }
}

/// Drive a same-thread future to completion, ticking as fast as possible.
fn drive_local<F: Future + 'static>(exe: &Executor, fut: F) {
    let handle = exe.spawn(fut);
    let mut handle = pin!(handle);
    let cx = &mut Context::from_waker(Waker::noop());
    while handle.as_mut().poll(cx).is_pending() {
        exe.tick();
    }
}

/// Drive a future woken from other threads to completion, yielding while we
/// wait on the waking thread.
fn drive_remote<F: Future + 'static>(exe: &Executor, fut: F) {
    let handle = exe.spawn(fut);
    let mut handle = pin!(handle);
    let cx = &mut Context::from_waker(Waker::noop());
    while handle.as_mut().poll(cx).is_pending() {
        exe.tick();
        thread::yield_now();
    }
}

fn bench_local(c: &mut Criterion) {
    let exe = Executor::new();
    let mut group = c.benchmark_group("local_wake");
    for n in [1usize, 100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                drive_local(
                    &exe,
                    SelfWake {
                        remaining: black_box(n),
                    },
                )
            });
        });
    }
    group.finish();
}

fn bench_spawn(c: &mut Criterion) {
    let exe = Executor::new();
    let mut group = c.benchmark_group("spawn_ready");
    // Baseline: spawn a batch of tasks that complete on first poll, no wakes.
    // Isolates the per-tick sync-queue drain overhead from the wake overhead.
    for n in [1usize, 100, 1_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                for _ in 0..n {
                    exe.spawn(async { black_box(()) }).detach();
                }
                while exe.tick() {}
            });
        });
    }
    group.finish();
}

fn bench_remote(c: &mut Criterion) {
    let exe = Executor::new();
    let mut group = c.benchmark_group("remote_wake");
    // Fewer iterations: cross-thread wakes are inherently noisier.
    for n in [1usize, 10, 100] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                drive_remote(
                    &exe,
                    RemoteWake {
                        remaining: black_box(n),
                    },
                )
            });
        });
    }
    group.finish();
}

criterion_group!(schedule, bench_local, bench_spawn, bench_remote);
criterion_main!(schedule);
