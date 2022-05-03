use crate::result::*;
use crate::transaction::tx::{Transaction, TransactionControl};
use crate::tvar::TVar;
use transaction::{Tx, TxBase};

use std::any::Any;

pub struct Deterministic {
    tx: Transaction,
}

impl Deterministic {
    pub fn new() -> Deterministic {
        Deterministic {
            tx: Transaction::new(),
        }
    }
}

impl TxBase for Deterministic {
    fn read<T: Send + Sync + Any + Clone>(&mut self, var: &TVar<T>) -> StmResult<T> {
        self.tx.read(var)
    }

    fn write<T: Any + Send + Sync + Clone>(&mut self, var: &TVar<T>, value: T) -> StmResult<()> {
        self.tx.write(var, value)
    }

    fn or<T, F1, F2>(&mut self, first: F1, second: F2) -> StmResult<T>
    where
        F1: Fn(&mut Transaction) -> StmResult<T>,
        F2: Fn(&mut Transaction) -> StmResult<T>,
    {
        self.tx.or(first, second)
    }
}

impl Tx for Deterministic {
    fn with_control<T, F, C>(&mut self, _control: C, _f: F) -> Option<T>
    where
        F: Fn(&mut Transaction) -> StmResult<T>,
        C: FnMut(StmError) -> TransactionControl,
    {
        unimplemented!()
    }
}
