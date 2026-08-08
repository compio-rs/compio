use std::{convert::Infallible, io};

use compio_actor::{
    Actor, ActorExit, Call, Cluster, Handler, Mailbox, supervisor::SupervisionEvent,
};

struct Worker;
struct Parent;

#[derive(Debug)]
struct Events;

impl Actor for Worker {
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

impl Actor for Parent {
    type Arguments = ();
    type Error = Infallible;
    type State = Vec<&'static str>;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(Vec::new())
    }
}

impl Handler<SupervisionEvent<Worker>> for Parent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        event: SupervisionEvent<Worker>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        match event {
            SupervisionEvent::ActorStarted(worker) => {
                state.push("started");
                worker.stop();
            }
            SupervisionEvent::ActorTerminated(_) => state.push("terminated"),
            SupervisionEvent::ActorFailed(_) => state.push("failed"),
        }
        Ok(())
    }
}

impl Handler<Call<Events, Vec<&'static str>>> for Parent {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        call: Call<Events, Vec<&'static str>>,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        call.reply(state.clone()).ok();
        Ok(())
    }
}

#[compio::main]
async fn main() -> io::Result<()> {
    let cluster = Cluster::new()?;
    let (parent, parent_handle) = cluster.spawn(|| Parent, ()).await.unwrap();
    let (_worker, worker_handle) = cluster
        .spawn(|| Worker, ())
        .with_supervisor(&parent)
        .await
        .unwrap();

    assert_eq!(worker_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(
        parent.call(Events).await.unwrap(),
        ["started", "terminated"]
    );

    parent.stop();
    assert_eq!(parent_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await
}
