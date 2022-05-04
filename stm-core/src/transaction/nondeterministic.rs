use transaction::{TxBase, Tx};
use crate::transaction::tx::{TransactionGuard, TransactionControl, Transaction};
use crate::result::*;
use crate::tvar::TVar;

use std::any::Any;

pub struct NonDeterministic{
    tx: Transaction
}

impl NonDeterministic {
    pub fn new() -> NonDeterministic {
        NonDeterministic { tx: Transaction::new() }
    }
}

impl TxBase for NonDeterministic {
   fn read<T: Send + Sync + Any + Clone>(&mut self, var: &TVar<T>) -> StmResult<T>
   {
       self.tx.read(var)
   }

    fn write<T: Any + Send + Sync + Clone>(&mut self, var: &TVar<T>, value: T) -> StmResult<()>
    {
        self.tx.write(var, value)
    }

    fn or<T, F1, F2>(&mut self, first: F1, second: F2) -> StmResult<T>
        where F1: Fn(&mut Transaction) -> StmResult<T>,
              F2: Fn(&mut Transaction) -> StmResult<T>
    {
        self.tx.or(first, second)
    }
}

impl Tx for NonDeterministic {
   fn with_control<T, F, C>(&mut self, mut control: C, f: F) -> Option<T>
    where F: Fn(&mut Transaction) -> StmResult<T>,
          C: FnMut(StmError) -> TransactionControl,
    {
        // create a log guard for initializing and cleaning up
        // the log
        let _guard = TransactionGuard::new();

       //let mut transaction = Transaction::new();

        // loop until success
        loop {
            // run the computation
            match f(&mut self.tx) {
                // on success exit loop
                Ok(t) => {
                    if self.tx.commit() {
                        return Some(t);
                    }
                }

                Err(e) => {
                    // Check if the user wants to abort the transaction.
                    if let TransactionControl::Abort = control(e) {
                        return None;
                    }
                }
            }

            // clear log before retrying computation
            self.tx.clear();
        }
    }
}

 
