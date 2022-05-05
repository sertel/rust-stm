use crate::result::*;
use crate::transaction::tx::{Transaction, TransactionControl, TransactionGuard};
use crate::tvar::TVar;
use transaction::{Tx, TxBase};

use std::any::Any;

use std::sync::mpsc::{Receiver, Sender};

type Token = ();

/// This is an implementation of DeSTM:
/// Ravichandran, K., Gavrilovska, A. and Pande, S., 2014, August.
/// DeSTM: harnessing determinism in STMs for application development. PACT 2014
/// https://dl.acm.org/doi/pdf/10.1145/2628071.2628094
///
/// Our implementation based on channels rather than synchronized variables.
///

struct Coordination {
    tx_count: usize,
    current_tx_count: usize,
}

impl Coordination {
    fn report_started(&mut self) {
        unimplemented!()
    }

    fn await_all_started(&self) {
        unimplemented!()
    }

    fn report_done(&mut self) {
        unimplemented!()
    }

    fn await_round_done(&self) {
        unimplemented!()
    }
}

pub struct Deterministic {
    token_tx: Sender<Token>,
    token_rx: Receiver<Token>,
    tx: Transaction,
    coordination: Coordination,
}

impl Deterministic {
    pub fn new() -> Deterministic {
        unimplemented!()
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
    fn with_control<T, F, C>(&mut self, mut control: C, f: F) -> Option<T>
    where
        F: Fn(&mut Transaction) -> StmResult<T>,
        C: FnMut(StmError) -> TransactionControl,
    {
        // create a log guard for initializing and cleaning up
        // the log
        let _guard = TransactionGuard::new();

        // loop until success
        loop {
            // Constaint #1: report started
            self.coordination.report_started();

            // run the computation
            let result = f(&mut self.tx);

            // Constraint #1:
            // - I have the token and
            // - All other transactions started already
            match self.token_rx.recv() {
                Err(_) => {
                    // we failed in the execution: tear down
                    return None;
                }
                Ok(token) => {
                    self.coordination.await_all_started();

                    let decision = match result {
                        // on success exit loop
                        Ok(t) => {
                            if self.tx.commit() {
                                (TransactionControl::Abort, Some(t))
                            } else {
                                // retry
                                (TransactionControl::Retry, None)
                            }
                        }
                        Err(e) => (control(e), None),
                    };

                    // whatever happens, I need pass along the token
                    self.token_tx
                        .send(token)
                        .expect("Invariant broken: could not send token.");

                    match decision {
                        (TransactionControl::Abort, r) => return r,
                        (TransactionControl::Retry, _) => {
                            // clear log before retrying computation
                            self.tx.clear();

                            // Constaint #2: finish round before starting next
                            self.coordination.await_round_done();
                        }
                    }
                }
            }
        }
    }
}
