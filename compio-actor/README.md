<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-actor

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-actor)](https://crates.io/crates/compio-actor)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--actor-latest)](https://docs.rs/compio-actor)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

An actor framework built for [Compio](https://compio-rs). It lets you keep state and asynchronous behavior together while the framework takes care of running each actor on a single worker.

An `Actor` owns its state and lifecycle. It is created on its worker, initialized by `pre_start`, and can use the remaining lifecycle hooks to start or clean up resources. The actor, its state, and its futures stay local to that worker, so none of them need to implement `Send`.

A `Handler<M>` implementation teaches an actor how to process one message type. Implement it more than once when an actor accepts different kinds of messages; all of them are handled serially through the same bounded FIFO mailbox.

Spawning an actor gives you a `Mailbox<A>`, a typed reference that can send every message handled by `A`. A `Broker<M>` narrows that down to a cloneable, send-only capability for one message type, which is handy when other code should not need to know the actor's concrete type.

Messages can be sent in two ways:

- A **cast** sends a message with `Mailbox::send` or `Broker::send` and continues without waiting for the actor to handle it. A full or closed mailbox returns the original message in `DeliverError`.
- A **call** sends a request with `Mailbox::call` or `Broker<Call<M, R>>::call` and waits for the handler to reply. The framework creates the `Call<M, R>` value and gives its reply capability to the handler. A handler can reply directly with `Call::reply`, or split the call with `Call::into_parts` and reply later through `Reply<R>`.

A `Cluster` places actors on workers managed by `compio-dispatcher`. `Cluster::spawn` returns a lazy builder: no actor is started until the builder is awaited. Configure it with `with_name`, `with_capacity`, and `with_supervisor`. A successful spawn returns the actor's `Mailbox` and an `ActorHandle<E>` that reports `ActorExit::Stopped` or `ActorExit::Failed(E)`. Dropping the handle does not stop the actor.

Named actors can be found with `Cluster::lookup`. `Mailbox::name` and `Broker::name` return that registered name, or `None` for an unnamed actor. From a lifecycle hook or message handler, `Cluster::current` returns a clone of the cluster running that actor; `Cluster::try_current` is the non-panicking form.

Anything that crosses between the cluster and an actor—factories, messages, mailboxes, startup arguments, and errors—must implement `Send`. Calling `stop` lets the current handler finish. The lifecycle order is `pre_start`, `post_start`, message handling, `pre_stop`, then `post_stop`. Messages and worker-local handler futures are type-erased internally, so each handled message currently requires two small allocations.

## Process groups

A `ProcessGroup<M>` load-balances one message type across any actors that can produce a `Broker<M>`. `Strategy::RoundRobin` is the available routing policy and the default; it can also be selected explicitly with `ProcessGroup::with_strategy`. Routing tries each member once, skipping closed mailboxes and falling through when a mailbox is full. The group does not keep a backlog: it returns the original message when every member is full or no live member remains.

`ProcessGroup::join` returns a membership token. Keep that token for as long as the actor should receive work; dropping it removes the actor from the group. A `ProcessGroup<Call<M, R>>` can also make load-balanced calls.

## Supervision

An actor becomes a supervisor by implementing `Handler<SupervisionEvent<Child>>`. Configure a child with `.with_supervisor(&parent)` and the parent receives `ActorStarted` after `post_start` succeeds, then either `ActorTerminated` or `ActorFailed` after the child stopped. Each event contains the child's `Mailbox`, so the handler can stop it, replace it, or apply another policy.

The child's registered name is released before its terminal event is delivered. A supervisor can therefore use `Cluster::current()` to spawn a replacement under the same name. Supervision events are best-effort casts: they are dropped if the supervisor's mailbox is full or closed.

## Usage

Enable Compio's `actor` and `macros` features:

```toml
[dependencies]
compio = { version = "0.19", features = ["actor", "macros"] }
```

The following actor handles casts, calls, and graceful shutdown:

```rust,ignore
use std::{convert::Infallible, io};

use compio::actor::{Actor, ActorExit, Broker, Call, Cluster, Handler, Mailbox};

struct Counter;

#[derive(Debug)]
struct Add(usize);

#[derive(Debug)]
struct Read;

#[derive(Debug)]
struct Stop;

// An actor defines the state it owns and how that state is initialized.
impl Actor for Counter {
    type State = usize;
    type Arguments = usize;
    type Error = Infallible;

    async fn pre_start(
        &self,
        _myself: &Mailbox<Self>,
        initial: usize,
    ) -> Result<usize, Infallible> {
        Ok(initial)
    }
}

// Each Handler implementation adds one message type to the actor.
impl Handler<Add> for Counter {
    async fn handle(
        &self,
        _: &Mailbox<Self>,
        Add(value): Add,
        state: &mut usize,
    ) -> Result<(), Infallible> {
        *state += value;
        Ok(())
    }
}

impl Handler<Stop> for Counter {
    async fn handle(
        &self,
        myself: &Mailbox<Self>,
        _stop: Stop,
        state: &mut usize,
    ) -> Result<(), Infallible> {
        assert_eq!(*state, 5);
        myself.stop();
        Ok(())
    }
}

impl Handler<Call<Read, usize>> for Counter {
    async fn handle(
        &self,
        _: &Mailbox<Self>,
        call: Call<Read, usize>,
        state: &mut usize,
    ) -> Result<(), Infallible> {
        call.reply(*state).ok();
        Ok(())
    }
}

#[compio::main]
async fn main() -> io::Result<()> {
    // A cluster manages the workers that run actors.
    let cluster = Cluster::new()?;

    // Awaiting the spawn builder creates the actor and returns its mailbox.
    let (counter, handle) = cluster
        .spawn(|| Counter, 0)
        .with_name("counter")
        .await
        .unwrap();
    assert_eq!(counter.name(), Some("counter"));

    // A named actor can be looked up to get another mailbox.
    let counter = cluster.lookup::<Counter, _>("counter").unwrap();

    // A broker exposes only the ability to send one message type.
    let add: Broker<Add> = counter.broker();
    assert_eq!(add.name(), Some("counter"));

    // Casts enqueue a message and return immediately.
    add.send(Add(2)).unwrap();
    counter.send(Add(3)).unwrap();

    // Calls wait for a response from the handler.
    assert_eq!(counter.call(Read).await.unwrap(), 5);
    counter.send(Stop).unwrap();

    // The handle reports how the actor exited.
    assert_eq!(handle.await.unwrap(), ActorExit::Stopped);
    cluster.join().await
}
```
