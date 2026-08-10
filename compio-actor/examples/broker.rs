use std::{convert::Infallible, io};

use compio_actor::{Actor, ActorExit, Broker, Call, Cluster, Handler, Mailbox};

#[derive(Debug)]
struct Add(usize);

#[derive(Debug)]
struct Read;

struct Counter;
struct Gauge;

impl Actor for Counter {
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

impl Actor for Gauge {
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

impl Handler<Add> for Gauge {
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

impl Handler<Call<Read, usize>> for Gauge {
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

#[compio_macros::main]
async fn main() -> io::Result<()> {
    let cluster = Cluster::new()?;
    let (counter, counter_handle) = cluster.spawn(|| Counter, ()).await.unwrap();
    let (gauge, gauge_handle) = cluster.spawn(|| Gauge, ()).await.unwrap();

    let actors: Vec<Broker<Add>> = vec![counter.broker(), gauge.broker()];
    for actor in &actors {
        actor.send(Add(2)).unwrap();
    }
    assert_eq!(counter.call(Read).await.unwrap(), 2);
    assert_eq!(gauge.call(Read).await.unwrap(), 2);

    counter.stop();
    gauge.stop();
    assert_eq!(counter_handle.await.unwrap(), ActorExit::Stopped);
    assert_eq!(gauge_handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await
}
