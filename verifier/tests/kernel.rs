use veneer::kernel::{check_eq, eval, nat, Expr, KernelError};

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

#[test]
fn bool_equality_after_evaluation() {
    // if(true; false)(true) ≐ true ∈ Bool
    let m = Expr::if_(Expr::True, Expr::False, Expr::True);
    let mut gas = 100;
    assert!(check_eq(&Expr::Bool, &m, &Expr::True, &mut gas).unwrap());
    let mut gas = 100;
    assert!(!check_eq(&Expr::Bool, &m, &Expr::False, &mut gas).unwrap());
}

#[test]
fn nat_equality_is_structural() {
    // 2 + 2 ≐ 4 ∈ Nat (functionality: equal indices, canonical compare)
    let add = |m: Expr, n: Expr| Expr::rec(n, "p", "acc", Expr::succ(Expr::var("acc")), m);
    let mut gas = 1000;
    assert!(check_eq(&Expr::Nat, &add(nat(2), nat(2)), &nat(4), &mut gas).unwrap());
    let mut gas = 1000;
    assert!(!check_eq(&Expr::Nat, &nat(2), &nat(4), &mut gas).unwrap());
}

#[test]
fn product_equality_is_componentwise() {
    let ty = Expr::prod(Expr::Bool, Expr::Nat);
    let a = Expr::pair(Expr::True, nat(2));
    let b = Expr::pair(Expr::True, Expr::fst(Expr::pair(nat(2), nat(9))));
    let mut gas = 100;
    assert!(check_eq(&ty, &a, &b, &mut gas).unwrap());
}

#[test]
fn function_equality_is_alpha() {
    let ty = Expr::arrow(Expr::Bool, Expr::Bool);
    let f = Expr::lam("x", Expr::var("x"));
    let g = Expr::lam("y", Expr::var("y"));
    let mut gas = 100;
    assert!(check_eq(&ty, &f, &g, &mut gas).unwrap());
    let h = Expr::lam("x", Expr::True);
    let mut gas = 100;
    assert!(!check_eq(&ty, &f, &h, &mut gas).unwrap());
}

#[test]
fn type_position_is_evaluated() {
    // if(Bool; Nat)(true) is the type Bool — types are programs (basis §II)
    let ty = Expr::if_(Expr::Bool, Expr::Nat, Expr::True);
    let mut gas = 100;
    assert!(check_eq(&ty, &Expr::True, &Expr::True, &mut gas).unwrap());
}

#[test]
fn non_type_in_type_position_is_stuck() {
    let mut gas = 100;
    assert!(matches!(
        check_eq(&Expr::True, &Expr::True, &Expr::True, &mut gas),
        Err(KernelError::Stuck(_))
    ));
}
