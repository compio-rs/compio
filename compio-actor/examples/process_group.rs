use std::{convert::Infallible, io};

use compio_actor::{
    Actor, ActorExit, Call, Cluster, Handler, Mailbox,
    process_group::{ProcessGroup, Strategy},
};

struct Worker;

#[derive(Debug)]
struct Add;
#[derive(Debug)]
struct Read;

impl Actor for Worker {
    type Arguments = ();
    type Error = Infallible;
    type State = usize;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        (): Self::Arguments,
    ) -> Result<Self::State, Self::Error> {
        Ok(0)
    }
}

impl Handler<Add> for Worker {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Add: Add,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        *state += 1;
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

#[compio::main]
async fn main() -> io::Result<()> {
    let cluster = Cluster::new()?;
    let (first, first_handle) = cluster.spawn(|| Worker, ()).await.unwrap();
    let (second, second_handle) = cluster.spawn(|| Worker, ()).await.unwrap();

    let workers = ProcessGroup::with_strategy(Strategy::RoundRobin);
    let _first = workers.join(first.broker());
    let _second = workers.join(second.broker());
    for _ in 0..4 {
        workers.send(Add).unwrap();
    }

    let readers = ProcessGroup::new();
    let _first = readers.join(first.broker::<Call<Read, usize>>());
    let _second = readers.join(second.broker::<Call<Read, usize>>());
    assert_eq!(readers.call(Read).await.unwrap(), 2);
    assert_eq!(readers.call(Read).await.unwrap(), 2);

    first.stop();
    second.stop();
    assert_eq!(first_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(second_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await
}
