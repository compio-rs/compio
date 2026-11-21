use std::{
    hint::black_box,
    io,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    time::Duration,
};

use compio_driver::reactor::{Event, OsReactor, Reactor};
use criterion::{Criterion, criterion_group, criterion_main};
use polling::{Events, Poller};

criterion_group!(
    reactor,
    rearm_and_wait,
    notify_and_wait,
    rearm_and_wait_many
);
criterion_main!(reactor);

const KEY: usize = 7;

/// An eventfd that stays readable: one counter write, never read.
fn ready_fd() -> OwnedFd {
    let fd = unsafe { libc::eventfd(1, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(fd >= 0, "eventfd: {}", io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// What the driver does once per completed operation on a ready fd: re-arm
/// the one-shot interest, then wait and collect the event.
fn rearm_and_wait(c: &mut Criterion) {
    let mut group = c.benchmark_group("rearm_and_wait");
    let fd = ready_fd();

    {
        let poller = Poller::new().unwrap();
        unsafe { poller.add(fd.as_raw_fd(), Event::none(KEY)) }.unwrap();
        let mut events = Events::new();
        group.bench_function("polling::Poller", |b| {
            b.iter(|| {
                poller.modify(fd.as_fd(), Event::readable(KEY)).unwrap();
                events.clear();
                poller.wait(&mut events, Some(Duration::ZERO)).unwrap();
                black_box(events.iter().next().unwrap());
            })
        });
        poller.delete(fd.as_fd()).unwrap();
    }

    {
        let mut reactor: Box<dyn Reactor> = Box::new(OsReactor::new(1024).unwrap());
        unsafe { reactor.add(fd.as_raw_fd(), Event::none(KEY)) }.unwrap();
        let mut events = Vec::with_capacity(1024);
        group.bench_function("dyn Reactor (OsReactor)", |b| {
            b.iter(|| {
                reactor.modify(fd.as_fd(), Event::readable(KEY)).unwrap();
                events.clear();
                reactor.wait(&mut events, Some(Duration::ZERO)).unwrap();
                black_box(events[0]);
            })
        });
        reactor.delete(fd.as_fd()).unwrap();
    }

    group.finish();
}

/// A cross-thread wake: notify, then the wait it ends.
fn notify_and_wait(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_and_wait");

    {
        let poller = Poller::new().unwrap();
        let mut events = Events::new();
        group.bench_function("polling::Poller", |b| {
            b.iter(|| {
                poller.notify().unwrap();
                events.clear();
                poller.wait(&mut events, None).unwrap();
            })
        });
    }

    {
        let mut reactor: Box<dyn Reactor> = Box::new(OsReactor::new(1024).unwrap());
        let notifier = reactor.notifier();
        let mut events = Vec::with_capacity(1024);
        group.bench_function("dyn Reactor (OsReactor)", |b| {
            b.iter(|| {
                notifier.notify().unwrap();
                events.clear();
                reactor.wait(&mut events, None).unwrap();
            })
        });
    }

    group.finish();
}

const MANY: usize = 64;

/// One wait that returns many ready fds, so the copy out of the poller's own
/// event buffer is paid once per fd.
fn rearm_and_wait_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("rearm_and_wait_many");
    let fds: Vec<OwnedFd> = (0..MANY).map(|_| ready_fd()).collect();

    {
        let poller = Poller::new().unwrap();
        for (key, fd) in fds.iter().enumerate() {
            unsafe { poller.add(fd.as_raw_fd(), Event::none(key)) }.unwrap();
        }
        let mut events = Events::new();
        group.bench_function("polling::Poller", |b| {
            b.iter(|| {
                for (key, fd) in fds.iter().enumerate() {
                    poller.modify(fd.as_fd(), Event::readable(key)).unwrap();
                }
                events.clear();
                poller.wait(&mut events, Some(Duration::ZERO)).unwrap();
                assert_eq!(events.len(), MANY);
                for event in events.iter() {
                    black_box(event);
                }
            })
        });
        for fd in &fds {
            poller.delete(fd.as_fd()).unwrap();
        }
    }

    {
        let mut reactor: Box<dyn Reactor> = Box::new(OsReactor::new(1024).unwrap());
        for (key, fd) in fds.iter().enumerate() {
            unsafe { reactor.add(fd.as_raw_fd(), Event::none(key)) }.unwrap();
        }
        let mut events = Vec::with_capacity(1024);
        group.bench_function("dyn Reactor (OsReactor)", |b| {
            b.iter(|| {
                for (key, fd) in fds.iter().enumerate() {
                    reactor.modify(fd.as_fd(), Event::readable(key)).unwrap();
                }
                events.clear();
                reactor.wait(&mut events, Some(Duration::ZERO)).unwrap();
                assert_eq!(events.len(), MANY);
                for event in events.iter() {
                    black_box(event);
                }
            })
        });
        for fd in &fds {
            reactor.delete(fd.as_fd()).unwrap();
        }
    }

    group.finish();
}
