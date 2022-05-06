[![Crates.io](https://img.shields.io/crates/v/cargo-readme.svg)](https://crates.io/crates/cargo-readme)

# Software Transactional Memory -- stm

This library implements
[software transactional memory](https://en.wikipedia.org/wiki/Software_transactional_memory),
often abbreviated with STM.

It is designed closely to haskells STM library. Read Simon Marlow's
*Parallel and Concurrent Programming in Haskell*
for more info. Especially the chapter about
Performance is also important for using STM in rust.

With locks the sequential composition of two
two threadsafe actions is no longer threadsafe because
other threads may interfer in between of these actions.
Applying a third lock to protect both may lead to common sources of errors
like deadlocks or race conditions.

Unlike locks Software transactional memory is composable.
It is typically implemented by writing all read and write
operations in a log. When the action has finished and
all the used `TVar`s are consistent, the writes are commited as
a single atomic operation.
Otherwise the computation repeats. This may lead to starvation,
but avoids common sources of bugs.

Panicing within STM does not poison the `TVar`s. STM ensures consistency by
never committing on panic.

## Usage

You should only use the functions that are transaction-safe.
Transaction-safe functions don't have side effects, except those provided by `TVar`.
Mutexes and other blocking mechanisms are especially dangerous, because they can
interfere with the internal locking scheme of the transaction and therefore
cause deadlocks.

Note, that Transaction-safety does *not* mean safety in the rust sense, but is a
subset of allowed behavior. Even if code is not transaction-safe, no segmentation
faults will happen.

You can run the top-level atomic operation by calling `atomically`.


```rust
atomically(|trans| {
    // some action
    // return value as `Result`, for example
    Ok(42)
});
```

Nested calls to `atomically` are not allowed. A run-time check prevents this.
Instead of using atomically internally, add a `&mut Transaction` parameter and
return `StmResult`.

Use ? on `StmResult`, to propagate a transaction error through the system.
Do not handle the error yourself.

```rust
let var = TVar::new(0);

let x = atomically(|trans| {
    var.write(trans, 42)?; // Pass failure to parent.
    var.read(trans) // Return the value saved in var.
});

println!("var = {}", x);
// var = 42

```

## Transaction safety

Software transactional memory is completely safe in the rust sense, so
undefined behavior will never occur.
Still there are multiple rules that
you should obey when dealing with software transactional memory.

* Don't run code with side effects, especially no IO-code.
Transactions repeat in failure cases. Using IO would repeat this IO-code.
Return a closure if you have to.
* Don't handle `StmResult` yourself.
Use `Transaction::or` to combine alternative paths and `optionally` to check if an inner
function has failed. Always use `?` and
never ignore a `StmResult`.
* Don't run `atomically` inside of another. `atomically` is designed to have side effects
and will therefore break transaction safety.
Nested calls are detected at runtime and handled with panicking.
When you use STM in the inner of a function, then
express it in the public interface, by taking `&mut Transaction` as parameter and
returning `StmResult<T>`. Callers can safely compose it into
larger blocks.
* Don't mix locks and transactions. Your code will easily deadlock or slow
down unpredictably.
* Don't use inner mutability to change the content of a `TVar`.

Panicking in a transaction is transaction-safe. The transaction aborts and
all changes are discarded. No poisoning or half written transactions happen.

## Speed

Generally keep your atomic blocks as small as possible, because
the more time you spend, the more likely it is, to collide with
other threads. For STM, reading `TVar`s is quite slow, because it
needs to look them up in the log every time.
Every used `TVar` increases the chance of collisions. Therefore you should
keep the amount of accessed variables as low as needed.

## Determinism

The whole idea of STM is speculative parallelism at the cost of
non-deterministic behavior.
Nevertheless there is a desire to execute computation deterministically,
for easier debugging and predictable performance. Hence, this library
also provides a deterministic STM implementation.

### Programming model

The programming model for deterministic STM is a bit more involved:

```rust

let f = |tx: &mut _| {
            // some code here
            Ok(5)
        };
let g = |tx: &mut _| {
            // some code here
            Ok(10)
        };

let mut dtm = dtm();
let tx1 = dtm.register();
let tx2 = dtm.register();
freeze(dtm);

thread::spawn(move || det_atomically(tx1, f) );
thread::spawn(move || det_atomically(tx2, g) );

```
Registering produces a handle that we can pass to a transaction.
Each transaction wants to own a handle and as such each handle can only be passed to one
transaction.
Handles are the way to specify the order of the transactions.
In the above example code, the transaction executing function `f` is executed before the
transaction executing function `g`.
Whether we call `freeze` before or after spawning the threads for the transactions is not
important.
But processing only starts when the set of transactions is frozen.

### Limitations of the programming model

The programming model is more restricted because the developer needs to specify an order
between the transactions running concurrently.
The following limitation is detrimental:

* Transactions \\*must not* share a thread.

Violating this limitation leads to deadlocks.

### Implications

Often in STM applications multiple small transactions are placed onto a single thread to solve
the thread granularity problem.
This is not easily possible anymore.
Instead, it is necessary to use one large transaction.
The problem is obvious:
What is the order between the 2nd transaction on thread 1 and the second transaction on
thread 2?
In order to solve this problem, the deterministic STM runtime would also need to take control
of the execution.

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
