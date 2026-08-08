use std::{
    convert::Infallible,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use compio_actor::{
    Actor, ActorExit, Call, Cluster, Handler, Mailbox,
    mailbox::DeliverError,
    process_group::{ProcessGroup, Strategy},
};
use compio_dispatcher::Dispatcher;
use futures_channel::oneshot;

fn cluster() -> Cluster {
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(2).unwrap())
        .build()
        .unwrap();
    Cluster::from_dispatcher(dispatcher)
}

#[derive(Debug)]
struct Work(usize);

#[derive(Debug)]
struct Read;

struct Worker {
    observed: Arc<AtomicUsize>,
}

impl Actor for Worker {
    type Arguments = usize;
    type Error = Infallible;
    type State = usize;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        initial: Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(initial)
    }

    async fn post_stop(
        &self,
        _myself: &Mailbox<Self>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.observed.store(*state, Ordering::Relaxed);
        Ok(())
    }
}

impl Handler<Work> for Worker {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Work(value): Work,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        *state += value;
        Ok(())
    }
}

impl Handler<Call<Read, usize>> for Worker {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<Read, usize>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        call.reply(*state).ok();
        Ok(())
    }
}

#[compio_macros::test]
async fn balances_casts_and_calls_round_robin() {
    let cluster = cluster();
    let first_observed = Arc::new(AtomicUsize::new(0));
    let second_observed = Arc::new(AtomicUsize::new(0));
    let (first, first_handle) = cluster
        .spawn(
            {
                let observed = first_observed.clone();
                move || Worker { observed }
            },
            10,
        )
        .await
        .unwrap();
    let (second, second_handle) = cluster
        .spawn(
            {
                let observed = second_observed.clone();
                move || Worker { observed }
            },
            20,
        )
        .await
        .unwrap();

    let work = ProcessGroup::new();
    let first_work = work.join(first.broker());
    let _second_work = work.join(second.broker());
    for _ in 0..4 {
        work.send(Work(1)).unwrap();
    }

    let reads = ProcessGroup::new();
    let first_read = reads.join(first.broker::<Call<Read, usize>>());
    let _second_read = reads.join(second.broker::<Call<Read, usize>>());
    assert_eq!(reads.call(Read).await.unwrap(), 12);
    assert_eq!(reads.call(Read).await.unwrap(), 22);

    let explicit = ProcessGroup::with_strategy(Strategy::RoundRobin);
    let _first_explicit = explicit.join(first.broker::<Call<Read, usize>>());
    let _second_explicit = explicit.join(second.broker::<Call<Read, usize>>());
    assert_eq!(explicit.call(Read).await.unwrap(), 12);

    first_work.leave();
    first_read.leave();
    assert_eq!(work.len(), 1);
    assert_eq!(reads.call(Read).await.unwrap(), 22);

    first.stop();
    second.stop();
    assert_eq!(first_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(second_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(first_observed.load(Ordering::Relaxed), 12);
    assert_eq!(second_observed.load(Ordering::Relaxed), 22);

    let error = work.send(Work(1)).unwrap_err();
    assert!(matches!(error, DeliverError::Closed(Work(1))));
    assert!(work.is_empty());
    cluster.join().await.unwrap();
}

#[derive(Debug)]
struct Block;

struct BlockedWorker;

impl Actor for BlockedWorker {
    type Arguments = Self::State;
    type Error = Infallible;
    type State = (mpsc::Sender<()>, Option<oneshot::Receiver<()>>);

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        arguments: Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(arguments)
    }
}

impl Handler<Block> for BlockedWorker {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Block: Block,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        state.0.send(()).unwrap();
        state.1.take().unwrap().await.ok();
        Ok(())
    }
}

impl Handler<Work> for BlockedWorker {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Work(_): Work,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[compio_macros::test]
async fn skips_full_members_without_allocating_a_backlog() {
    let cluster = cluster();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let (blocked, blocked_handle) = cluster
        .spawn(|| BlockedWorker, (entered_tx, Some(release_rx)))
        .with_capacity(NonZeroUsize::new(1).unwrap())
        .await
        .unwrap();
    let observed = Arc::new(AtomicUsize::new(0));
    let (available, available_handle) = cluster
        .spawn(
            {
                let observed = observed.clone();
                move || Worker { observed }
            },
            0,
        )
        .await
        .unwrap();

    blocked.send(Block).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    blocked.send(Work(1)).unwrap();

    let group = ProcessGroup::new();
    let _blocked = group.join(blocked.broker());
    let _available = group.join(available.broker());
    group.send(Work(7)).unwrap();
    assert_eq!(available.call(Read).await.unwrap(), 7);

    available.stop();
    blocked.stop();
    release_tx.send(()).ok();
    assert_eq!(available_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(blocked_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(observed.load(Ordering::Relaxed), 7);
    cluster.join().await.unwrap();
}
