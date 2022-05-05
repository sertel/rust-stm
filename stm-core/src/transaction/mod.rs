// Copyright 2015-2016 rust-stm Developers
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

pub mod control_block;
pub mod log_var;
mod tx;
mod deterministic;
mod nondeterministic;

pub use self::tx::{Transaction, TransactionControl};
pub use self::deterministic::Coordination as DTM;
pub use self::deterministic::TxHandle as DTMHandle;

use std::any::Any;
use crate::tvar::TVar;
use super::result::*;

use self::deterministic::Deterministic;
use self::nondeterministic::NonDeterministic;

pub enum TxVersion {
    Deterministic(DTMHandle),
    NonDeterministic
}

pub trait TxBase {
    /// Read a variable and return the value.
    ///
    /// The returned value is not always consistent with the current value of the var,
    /// but may be an outdated or or not yet commited value.
    ///
    /// The used code should be capable of handling inconsistent states
    /// without running into infinite loops.
    /// Just the commit of wrong values is prevented by STM.
    fn read<T: Send + Sync + Any + Clone>(&mut self, var: &TVar<T>) -> StmResult<T>;

    /// Write a variable.
    ///
    /// The write is not immediately visible to other threads,
    /// but atomically commited at the end of the computation.
    fn write<T: Any + Send + Sync + Clone>(&mut self, var: &TVar<T>, value: T) -> StmResult<()>;

    /// Combine two calculations. When one blocks with `retry`,
    /// run the other, but don't commit the changes in the first.
    ///
    /// If both block, `Transaction::or` still waits for `TVar`s in both functions.
    /// Use `Transaction::or` instead of handling errors directly with the `Result::or`.
    /// The later does not handle all the blocking correctly.
    fn or<T, F1, F2>(&mut self, first: F1, second: F2) -> StmResult<T>
        where F1: Fn(&mut Transaction) -> StmResult<T>,
              F2: Fn(&mut Transaction) -> StmResult<T>;
}

pub trait Tx: TxBase {
    /// Run a function with a transaction.
    ///
    /// `with_control` takes another control function, that
    /// can steer the control flow and possible terminate early.
    ///
    /// `control` can react to counters, timeouts or external inputs.
    ///
    /// It allows the user to fall back to another strategy, like a global lock
    /// in the case of too much contention.
    ///
    /// Please not, that the transaction may still infinitely wait for changes when `retry` is
    /// called and `control` does not abort.
    /// If you need a timeout, another thread should signal this through a TVar.
    fn with_control<T, F, C>(&mut self, control: C, f: F) -> Option<T>
    where F: Fn(&mut Transaction) -> StmResult<T>,
          C: FnMut(StmError) -> TransactionControl;
}


/// Run a function with a transaction.
///
/// It is equivalent to `atomically`.
pub fn with<T, F>(v: TxVersion, f: F) -> T
where F: Fn(&mut Transaction) -> StmResult<T>,
{
    let r  =
        match v {
            TxVersion::Deterministic(handle) => 
                Deterministic::new(handle).with_control(|_| TransactionControl::Retry, f),
            TxVersion::NonDeterministic => 
                NonDeterministic::new().with_control(|_| TransactionControl::Retry, f)
        };
    match r {
        Some(t) => t,
        None    => unreachable!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn transaction_simple() {
        let x = with(TxVersion::NonDeterministic, |_| Ok(42));
        assert_eq!(x, 42);
    }

    #[test]
    fn transaction_read() {
        let read = TVar::new(42);

        let x = with(TxVersion::NonDeterministic, |trans| {
            read.read(trans)
        });

        assert_eq!(x, 42);
    }

    /// Run a transaction with a control function, that always aborts.
    /// The transaction still tries to run a single time and should successfully
    /// commit in this test.
    #[test]
    fn transaction_with_control_abort_on_single_run() {
        let read = TVar::new(42);

        let x = NonDeterministic::new()
                .with_control(|_| TransactionControl::Abort, |tx| {
                    read.read(tx)
                });

        assert_eq!(x, Some(42));
    }

    /// Run a transaction with a control function, that always aborts.
    /// The transaction retries infinitely often. The control function will abort this loop.
    #[test]
    fn transaction_with_control_abort_on_retry() {
        let x: Option<i32> = NonDeterministic::new()
            .with_control(|_| TransactionControl::Abort, |_| {
                Err(StmError::Retry)
            });

        assert_eq!(x, None);
    }


    #[test]
    fn transaction_write() {
        let write = TVar::new(42);

        with(TxVersion::NonDeterministic, |trans| {
            write.write(trans, 0)
        });

        assert_eq!(write.read_atomic(), 0);
    }

    #[test]
    fn transaction_copy() {
        let read = TVar::new(42);
        let write = TVar::new(0);

        with(TxVersion::NonDeterministic, |trans| {
            let r = read.read(trans)?;
            write.write(trans, r)
        });

        assert_eq!(write.read_atomic(), 42);
    }

    // Dat name. seriously?
    #[test]
    fn transaction_control_stuff() {
        let read = TVar::new(42);
        let write = TVar::new(0);

        with(TxVersion::NonDeterministic, |trans| {
            let r = read.read(trans)?;
            write.write(trans, r)
        });

        assert_eq!(write.read_atomic(), 42);
    }

    /// Test if nested transactions are correctly detected.
    #[test]
    #[should_panic]
    fn transaction_nested_fail() {
        with(TxVersion::NonDeterministic, |_| {
            with(TxVersion::NonDeterministic, |_| Ok(42));
            Ok(1)
        });
    }
}
