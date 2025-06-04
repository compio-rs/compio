use std::{
    cell::{Cell, RefCell, UnsafeCell},
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

pub struct BiLock<T>(Rc<Inner<T>>);

impl<T> Debug for BiLock<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BiLock")
            .field("locked", &self.0.locked.get())
            .finish()
    }
}

impl<T> BiLock<T> {
    pub fn new(data: T) -> (Self, Self) {
        let inner = Rc::new(Inner {
            data: UnsafeCell::new(data),
            locked: Cell::new(false),
            waiter: RefCell::new(None),
        });
        (Self(inner.clone()), Self(inner))
    }

    pub fn lock(&self) -> Waiting<'_, T> {
        Waiting { lock: &self.0 }
    }

    pub fn try_join(self, other: Self) -> Option<T> {
        if Rc::ptr_eq(&self.0, &other.0) {
            drop(other);
            let value = Rc::try_unwrap(self.0)
                .map_err(|_| ())
                .expect("BiLock is still shared")
                .data
                .into_inner();
            Some(value)
        } else {
            None
        }
    }

    pub fn join(self, other: Self) -> T {
        if let Some(value) = self.try_join(other) {
            value
        } else {
            #[cold]
            fn panic_unrelated() -> ! {
                panic!("Unrelated `BiLock` passed to `BiLock::join`.")
            }

            panic_unrelated()
        }
    }
}

pub struct Waiting<'a, T> {
    lock: &'a Inner<T>,
}

impl<'a, T> Future for Waiting<'a, T> {
    type Output = BiLockGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.lock.locked.replace(true) {
            this.lock.waiter.replace(Some(cx.waker().clone()));
            Poll::Pending
        } else {
            Poll::Ready(BiLockGuard { lock: this.lock })
        }
    }
}

struct Inner<T: ?Sized> {
    locked: Cell<bool>,
    waiter: RefCell<Option<Waker>>,
    data: UnsafeCell<T>,
}

pub struct BiLockGuard<'a, T: ?Sized> {
    lock: &'a Inner<T>,
}

impl<T: ?Sized> Deref for BiLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for BiLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for BiLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.set(false);
        if let Some(waker) = self.lock.waiter.borrow_mut().take() {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{FutureExt, task::noop_waker_ref};

    use super::*;

    #[compio_macros::test]
    async fn test_bilock() {
        let (lock1, lock2) = BiLock::new(42);
        let guard1 = lock1.lock().await;
        assert_eq!(*guard1, 42);

        {
            let mut guard2 = lock2.lock();
            let res = guard2.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
            assert!(res.is_pending())
        }

        drop(guard1);
        let mut guard2 = lock2.lock();
        let res = guard2.poll_unpin(&mut Context::from_waker(noop_waker_ref()));
        assert!(res.is_ready())
    }
}
