//! The transaction gate: at most one `Tx` is open at a time.
//!
//! A `Tx` here buffers its writes and applies them in one `Sql::atomically` at commit, but `allocate` reads the counter
//! when it is called, and the counter is written by the commit. Two transactions open together would read the same
//! counter. Inside a Durable Object that cannot happen as long as nothing between `begin` and `commit` yields to the
//! event loop, but that is a property of the use-case, not of the type; natively, tokio threads run requests in
//! parallel and it is false at once. The gate makes the property the adapter's own: `begin` waits for it, the `Tx`
//! holds it until it commits, rolls back or is dropped, so the read and the write of the counter cannot interleave
//! with another transaction's, on either twin.
//!
//! Runtime-agnostic (std only): the object runs this on workerd's single-threaded executor and the twin on tokio.
//! Cancel-safe: a `begin` that is dropped while waiting holds nothing. Waking is not FIFO (every waiter is woken on
//! release), which is fine for the few holders a request mix produces.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

#[derive(Default)]
struct State {
    held: bool,
    waiters: Vec<Waker>,
}

#[derive(Default)]
pub struct Gate {
    state: Mutex<State>,
}

impl Gate {
    pub fn acquire(self: &Arc<Self>) -> Acquire {
        Acquire { gate: self.clone() }
    }
}

pub struct Acquire {
    gate: Arc<Gate>,
}

impl Future for Acquire {
    type Output = Permit;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Permit> {
        // MUTANT (control run only, feature `mutant_no_gate`): transactions are not serialised. Two `allocate`s then read
        // the same counter, and the 50-writer test must catch it.
        #[cfg(feature = "mutant_no_gate")]
        return Poll::Ready(Permit { gate: self.gate.clone() });
        #[allow(unreachable_code)]
        let mut s = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        if !s.held {
            s.held = true;
            return Poll::Ready(Permit { gate: self.gate.clone() });
        }
        if !s.waiters.iter().any(|w| w.will_wake(cx.waker())) {
            s.waiters.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// Held by a `Tx` for as long as it is open. Dropping it opens the gate.
pub struct Permit {
    gate: Arc<Gate>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        // MUTANT (control run only, feature `mutant_gate_leak`): an abandoned transaction never opens the gate again.
        #[cfg(feature = "mutant_gate_leak")]
        {
            return;
        }
        #[cfg(not(feature = "mutant_gate_leak"))]
        {
            let waiters = {
                let mut s = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
                s.held = false;
                std::mem::take(&mut s.waiters)
            };
            for w in waiters {
                w.wake();
            }
        }
    }
}
