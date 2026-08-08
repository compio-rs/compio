use std::{convert::Infallible, io};

use compio_actor::{Actor, ActorExit, Call, Cluster, Handler, Mailbox};

struct Counter;

#[derive(Debug)]
struct Add(usize);

#[derive(Debug)]
struct Read;

impl Actor for Counter {
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
}

impl Handler<Add> for Counter {
    async fn handle(
        &self,
        _myself: &Mailbox<Self>,
        Add(value): Add,
        state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        *state += value;
        Ok(())
    }
}

/// A [`Call`] message is one that the actor can reply to with [`Call::reply`].
impl Handler<Call<Read, usize>> for Counter {
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
    let (counter, handle) = cluster.spawn(|| Counter, 0).await.unwrap();

    counter.send(Add(3)).unwrap();
    assert_eq!(counter.call(Read).await.unwrap(), 3);

    counter.stop();
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await
}
