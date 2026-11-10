//! The round-trip counter of issue #39 against a real Postgres: a `CountingConnector` counts exactly the statements the
//! adapter sends, and each connector's count is its own.
//!
//! `model_counting_connector_counts_exactly_the_statements_of_generated_sequences` generates sequences of transaction and
//! read calls and checks, step by step, how many statements each sent against a small model of what the adapter sends
//! (read from `crates/thrust-adapters-postgres/src/lib.rs`, written out below).
//! `model_counting_strategy_reaches_every_step_class` is its Rule 8 control: a generator that never wrote inside a
//! transaction, say, would pass on a counter that missed the `BEGIN`. Three properties a generated sequence cannot show
//! are pinned as tests of their own.
//!
//! What the adapter sends:
//!
//! * `begin` checks a connection out and sends nothing: `BEGIN` waits for the first write (FR-22);
//! * `allocate` is one statement, on the transaction's own connection before a write and on a second one after it (the
//!   second checkout is why the counter sits on the connector and not on `conn()`);
//! * `insert_work_item` is one statement, and `BEGIN` goes first when it is the transaction's first write;
//! * `commit` and `rollback` are one statement each when a write opened the transaction, and none otherwise;
//! * a dropped transaction sends nothing: its connection is discarded, and the server rolls back what it held;
//! * `work_item_page` is one statement for a page that has rows and two for one that has none (the second is the count);
//! * `list_work_items` is one statement.
//!
//! Budget: `THRUST_MODEL_CASES` cases (default 32), shared with the other model test. Every call has a 20 s timeout.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence, TestCaseError, TestRunner};
use thrust_adapters_postgres::{CountingConnector, PgDatabase, RoundTrips};
use thrust_contracts::*;
use thrust_ports::*;

use super::{env, item, scope, seeded, Env};

const CALL: Duration = Duration::from_secs(20);

/// A database over the environment's pool whose statements are counted into the returned counter, as the native server
/// builds one per request and the Worker builds one per request on its own connector.
fn counted(env: &Env) -> (PgDatabase, RoundTrips) {
    let rt = RoundTrips::new();
    (PgDatabase::new(Arc::new(CountingConnector::new(env.conn.clone(), rt.clone()))), rt)
}

/// One port call, bounded: a wedged pool fails the case and does not hang the run.
async fn call<T>(what: &str, f: impl Future<Output = Result<T, DbError>>) -> Result<T, TestCaseError> {
    match tokio::time::timeout(CALL, f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(TestCaseError::fail(format!("{what}: {e:?}"))),
        Err(_) => Err(TestCaseError::fail(format!("{what} took longer than {CALL:?}"))),
    }
}

#[derive(Clone, Debug)]
enum Step {
    /// `allocate` on the open transaction, opening one with `begin` if none is open.
    Allocate,
    /// `insert_work_item` on the open transaction (opened if need be), with a number of its own.
    Insert,
    /// `commit` the open transaction (opened if need be: an empty one is a case of its own).
    Commit,
    Rollback,
    /// Drop the open transaction unfinished.
    Abandon,
    /// `work_item_page` of the project's committed rows.
    Page {
        limit: u32,
        page: u32,
    },
    /// `list_work_items`.
    List,
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        prop_oneof![
            3 => Just(Step::Allocate),
            4 => Just(Step::Insert),
            2 => Just(Step::Commit),
            1 => Just(Step::Rollback),
            1 => Just(Step::Abandon),
            2 => (1u32..=3, 0u32..=3).prop_map(|(limit, page)| Step::Page { limit, page }),
            1 => Just(Step::List),
        ],
        1..24,
    )
}

/// The kinds of step the model tells apart, which is what the control requires the generator to reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    AllocateBeforeWrite,
    AllocateAfterWrite,
    InsertFirst,
    InsertLater,
    CommitWritten,
    CommitEmpty,
    RollbackWritten,
    RollbackEmpty,
    AbandonWritten,
    AbandonEmpty,
    PageWithRows,
    PageWithoutRows,
    List,
}

impl Class {
    const ALL: [Class; 13] = [
        Class::AllocateBeforeWrite,
        Class::AllocateAfterWrite,
        Class::InsertFirst,
        Class::InsertLater,
        Class::CommitWritten,
        Class::CommitEmpty,
        Class::RollbackWritten,
        Class::RollbackEmpty,
        Class::AbandonWritten,
        Class::AbandonEmpty,
        Class::PageWithRows,
        Class::PageWithoutRows,
        Class::List,
    ];
}

/// What the adapter has sent and written so far, which is all the statement count of the next step depends on.
#[derive(Default)]
struct Model {
    /// A write has opened the transaction: `BEGIN` has been sent.
    wrote: bool,
    /// Rows written in the open transaction, visible to everyone else once it commits.
    pending: u32,
    /// Rows the project shows to any connection.
    visible: u32,
}

impl Model {
    /// The statements `step` sends, and its class; the model then moves on as the adapter has.
    fn plan(&mut self, step: &Step) -> (u32, Class) {
        match step {
            Step::Allocate => (1, if self.wrote { Class::AllocateAfterWrite } else { Class::AllocateBeforeWrite }),
            Step::Insert => {
                let first = !self.wrote;
                self.wrote = true;
                self.pending += 1;
                if first {
                    (2, Class::InsertFirst)
                } else {
                    (1, Class::InsertLater)
                }
            }
            Step::Commit => {
                let wrote = self.wrote;
                self.visible += self.pending;
                self.end();
                if wrote {
                    (1, Class::CommitWritten)
                } else {
                    (0, Class::CommitEmpty)
                }
            }
            Step::Rollback => {
                let wrote = self.wrote;
                self.end();
                if wrote {
                    (1, Class::RollbackWritten)
                } else {
                    (0, Class::RollbackEmpty)
                }
            }
            Step::Abandon => {
                let wrote = self.wrote;
                self.end();
                (0, if wrote { Class::AbandonWritten } else { Class::AbandonEmpty })
            }
            Step::Page { limit, page } => {
                if page * limit < self.visible {
                    (1, Class::PageWithRows)
                } else {
                    (2, Class::PageWithoutRows)
                }
            }
            Step::List => (1, Class::List),
        }
    }

    fn end(&mut self) {
        self.wrote = false;
        self.pending = 0;
    }
}

async fn run_case(env: &Env, steps: &[Step]) -> Result<(), TestCaseError> {
    // a project of the case's own, seeded through the uncounted database: only the steps are counted
    let (ws, _other, p, _u) = seeded(env).await;
    let s = scope(ws, p);
    let (db, rt) = counted(env);
    let mut model = Model::default();
    let mut tx: Option<Box<dyn Tx>> = None;
    // numbers of the case's own, far from those `allocate` hands out (nothing inserts those)
    let mut seq = 1_000_000_i64;
    for (n, step) in steps.iter().enumerate() {
        let (want, class) = model.plan(step);
        let before = rt.get();
        match step {
            Step::Page { limit, page } => {
                let q = WorkItemQuery {
                    order: WorkItemOrder { field: WorkItemOrderField::SequenceId, descending: false },
                    limit: *limit,
                    page: *page,
                    updated_after: None,
                    created_by: None,
                };
                call("work_item_page", db.work_item_page(&s, &q)).await?;
            }
            Step::List => {
                call("list_work_items", db.list_work_items(&s, Visibility::Visible)).await?;
            }
            _ => {
                if tx.is_none() {
                    tx = Some(call("begin", db.begin(&s)).await?);
                }
                match step {
                    Step::Allocate => {
                        let t = tx.as_mut().unwrap();
                        call("allocate", t.allocate(&s)).await?;
                    }
                    Step::Insert => {
                        seq += 1;
                        let a = Allocation { sequence_id: seq, sort_order: seq * 10_000 };
                        let t = tx.as_mut().unwrap();
                        call("insert_work_item", t.insert_work_item(&s, &item(ws, p, a, "counted", None))).await?;
                    }
                    Step::Commit => {
                        let t = tx.take().unwrap();
                        call("commit", t.commit()).await?;
                    }
                    Step::Rollback => {
                        let t = tx.take().unwrap();
                        call("rollback", t.rollback()).await?;
                    }
                    Step::Abandon => drop(tx.take()),
                    Step::Page { .. } | Step::List => {}
                }
            }
        }
        let got = rt.get() - before;
        prop_assert_eq!(
            got,
            want,
            "step {} ({:?}) sent {} statements, the model says {}: {:?}",
            n,
            class,
            got,
            want,
            &steps[..=n]
        );
    }
    drop(tx);
    Ok(())
}

fn config() -> Config {
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config {
        cases,
        failure_persistence,
        source_file: Some(file!()),
        max_shrink_iters: 512,
        max_shrink_time: 120_000,
        ..Config::default()
    }
}

#[test]
fn model_counting_connector_counts_exactly_the_statements_of_generated_sequences() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let env = rt.block_on(env());
    let config = config();
    assert!(config.cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&steps(), |steps| rt.block_on(run_case(&env, &steps)));
    rt.block_on(async move { drop(env) });
    if let Err(e) = result {
        panic!("{e}");
    }
}

/// Rule 8 for the test above: samples sequences with a fixed seed, plans them with the model alone and requires every
/// class of step at least once.
#[test]
fn model_counting_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..256 {
        let sequence = steps().new_tree(&mut runner).unwrap().current();
        let mut model = Model::default();
        for step in &sequence {
            seen.insert(model.plan(step).1);
        }
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "the generator never produced {missing:?}");
}

/// Pinned regression (issue #39): `Counted::discard` must reach the inner handle. The trait's default `discard` does
/// nothing, which hands a connection that is still inside an abandoned transaction back to the pool: the wedge
/// `mutant_discard_noop` makes, through the counting connector. The same loop as the uncounted test of the same name,
/// with the count the loop must cost: dropping a transaction sends nothing.
#[tokio::test]
async fn an_abandoned_transaction_does_not_poison_the_pool_through_the_counting_connector() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let s = scope(ws, p);
    let (db, rt) = counted(&env);
    tokio::time::timeout(Duration::from_secs(30), async {
        for _ in 0..40 {
            // more iterations than pool slots, and the abandoned transaction WRITES, so it is inside one when dropped
            let mut tx = db.begin(&s).await.unwrap();
            let a = tx.allocate(&s).await.unwrap();
            tx.insert_work_item(&s, &item(ws, p, a, "abandoned", None)).await.unwrap();
            drop(tx); // never committed
        }
        assert_eq!(
            rt.get(),
            40 * 3,
            "an iteration is its allocation, its BEGIN and its INSERT; the drop sends nothing"
        );
        let mut tx = db.begin(&s).await.unwrap();
        let a = tx.allocate(&s).await.unwrap();
        assert_eq!(a.sequence_id, 41, "the numbers of abandoned transactions are not given back (FR-22)");
        tx.insert_work_item(&s, &item(ws, p, a, "kept", None)).await.unwrap();
        tx.commit().await.unwrap();
        let rows = db.list_work_items(&s, Visibility::WithDeleted).await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["kept"],
            "abandoned transactions must roll back"
        );
    })
    .await
    .expect("abandoned transactions wedged the pool");
}

/// Pinned regression (issue #39): a statement counts when it is sent, not when it succeeds. Postgres refuses a
/// duplicate live external key after the INSERT has gone out, and `BEGIN` went before it.
#[tokio::test]
async fn a_statement_postgres_refuses_still_counts() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let s = scope(ws, p);
    let mut tx = env.db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    tx.insert_work_item(&s, &item(ws, p, a, "keep", Some("1"))).await.unwrap();
    tx.commit().await.unwrap();

    let (db, rt) = counted(&env);
    let mut tx = db.begin(&s).await.unwrap();
    let a = tx.allocate(&s).await.unwrap();
    assert_eq!(rt.get(), 1, "the allocation");
    let err = tx.insert_work_item(&s, &item(ws, p, a, "dup", Some("1"))).await.unwrap_err();
    assert!(matches!(err, DbError::Conflict(_)), "got {err:?}");
    assert_eq!(rt.get(), 3, "the refused INSERT was sent, and so was the BEGIN before it");
    tx.rollback().await.unwrap();
    assert_eq!(rt.get(), 4, "rolling back what the refused write left open is a statement");
}

/// Pinned regression (issue #39): a count is the connector's, not the pool's or the process's. The native server builds
/// one connector per request over one shared pool, and requests overlap, so two counted databases working at once must
/// not see each other's statements, and an uncounted one over the same pool must count in neither.
#[tokio::test]
async fn counting_connectors_over_one_pool_count_separately() {
    let env = env().await;
    let (ws, _o, p, _u) = seeded(&env).await;
    let s = scope(ws, p);
    let (a, a_rt) = counted(&env);
    let (b, b_rt) = counted(&env);
    tokio::join!(lists(&a, s, 5), lists(&b, s, 7), lists(env.db.as_ref(), s, 11));
    assert_eq!((a_rt.get(), b_rt.get()), (5, 7));
}

/// `n` lists of the project's work items, one statement each.
async fn lists(db: &PgDatabase, s: Scope, n: usize) {
    for _ in 0..n {
        db.list_work_items(&s, Visibility::Visible).await.unwrap();
    }
}
