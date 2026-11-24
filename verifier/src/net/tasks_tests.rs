use super::*;
use crate::net::envelope::tests::{task, witness};
use crate::net::envelope::{Check, Outcome, TaskResult};
use crate::net::hlc::Stamp;
use crate::net::op::{Body, Op};

const LEASE: u64 = 60_000;

fn op(ms: u64, replica: &str, body: Body) -> Op {
    Op { at: Stamp { wall_ms: ms, counter: 0, replica: replica.into() }, body }
}
fn create(ms: u64) -> Op {
    op(ms, "orch", Body::TaskCreate { id: "t1".into(), task: task() })
}
fn claim(ms: u64, r: &str, run: &str) -> Op {
    op(ms, r, Body::TaskClaim { id: "t1".into(), run: run.into(), harness: "claude".into(), lease_ms: LEASE })
}
fn beat(ms: u64, r: &str, run: &str) -> Op {
    op(ms, r, Body::TaskHeartbeat { id: "t1".into(), run: run.into() })
}
fn done(ms: u64, r: &str, run: &str) -> Op {
    let mut res = TaskResult::new(Outcome::Done, "shipped");
    res.witness = Some(witness());
    res.acceptance = vec![Check { criterion: "cargo test passes".into(), pass: true, note: None }];
    op(ms, r, Body::TaskComplete { id: "t1".into(), run: run.into(), result: res })
}
fn one(ops: &[Op]) -> TaskState {
    let (m, orphans) = fold(ops);
    assert!(orphans.is_empty(), "{orphans:?}");
    m.get("t1").cloned().unwrap()
}

#[test]
fn happy_path() {
    let s = one(&[create(0), claim(10, "mac", "r1"), beat(20, "mac", "r1"), done(30, "mac", "r1")]);
    assert_eq!(s.status_at(30), Status::Done);
    assert_eq!(s.result.unwrap().summary, "shipped");
    assert!(s.ignored.is_empty());
}

#[test]
fn concurrent_claims_resolve_to_earliest_stamp_on_every_replica() {
    let ops = vec![create(0), claim(10, "mac", "r1"), claim(10, "mini", "r2")];
    let mut reversed = ops.clone();
    reversed[1..].reverse();
    reversed.sort_by(|a, b| a.at.cmp(&b.at)); // what op::load does
    for o in [ops, reversed] {
        let s = one(&o);
        assert_eq!(s.holder.as_ref().unwrap().run, "r1", "mac < mini on the replica tiebreak");
        assert_eq!(s.ignored.len(), 1);
    }
}

#[test]
fn loser_cannot_complete() {
    let s = one(&[create(0), claim(10, "mac", "r1"), claim(11, "mini", "r2"), done(20, "mini", "r2")]);
    assert_eq!(s.status_at(20), Status::Running);
    assert!(s.ignored.iter().any(|i| i.contains("does not hold")));
}

#[test]
fn lapsed_lease_requeues_then_fails() {
    // Lease lapses as observed at query time.
    let s = one(&[create(0), claim(10, "mac", "r1")]);
    assert_eq!(s.status_at(10 + LEASE), Status::Running);
    assert_eq!(s.status_at(11 + LEASE), Status::Queued);
    // A later claim by another replica is granted once the lease has lapsed.
    let s = one(&[create(0), claim(10, "mac", "r1"), claim(20 + LEASE, "mini", "r2")]);
    assert_eq!(s.holder.as_ref().unwrap().run, "r2");
    assert_eq!(s.lapsed, 1);
    // Heartbeats keep it alive.
    let s = one(&[create(0), claim(10, "mac", "r1"), beat(LEASE, "mac", "r1"), claim(LEASE + 20, "mini", "r2")]);
    assert_eq!(s.holder.as_ref().unwrap().run, "r1");
    // Third lapse fails the task.
    let mut ops = vec![create(0)];
    for k in 0..MAX_ATTEMPTS as u64 {
        ops.push(claim(10 + k * 2 * LEASE, "mac", &format!("r{k}")));
    }
    let s = one(&ops);
    let last = 10 + (MAX_ATTEMPTS as u64 - 1) * 2 * LEASE;
    assert_eq!(s.status_at(last + LEASE + 1), Status::Failed);
}

#[test]
fn ask_answer_resume() {
    let ask = op(20, "mac", Body::TaskAsk {
        id: "t1".into(),
        run: "r1".into(),
        question: "Which API?".into(),
        session_id: Some("sess".into()),
    });
    let answer = op(30, "orch", Body::TaskAnswer { id: "t1".into(), text: "v2".into() });
    let s = one(&[create(0), claim(10, "mac", "r1"), ask.clone()]);
    assert_eq!(s.status_at(25), Status::NeedsInput);
    assert!(s.status_at(25).settled());
    let s = one(&[create(0), claim(10, "mac", "r1"), ask, answer, claim(40, "mac", "r2"), done(50, "mac", "r2")]);
    assert_eq!(s.dialogue, vec!["Q: Which API?", "A: v2"]);
    assert_eq!(s.resume_session.as_deref(), Some("sess"));
    assert_eq!(s.status_at(50), Status::Done);
}

#[test]
fn done_without_witness_is_ignored() {
    let mut r = TaskResult::new(Outcome::Done, "trust me");
    r.acceptance = vec![];
    let bad = op(20, "mac", Body::TaskComplete { id: "t1".into(), run: "r1".into(), result: r });
    let s = one(&[create(0), claim(10, "mac", "r1"), bad]);
    assert_eq!(s.status_at(20), Status::Running);
    assert!(s.ignored[0].contains("witness"));
}

#[test]
fn cancel_absorbs_later_ops() {
    let cancel = op(15, "orch", Body::TaskCancel { id: "t1".into(), reason: "dup".into() });
    let s = one(&[create(0), claim(10, "mac", "r1"), cancel, done(20, "mac", "r1")]);
    assert_eq!(s.status_at(20), Status::Canceled);
    assert!(s.result.is_none());
}

#[test]
fn orphans_and_dependencies() {
    let (m, orphans) = fold(&[claim(5, "mac", "r1")]);
    assert!(m.is_empty() && orphans.len() == 1);
    let mut dependent = task();
    dependent.depends_on = vec!["t1".into()];
    let (m, _) = fold(&[create(0)]);
    assert!(!deps_done(&m, &dependent, 0));
    let (m, _) = fold(&[create(0), claim(1, "mac", "r1"), done(2, "mac", "r1")]);
    assert!(deps_done(&m, &dependent, 2));
}
