// Copyright 2015-2016 rust-stm Developers
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry::*;
use std::mem;
use std::sync::Arc;
use std::any::Any;
use std::cell::Cell;

use crate::transaction::log_var::LogVar;
use crate::transaction::log_var::LogVar::*;

use crate::tvar::{TVar, VarControlBlock};
use crate::result::*;
use crate::result::StmError::*;

thread_local!(static TRANSACTION_RUNNING: Cell<bool> = Cell::new(false));

/// `TransactionGuard` checks against nested STM calls.
///
/// Use guard, so that it correctly marks the Transaction as finished.
pub struct TransactionGuard;

impl TransactionGuard {
    pub fn new() -> TransactionGuard {
        TRANSACTION_RUNNING.with(|t| {
            assert!(!t.get(), "STM: Nested Transaction");
            t.set(true);
        });
        TransactionGuard
    }
}

impl Drop for TransactionGuard {
    fn drop(&mut self) {
        TRANSACTION_RUNNING.with(|t| {
            t.set(false);
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionControl {
    Retry, Abort
}

/// Transaction tracks all the read and written variables.
///
/// It is used for checking vars, to ensure atomicity.
pub struct Transaction {

    /// Map of all vars that map the `VarControlBlock` of a var to a `LogVar`.
    /// The `VarControlBlock` is unique because it uses it's address for comparing.
    ///
    /// The logs need to be accessed in a order to prevend dead-locks on locking.
    vars: BTreeMap<Arc<VarControlBlock>, LogVar>,
}

impl Transaction {
    /// Create a new log.
    ///
    /// Normally you don't need to call this directly.
    /// Use `atomically` instead.
    pub fn new() -> Transaction {
        Transaction { vars: BTreeMap::new() }
    }

    /// Perform a downcast on a var.
    fn downcast<T: Any + Clone>(var: Arc<dyn Any>) -> T {
        match var.downcast_ref::<T>() {
            Some(s) => s.clone(),
            None    => unreachable!("TVar has wrong type")
        }
    }

    /// Read a variable and return the value.
    ///
    /// The returned value is not always consistent with the current value of the var,
    /// but may be an outdated or or not yet commited value.
    ///
    /// The used code should be capable of handling inconsistent states
    /// without running into infinite loops.
    /// Just the commit of wrong values is prevented by STM.
    pub fn read<T: Send + Sync + Any + Clone>(&mut self, var: &TVar<T>) -> StmResult<T> {
        let ctrl = var.control_block().clone();
        // Check if the same var was written before.
        let value = match self.vars.entry(ctrl) {

            // If the variable has been accessed before, then load that value.
            Occupied(mut entry) => entry.get_mut().read(),

            // Else load the variable statically.
            Vacant(entry) => {
                // Read the value from the var.
                let value = var.read_ref_atomic();

                // Store in in an entry.
                entry.insert(Read(value.clone()));
                value
            }
        };

        // For now always succeeds, but that may change later.
        Ok(Transaction::downcast(value))
    }

    /// Write a variable.
    ///
    /// The write is not immediately visible to other threads,
    /// but atomically commited at the end of the computation.
    pub fn write<T: Any + Send + Sync + Clone>(&mut self, var: &TVar<T>, value: T) -> StmResult<()> {
        // box the value
        let boxed = Arc::new(value);

        // new control block
        let ctrl = var.control_block().clone();
        // update or create new entry
        match self.vars.entry(ctrl) {
            Occupied(mut entry)     => entry.get_mut().write(boxed),
            Vacant(entry)       => { entry.insert(Write(boxed)); }
        }

        // For now always succeeds, but that may change later.
        Ok(())
    }

    /// Combine two calculations. When one blocks with `retry`,
    /// run the other, but don't commit the changes in the first.
    ///
    /// If both block, `Transaction::or` still waits for `TVar`s in both functions.
    /// Use `Transaction::or` instead of handling errors directly with the `Result::or`.
    /// The later does not handle all the blocking correctly.
    pub fn or<T, F1, F2>(&mut self, first: F1, second: F2) -> StmResult<T>
        where F1: Fn(&mut Transaction) -> StmResult<T>,
              F2: Fn(&mut Transaction) -> StmResult<T>,
    {
        // Create a backup of the log.
        let mut copy = Transaction {
            vars: self.vars.clone()
        };

        // Run the first computation.
        let f = first(self);

        match f {
            // Run other on manual retry call.
            Err(Retry)      => {
                // swap, so that self is the current run
                mem::swap(self, &mut copy);

                // Run other action.
                let s = second(self);

                // If both called retry then exit.
                match s {
                    Err(Failure)        => Err(Failure),
                    s => {
                        self.combine(copy);
                        s
                    }
                }
            }

            // Return success and failure directly
            x               => x,
        }
    }

    /// Combine two logs into a single log, to allow waiting for all reads.
    fn combine(&mut self, other: Transaction) {
        // combine reads
        for (var, value) in other.vars {
            // only insert new values
            if let Some(value) = value.obsolete() {
                self.vars.entry(var).or_insert(value);
            }
        }
    }

    /// Clear the log's data.
    ///
    /// This should be used before redoing a computation, but
    /// nowhere else.
    pub fn clear(&mut self) {
        self.vars.clear();
    }

    /// Write the log back to the variables.
    ///
    /// Return true for success and false, if a read var has changed
    pub fn commit(&mut self) -> bool {
        // Use two phase locking for safely writing data back to the vars.

        // First phase: acquire locks.
        // Check for consistency of all the reads and perform
        // an early return if something is not consistent.

        // Created arrays for storing the locks
        // vector of locks.
        let mut read_vec = Vec::with_capacity(self.vars.len());

        // vector of tuple (value, lock)
        let mut write_vec = Vec::with_capacity(self.vars.len());

        // vector of written variables
        let mut written = Vec::with_capacity(self.vars.len());


        for (var, value) in &self.vars {
            // lock the variable and read the value

            match *value {
                // We need to take a write lock.
                Write(ref w) | ReadObsoleteWrite(_,ref w)=> {
                    // take write lock
                    let lock = var.value.write();
                    // add all data to the vector
                    write_vec.push((w, lock));
                    written.push(var);
                }

                // We need to check for consistency and
                // take a write lock.
                ReadWrite(ref original,ref w) => {
                    // take write lock
                    let lock = var.value.write();

                    if !Arc::ptr_eq(&lock, original) {
                        return false;
                    }
                    // add all data to the vector
                    write_vec.push((w, lock));
                    written.push(var);
                }
                // Nothing to do. ReadObsolete is only needed for blocking, not
                // for consistency checks.
                ReadObsolete(_) => { }
                // Take read lock and check for consistency.
                Read(ref original) => {
                    // Take a read lock.
                    let lock = var.value.read();

                    if !Arc::ptr_eq(&lock, original) {
                        return false;
                    }

                    read_vec.push(lock);
                }
            }
        }

        // Second phase: write back and release

        // Release the reads first.
        // This allows other threads to continue quickly.
        drop(read_vec);

        for (value, mut lock) in write_vec {
            // Commit value.
            *lock = value.clone();
        }

        for var in written {
            // Unblock all threads waiting for it.
            var.wake_all();
        }

        // Commit succeded.
        true
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn read() {
        let mut log = Transaction::new();
        let var = TVar::new(vec![1, 2, 3, 4]);

        // The variable can be read.
        assert_eq!(&*log.read(&var).unwrap(), &[1, 2, 3, 4]);
    }

    #[test]
    fn write_read() {
        let mut log = Transaction::new();
        let var = TVar::new(vec![1, 2]);

        log.write(&var, vec![1, 2, 3, 4]).unwrap();

        // Consecutive reads get the updated version.
        assert_eq!(log.read(&var).unwrap(), [1, 2, 3, 4]);

        // The original value is still preserved.
        assert_eq!(var.read_atomic(), [1, 2]);
    }
}
