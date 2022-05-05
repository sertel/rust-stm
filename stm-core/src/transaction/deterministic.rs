use crate::result::*;
use crate::transaction::tx::{Transaction, TransactionControl, TransactionGuard};
use crate::tvar::TVar;
use transaction::{Tx, TxBase};

use std::any::Any;

use std::sync::mpsc::{channel, Receiver, Sender};

type Token = ();
enum Done {
    Retry,
    Completed,
}

/// This is an implementation of DeSTM:
/// Ravichandran, K., Gavrilovska, A. and Pande, S., 2014, August.
/// DeSTM: harnessing determinism in STMs for application development. PACT 2014
/// https://dl.acm.org/doi/pdf/10.1145/2628071.2628094
///
/// Our implementation is based on channels rather than synchronized variables.
///

struct TxCoordinationState {
    done_rx: Receiver<Done>,
    coordination_tx: Sender<(Receiver<Token>, Sender<Token>)>,
}

pub struct TxHandle {
    done_tx: Sender<Done>,
    coordination_rx: Receiver<(Receiver<Token>, Sender<Token>)>,
}

struct Coordination {
    /// This list essentially defines the order of the transactions.
    txs: Vec<TxCoordinationState>,
}

impl Coordination {
    pub fn new() -> Coordination {
        Coordination { txs: Vec::new() }
    }

    pub fn register(&mut self) -> TxHandle {
        let (done_tx, done_rx) = channel();
        let (coordination_tx, coordination_rx) = channel();
        self.txs.push(TxCoordinationState {
            done_rx,
            coordination_tx,
        });
        TxHandle {
            done_tx,
            coordination_rx,
        }
    }

    fn assign_channels(&self) -> Receiver<Token> {
        let (first_tx, mut prev_rx) = channel();
        let (next_tx, last_rx) = channel();

        first_tx
            .send(())
            .expect("Invariant broken: first send failed.");
        let (last, elements) = self
            .txs
            .split_last()
            .expect("Invariant broken: no transactions.");
        for e in elements {
            let (n_tx, n_rx) = channel();
            e.coordination_tx
                .send((prev_rx, n_tx))
                .expect("Invariant broken: could not dispatch coordination");
            prev_rx = n_rx;
        }

        last_rx
    }

    pub fn coordinate(&mut self) {
        while self.txs.len() > 0 {
            self.assign_channels();

            // retrieve all results
            let mut retries = Vec::new();
            // note: this loop preserves the order of the transactions!
            for tx in self.txs.drain(..) {
                let done = tx
                    .done_rx
                    .recv()
                    .expect("Invariant broken: coordinator could not receive done signal.");
                match done {
                    Done::Completed => (),
                    Done::Retry => retries.push(tx),
                }
            }

            // reassign channels
            self.txs = retries;
        }
    }
}

pub struct Deterministic {
    handle: TxHandle,
    tx: Transaction,
}

impl Deterministic {
    pub fn new(handle: TxHandle) -> Deterministic {
        Deterministic {
            handle,
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
            // Constraint #2: finish prev round before starting the next
            let (token_rx, token_tx) = self
                .handle
                .coordination_rx
                .recv()
                .expect("Invariant broken: could not receive token channels.");

            // run the computation
            let result = f(&mut self.tx);

            // Constraint #1:
            // - I have the token and
            // - All other transactions started already
            match token_rx.recv() {
                Err(_) => {
                    // we failed in the execution: tear down
                    return None;
                }
                Ok(token) => {
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

                    // whatever happens, I need to pass along the token
                    token_tx
                        .send(token)
                        .expect("Invariant broken: could not send token.");

                    // report the decision
                    match decision {
                        (TransactionControl::Abort, r) => {
                            self.handle
                                .done_tx
                                .send(Done::Completed)
                                .expect("Invariant broken: sending done signal failed.");
                            return r;
                        }
                        (TransactionControl::Retry, _) => {
                            // clear log before retrying computation
                            self.tx.clear();
                            self.handle
                                .done_tx
                                .send(Done::Retry)
                                .expect("Invariant broken: sending done signal failed.");
                        }
                    }
                }
            }
        }
    }
}
