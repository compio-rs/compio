use std::{
    io,
    ops::Deref,
    task::{Context, Poll},
    time::Duration,
};

use compio_log::error;
use compio_runtime::Runtime;
use mod_use::mod_use;

mod_use![sys];

pub struct RuntimeCompat<A> {
    runtime: A,
}

impl<A: sys::Adapter> RuntimeCompat<A> {
    pub fn new(runtime: Runtime) -> io::Result<Self> {
        let runtime = A::new(runtime)?;
        Ok(Self { runtime })
    }

    pub async fn execute<F: Future>(&self, f: F) -> F::Output {
        let waker = self.runtime.waker();
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(f);
        loop {
            if let Poll::Ready(result) = self.runtime.enter(|| future.as_mut().poll(&mut context)) {
                self.runtime.enter(|| self.runtime.run());
                return result;
            }

            let mut remaining_tasks = self.runtime.enter(|| self.runtime.run());

            remaining_tasks |= self.runtime.flush();

            let timeout = if remaining_tasks {
                Some(Duration::ZERO)
            } else {
                self.runtime.current_timeout()
            };

            match self.runtime.wait(timeout).await {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => panic!("failed to wait for driver: {e:?}"),
            }

            if let Err(_e) = self.runtime.clear() {
                error!("failed to clear notifier: {_e:?}");
            }

            self.runtime.poll_with(Some(Duration::ZERO));
        }
    }
}

impl<A: sys::Adapter> Deref for RuntimeCompat<A> {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
