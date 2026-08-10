use std::{convert::Infallible, num::NonZeroUsize};

use compio_actor::{
    Actor, ActorExit, ActorHandle, Call, Cluster, Handler, Mailbox, supervisor::SupervisionEvent,
};
use compio_dispatcher::Dispatcher;

fn cluster() -> Cluster {
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(2).unwrap())
        .build()
        .unwrap();
    Cluster::from_dispatcher(dispatcher)
}

#[derive(Debug)]
struct Stop;

#[derive(Debug)]
struct Fail;

#[derive(Debug)]
struct Ready;

struct Child;

impl Actor for Child {
    type Arguments = ();
    type Error = &'static str;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(())
    }
}

impl Handler<Stop> for Child {
    async fn handle(
        &self,
        myself: &Mailbox<Self>,
        _stop: Stop,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        myself.stop();
        Ok(())
    }
}

impl Handler<Fail> for Child {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Fail: Fail,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Err("child failed")
    }
}

impl Handler<Call<Ready, ()>> for Child {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<Ready, ()>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        call.reply(()).ok();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservedEvent {
    Started,
    Terminated,
    Failed,
}

#[derive(Debug)]
struct Events;

struct Parent;

impl Actor for Parent {
    type Arguments = ();
    type Error = Infallible;
    type State = Vec<ObservedEvent>;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(Vec::new())
    }
}

impl Handler<SupervisionEvent<Child>> for Parent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        event: SupervisionEvent<Child>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        state.push(match event {
            SupervisionEvent::ActorStarted(_) => ObservedEvent::Started,
            SupervisionEvent::ActorTerminated(_) => ObservedEvent::Terminated,
            SupervisionEvent::ActorFailed(_) => ObservedEvent::Failed,
        });
        Ok(())
    }
}

impl Handler<Call<Events, Vec<ObservedEvent>>> for Parent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<Events, Vec<ObservedEvent>>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        call.reply(state.clone()).ok();
        Ok(())
    }
}

#[compio_macros::test]
async fn reports_child_start_termination_and_failure() {
    let cluster = cluster();
    let (parent, parent_handle) = cluster.spawn(|| Parent, ()).await.unwrap();

    let (stopping, stopping_handle) = cluster
        .spawn(|| Child, ())
        .with_supervisor(&parent)
        .await
        .unwrap();
    stopping.send(Stop).unwrap();
    assert_eq!(stopping_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(
        parent.call(Events).await.unwrap(),
        [ObservedEvent::Started, ObservedEvent::Terminated]
    );

    let (failing, failing_handle) = cluster
        .spawn(|| Child, ())
        .with_supervisor(&parent)
        .await
        .unwrap();
    failing.send(Fail).unwrap();
    assert_eq!(
        failing_handle.await.unwrap(),
        ActorExit::Failed("child failed")
    );
    assert_eq!(
        parent.call(Events).await.unwrap(),
        [
            ObservedEvent::Started,
            ObservedEvent::Terminated,
            ObservedEvent::Started,
            ObservedEvent::Failed,
        ]
    );

    parent.stop();
    assert_eq!(parent_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}

#[compio_macros::test]
async fn stopping_a_supervisor_does_not_stop_its_children() {
    let cluster = cluster();
    let (parent, parent_handle) = cluster.spawn(|| Parent, ()).await.unwrap();
    let (child, child_handle) = cluster
        .spawn(|| Child, ())
        .with_supervisor(&parent)
        .await
        .unwrap();
    child.call(Ready).await.unwrap();

    parent.stop();
    assert_eq!(parent_handle.await.unwrap(), ActorExit::Stopped);
    assert!(!child.is_closed());

    child.stop();
    assert_eq!(child_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}

struct StoppingParent;

impl Actor for StoppingParent {
    type Arguments = ();
    type Error = Infallible;
    type State = ();

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(())
    }
}

impl Handler<SupervisionEvent<Child>> for StoppingParent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        event: SupervisionEvent<Child>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        if let SupervisionEvent::ActorStarted(child) = event {
            child.stop();
        }
        Ok(())
    }
}

#[compio_macros::test]
async fn a_supervisor_can_control_a_child_from_its_event() {
    let cluster = cluster();
    let (parent, parent_handle) = cluster.spawn(|| StoppingParent, ()).await.unwrap();
    let (_child, child_handle) = cluster
        .spawn(|| Child, ())
        .with_supervisor(&parent)
        .await
        .unwrap();

    assert_eq!(child_handle.await.unwrap(), ActorExit::Stopped);
    assert!(!parent.is_closed());
    parent.stop();
    assert_eq!(parent_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}

struct RestartingParent;

struct RestartState {
    child: Option<Mailbox<Child>>,
    handle: Option<ActorHandle<&'static str>>,
}

impl Actor for RestartingParent {
    type Arguments = ();
    type Error = Infallible;
    type State = RestartState;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(RestartState {
            child: None,
            handle: None,
        })
    }
}

impl Handler<SupervisionEvent<Child>> for RestartingParent {
    async fn handle(
        &self,
        myself: &Mailbox<Self>,
        event: SupervisionEvent<Child>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        if let SupervisionEvent::ActorFailed(child) = event {
            let (child, handle) = Cluster::current()
                .spawn(|| Child, ())
                .with_name(child.name().expect("failed child is registered").to_owned())
                .with_supervisor(myself)
                .await
                .unwrap();
            state.child = Some(child);
            state.handle = Some(handle);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Replacement;

impl Handler<Call<Replacement, Mailbox<Child>>> for RestartingParent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<Replacement, Mailbox<Child>>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        call.reply(state.child.as_ref().unwrap().clone()).ok();
        Ok(())
    }
}

#[derive(Debug)]
struct WaitForReplacement;

impl Handler<Call<WaitForReplacement, ActorExit<&'static str>>> for RestartingParent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<WaitForReplacement, ActorExit<&'static str>>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        let exit = state.handle.take().unwrap().await.unwrap();
        call.reply(exit).ok();
        Ok(())
    }
}

#[compio_macros::test]
async fn a_supervisor_can_respawn_a_failed_named_actor() {
    let cluster = cluster();
    let (parent, parent_handle) = cluster.spawn(|| RestartingParent, ()).await.unwrap();
    let (child, child_handle) = cluster
        .spawn(|| Child, ())
        .with_name("worker")
        .with_supervisor(&parent)
        .await
        .unwrap();

    child.send(Fail).unwrap();
    assert_eq!(
        child_handle.await.unwrap(),
        ActorExit::Failed("child failed")
    );

    let replacement = parent.call(Replacement).await.unwrap();
    assert_eq!(replacement.name(), Some("worker"));
    assert_eq!(
        cluster.lookup::<Child, _>("worker").unwrap().name(),
        Some("worker")
    );

    replacement.stop();
    assert_eq!(
        parent.call(WaitForReplacement).await.unwrap(),
        ActorExit::Stopped
    );
    parent.stop();
    assert_eq!(parent_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await.unwrap();
}
