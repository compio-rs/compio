use std::{
    cell::Cell,
    convert::Infallible,
    num::NonZeroUsize,
    rc::Rc,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use compio_actor::{
    Actor, ActorExit, Broker, Call, Cluster, Handler,
    cluster::SpawnError,
    mailbox::{CallError, DeliverError},
};
use compio_dispatcher::Dispatcher;

fn cluster() -> Cluster {
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(1).unwrap())
        .build()
        .unwrap();
    Cluster::from_dispatcher(dispatcher)
}

fn assert_send<T: Send>(_: &T) {}

fn assert_send_sync<T: Send + Sync>(_: &T) {}

#[derive(Debug)]
struct Add(usize);

#[derive(Debug)]
struct Multiply(usize);

#[derive(Debug)]
struct Finish;

#[derive(Debug, PartialEq, Eq)]
struct Read;

#[derive(Debug, PartialEq, Eq)]
struct Ignore;

#[derive(Debug, PartialEq, Eq)]
struct Echo(String);

struct Counter {
    local: Rc<Cell<usize>>,
    observed: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    lifecycle: Arc<Mutex<Vec<&'static str>>>,
}

impl Actor for Counter {
    type Arguments = usize;
    type Error = Infallible;
    type State = Rc<Cell<usize>>;

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        initial: Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        self.lifecycle.lock().unwrap().push("pre_start");
        self.local.set(initial);
        Ok(self.local.clone())
    }

    async fn post_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("post_start");
        Ok(())
    }

    async fn pre_stop(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("pre_stop");
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("post_stop");
        self.stopped.store(true, Ordering::Relaxed);
        Ok(())
    }
}

impl Handler<Add> for Counter {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Add(value): Add,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        state.set(state.get() + value);
        self.observed.store(state.get(), Ordering::Relaxed);
        Ok(())
    }
}

impl Handler<Multiply> for Counter {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Multiply(value): Multiply,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        state.set(state.get() * value);
        self.observed.store(state.get(), Ordering::Relaxed);
        Ok(())
    }
}

impl Handler<Finish> for Counter {
    async fn handle(
        &self,
        myself: &compio_actor::Mailbox<Self>,
        Finish: Finish,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        myself.stop();
        Ok(())
    }
}

impl Handler<Call<Read, usize>> for Counter {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        reply: Call<Read, usize>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        assert_eq!(reply.message(), &Read);
        reply.reply(state.get()).ok();
        Ok(())
    }
}

impl Handler<Call<Ignore, usize>> for Counter {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _reply: Call<Ignore, usize>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Handler<Call<Echo, String>> for Counter {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        call: Call<Echo, String>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        let (Echo(message), port) = call.into_parts();
        port.reply(message).ok();
        Ok(())
    }
}

struct Gauge {
    observed: Arc<AtomicUsize>,
}

impl Actor for Gauge {
    type Arguments = ();
    type Error = Infallible;
    type State = usize;

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(0)
    }
}

impl Handler<Add> for Gauge {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Add(value): Add,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        *state += value;
        self.observed.store(*state, Ordering::Relaxed);
        Ok(())
    }
}

impl Handler<Finish> for Gauge {
    async fn handle(
        &self,
        myself: &compio_actor::Mailbox<Self>,
        Finish: Finish,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        myself.stop();
        Ok(())
    }
}

#[compio::test]
async fn handles_multiple_types_and_brokers_erase_actor_types() {
    let cluster = cluster();
    let counter_observed = Arc::new(AtomicUsize::new(0));
    let counter_stopped = Arc::new(AtomicBool::new(false));
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let actor_observed = counter_observed.clone();
    let actor_stopped = counter_stopped.clone();
    let actor_lifecycle = lifecycle.clone();
    let (counter_ref, counter_handle) = cluster
        .spawn(
            move || Counter {
                local: Rc::new(Cell::new(0)),
                observed: actor_observed,
                stopped: actor_stopped,
                lifecycle: actor_lifecycle,
            },
            1,
        )
        .await
        .unwrap();

    let gauge_observed = Arc::new(AtomicUsize::new(0));
    let actor_observed = gauge_observed.clone();
    let (gauge_ref, gauge_handle) = cluster
        .spawn(
            move || Gauge {
                observed: actor_observed,
            },
            (),
        )
        .await
        .unwrap();

    assert_send_sync(&counter_ref);
    assert_send(&counter_handle);
    let brokers: Vec<Broker<Add>> = vec![counter_ref.broker(), gauge_ref.broker()];
    assert_send_sync(&brokers[0]);
    assert_eq!(counter_ref.name(), None);
    assert_eq!(brokers[0].name(), None);
    for broker in &brokers {
        broker.send(Add(2)).unwrap();
    }
    counter_ref.send(Multiply(3)).unwrap();
    counter_ref.send(Finish).unwrap();
    gauge_ref.send(Finish).unwrap();

    assert_eq!(counter_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(gauge_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(counter_observed.load(Ordering::Relaxed), 9);
    assert_eq!(gauge_observed.load(Ordering::Relaxed), 2);
    assert!(counter_stopped.load(Ordering::Relaxed));
    assert_eq!(
        *lifecycle.lock().unwrap(),
        ["pre_start", "post_start", "pre_stop", "post_stop"]
    );
    assert!(counter_ref.is_closed());
    cluster.join().await.unwrap();
}

#[compio::test]
async fn mailbox_and_broker_can_call_an_actor() {
    let cluster = cluster();
    let observed = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let (mailbox, handle) = cluster
        .spawn(
            move || Counter {
                local: Rc::new(Cell::new(0)),
                observed,
                stopped,
                lifecycle,
            },
            7,
        )
        .await
        .unwrap();

    assert_eq!(mailbox.call(Read).await.unwrap(), 7);
    mailbox.send(Add(5)).unwrap();

    let broker: Broker<Call<Read, usize>> = mailbox.broker();
    assert_eq!(broker.call(Read).await.unwrap(), 12);
    assert_eq!(
        mailbox.call(Echo(String::from("owned"))).await.unwrap(),
        "owned"
    );
    assert_eq!(mailbox.call(Ignore).await, Err(CallError::NoReply));

    mailbox.stop();
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(mailbox.call(Read).await, Err(CallError::Closed(Read)));
    cluster.join().await.unwrap();
}

#[derive(Debug)]
struct Block;

#[derive(Debug)]
struct Queued(usize);

struct Gate;

impl Actor for Gate {
    type Arguments = (mpsc::Sender<()>, Arc<Barrier>);
    type Error = Infallible;
    type State = Self::Arguments;

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        arguments: Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(arguments)
    }
}

impl Handler<Block> for Gate {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Block: Block,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        state.0.send(()).unwrap();
        state.1.wait();
        Ok(())
    }
}

impl Handler<Queued> for Gate {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Queued(value): Queued,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        unreachable!("queued message {value} was handled")
    }
}

#[compio::test]
async fn stop_bypasses_a_full_erased_mailbox_and_broker_recovers_messages() {
    let cluster = cluster();
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let (actor_ref, handle) = cluster
        .spawn(|| Gate, (entered_tx, release.clone()))
        .with_capacity(NonZeroUsize::new(1).unwrap())
        .await
        .unwrap();
    let broker = actor_ref.broker::<Queued>();

    actor_ref.send(Block).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    actor_ref.send(Queued(1)).unwrap();
    let error = broker.send(Queued(2)).unwrap_err();
    assert!(matches!(error, DeliverError::Full(Queued(2))));

    assert!(actor_ref.stop());
    assert!(!actor_ref.stop());
    release.wait();
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);

    let error = broker.send(Queued(3)).unwrap_err();
    assert!(matches!(error, DeliverError::Closed(Queued(3))));
    cluster.join().await.unwrap();
}

#[derive(Debug)]
struct Fail;

struct Fails {
    post_stop_called: Arc<AtomicBool>,
}

impl Actor for Fails {
    type Arguments = ();
    type Error = &'static str;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.post_stop_called.store(true, Ordering::Relaxed);
        Err("post-stop failed")
    }
}

impl Handler<Fail> for Fails {
    async fn handle(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        Fail: Fail,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Err("handler failed")
    }
}

#[compio::test]
async fn reports_handler_failure_across_the_cluster() {
    let cluster = cluster();
    let post_stop_called = Arc::new(AtomicBool::new(false));
    let actor_post_stop_called = post_stop_called.clone();
    let (actor_ref, handle) = cluster
        .spawn(
            move || Fails {
                post_stop_called: actor_post_stop_called,
            },
            (),
        )
        .await
        .unwrap();

    actor_ref.send(Fail).unwrap();
    assert_eq!(handle.await.unwrap(), ActorExit::Failed("handler failed"));
    assert!(post_stop_called.load(Ordering::Relaxed));
    cluster.join().await.unwrap();
}

struct PostStartFails {
    lifecycle: Arc<Mutex<Vec<&'static str>>>,
}

impl Actor for PostStartFails {
    type Arguments = ();
    type Error = &'static str;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        self.lifecycle.lock().unwrap().push("pre_start");
        Ok(())
    }

    async fn post_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("post_start");
        Err("post-start failed")
    }

    async fn pre_stop(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("pre_stop");
        Err("pre-stop failed")
    }

    async fn post_stop(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        self.lifecycle.lock().unwrap().push("post_stop");
        Err("post-stop failed")
    }
}

#[compio::test]
async fn post_start_failure_runs_stop_hooks_and_preserves_the_first_error() {
    let cluster = cluster();
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let actor_lifecycle = lifecycle.clone();
    let (_actor_ref, handle) = cluster
        .spawn(
            move || PostStartFails {
                lifecycle: actor_lifecycle,
            },
            (),
        )
        .await
        .unwrap();

    assert_eq!(
        handle.await.unwrap(),
        ActorExit::Failed("post-start failed")
    );
    assert_eq!(
        *lifecycle.lock().unwrap(),
        ["pre_start", "post_start", "pre_stop", "post_stop"]
    );
    cluster.join().await.unwrap();
}

struct StartFails;

impl Actor for StartFails {
    type Arguments = ();
    type Error = &'static str;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Err("start failed")
    }
}

#[compio::test]
async fn reports_start_failure_from_the_worker() {
    let cluster = cluster();
    let result = cluster.spawn(|| StartFails, ()).await;
    assert!(matches!(result, Err(SpawnError::Start("start failed"))));
    cluster.join().await.unwrap();
}

#[compio::test]
async fn named_actors_can_be_looked_up_and_names_are_released_on_exit() {
    let cluster = cluster();
    let name = String::from("primary-gauge");
    let observed = Arc::new(AtomicUsize::new(0));
    let actor_observed = observed.clone();
    let (mailbox, handle) = cluster
        .spawn(
            move || Gauge {
                observed: actor_observed,
            },
            (),
        )
        .with_name(name.clone())
        .with_capacity(NonZeroUsize::new(8).unwrap())
        .await
        .unwrap();
    assert_eq!(mailbox.capacity(), NonZeroUsize::new(8).unwrap());
    assert_eq!(mailbox.name(), Some(name.as_str()));
    assert_eq!(mailbox.broker::<Finish>().name(), Some(name.as_str()));

    let registered = cluster.lookup::<Gauge, _>(name.clone()).unwrap();
    assert_eq!(registered.name(), Some(name.as_str()));
    assert!(cluster.lookup::<Counter, _>(name.clone()).is_none());

    let duplicate = cluster
        .spawn(
            || Gauge {
                observed: Arc::new(AtomicUsize::new(0)),
            },
            (),
        )
        .with_name(name.clone())
        .await;
    assert!(matches!(
        duplicate,
        Err(SpawnError::NameTaken(taken)) if taken == name
    ));

    assert!(registered.stop());
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    assert!(mailbox.is_closed());
    assert!(cluster.lookup::<Gauge, _>(name.clone()).is_none());

    let (replacement, replacement_handle) = cluster
        .spawn(
            || Gauge {
                observed: Arc::new(AtomicUsize::new(0)),
            },
            (),
        )
        .with_name(name)
        .await
        .unwrap();
    assert_eq!(replacement.name(), Some("primary-gauge"));
    replacement.stop();
    assert_eq!(replacement_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}

#[compio::test]
async fn failed_named_actor_start_releases_its_name() {
    let cluster = cluster();
    let result = cluster.spawn(|| StartFails, ()).with_name("fails").await;
    assert!(matches!(result, Err(SpawnError::Start("start failed"))));
    assert!(cluster.lookup::<StartFails, _>("fails").is_none());
    cluster.join().await.unwrap();
}

#[compio::test]
async fn dropped_spawn_builder_does_not_start_or_reserve_an_actor() {
    let cluster = cluster();
    let spawn = cluster
        .spawn(|| -> Gauge { panic!("dropped builder started") }, ())
        .with_name("lazy");
    drop(spawn);
    assert!(cluster.lookup::<Gauge, _>("lazy").is_none());

    let (mailbox, handle) = cluster
        .spawn(
            || Gauge {
                observed: Arc::new(AtomicUsize::new(0)),
            },
            (),
        )
        .with_name("lazy")
        .await
        .unwrap();
    mailbox.stop();
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}

struct SlowStart;

impl Actor for SlowStart {
    type Arguments = (mpsc::Sender<()>, Arc<Barrier>);
    type Error = Infallible;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &compio_actor::Mailbox<Self>,
        (entered, release): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        entered.send(()).unwrap();
        release.wait();
        Ok(())
    }
}

#[compio::test]
async fn named_actor_is_hidden_until_startup_succeeds() {
    let cluster = Arc::new(cluster());
    let lookup_cluster = cluster.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let lookup_release = release.clone();
    let lookup = std::thread::spawn(move || {
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(lookup_cluster.lookup::<SlowStart, _>("starting").is_none());
        lookup_release.wait();
    });

    let (mailbox, handle) = cluster
        .spawn(|| SlowStart, (entered_tx, release))
        .with_name("starting")
        .await
        .unwrap();
    lookup.join().unwrap();
    assert!(cluster.lookup::<SlowStart, _>("starting").is_some());

    mailbox.stop();
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    Arc::try_unwrap(cluster).ok().unwrap().join().await.unwrap();
}
