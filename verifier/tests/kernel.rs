use veneer::kernel::{eval, nat, Expr, KernelError};

#[test]
fn values_evaluate_to_themselves() {
    let mut gas = 100;
    assert_eq!(eval(&Expr::True, &mut gas).unwrap(), Expr::True);
    assert_eq!(eval(&nat(3), &mut gas).unwrap(), nat(3));
}

#[test]
fn if_selects_branch() {
    let mut gas = 100;
    let e = Expr::if_(nat(1), nat(2), Expr::True);
    assert_eq!(eval(&e, &mut gas).unwrap(), nat(1));
    let e = Expr::if_(nat(1), nat(2), Expr::False);
    let mut gas = 100;
    assert_eq!(eval(&e, &mut gas).unwrap(), nat(2));
}

#[test]
fn beta_reduction_substitutes() {
    // ap(λx.pair(x,x), true) ⇓ (true, true)
    let f = Expr::lam("x", Expr::pair(Expr::var("x"), Expr::var("x")));
    let mut gas = 100;
    let v = eval(&Expr::ap(f, Expr::True), &mut gas).unwrap();
    assert_eq!(v, Expr::pair(Expr::True, Expr::True));
}

#[test]
fn projections_reduce() {
    let p = Expr::pair(nat(1), nat(2));
    let mut gas = 100;
    assert_eq!(eval(&Expr::fst(p.clone()), &mut gas).unwrap(), nat(1));
    let mut gas = 100;
    assert_eq!(eval(&Expr::snd(p), &mut gas).unwrap(), nat(2));
}

#[test]
fn rec_computes_addition() {
    // add(m, n) = rec(n; _, acc. succ(acc))(m)
    let add = |m: Expr, n: Expr| Expr::rec(n, "p", "acc", Expr::succ(Expr::var("acc")), m);
    let mut gas = 1000;
    assert_eq!(eval(&add(nat(2), nat(3)), &mut gas).unwrap(), nat(5));
}

#[test]
fn binder_shadowing_respected() {
    // ap(λx.λx.x, true) applied to false ⇓ false (inner binder wins)
    let inner = Expr::lam("x", Expr::var("x"));
    let outer = Expr::lam("x", inner);
    let e = Expr::ap(Expr::ap(outer, Expr::True), Expr::False);
    let mut gas = 100;
    assert_eq!(eval(&e, &mut gas).unwrap(), Expr::False);
}

#[test]
fn omega_runs_out_of_gas() {
    // (λx. x x)(λx. x x) — untypable, but the evaluator must stay total.
    let w = Expr::lam("x", Expr::ap(Expr::var("x"), Expr::var("x")));
    let omega = Expr::ap(w.clone(), w);
    let mut gas = 50;
    assert_eq!(eval(&omega, &mut gas), Err(KernelError::OutOfGas));
}

#[test]
fn stuck_terms_are_errors_not_panics() {
    let mut gas = 100;
    assert!(matches!(eval(&Expr::fst(Expr::True), &mut gas), Err(KernelError::Stuck(_))));
    let mut gas = 100;
    assert!(matches!(eval(&Expr::var("x"), &mut gas), Err(KernelError::UnboundVar(_))));
}
