use veneer::kernel::{bytes_type, check_eq, eval, from_bytes, nat, Expr, KernelError};

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

use proptest::prelude::*;

#[test]
fn byte_encoding_roundtrips_through_check_eq() {
    let h1: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 255];
    let h2: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 254];
    let ty = bytes_type(8);
    let mut gas = 100_000;
    assert!(check_eq(&ty, &from_bytes(&h1), &from_bytes(&h1), &mut gas).unwrap());
    let mut gas = 100_000;
    assert!(!check_eq(&ty, &from_bytes(&h1), &from_bytes(&h2), &mut gas).unwrap());
}

proptest! {
    #[test]
    fn check_eq_is_reflexive_on_bytes(bs in proptest::collection::vec(any::<u8>(), 1..16)) {
        let ty = bytes_type(bs.len());
        let mut gas = 1_000_000;
        prop_assert!(check_eq(&ty, &from_bytes(&bs), &from_bytes(&bs), &mut gas).unwrap());
    }

    #[test]
    fn check_eq_is_symmetric_on_bytes(
        len in 1usize..8,
        a in proptest::collection::vec(any::<u8>(), 1..8),
        b in proptest::collection::vec(any::<u8>(), 1..8),
    ) {
        // Truncate/extend to exactly `len` bytes to avoid mass rejection.
        let a: Vec<u8> = a.into_iter().cycle().take(len).collect();
        let b: Vec<u8> = b.into_iter().cycle().take(len).collect();
        let ty = bytes_type(len);
        let mut g1 = 1_000_000;
        let mut g2 = 1_000_000;
        let ab = check_eq(&ty, &from_bytes(&a), &from_bytes(&b), &mut g1).unwrap();
        let ba = check_eq(&ty, &from_bytes(&b), &from_bytes(&a), &mut g2).unwrap();
        prop_assert_eq!(ab, ba);
    }

    #[test]
    fn eval_is_total_under_gas(n in 0u64..500) {
        // Primitive recursion terminates; gas makes even adversarial terms total.
        let add = Expr::rec(nat(n), "p", "acc", Expr::succ(Expr::var("acc")), nat(n));
        let mut gas = 10_000;
        let r = eval(&add, &mut gas);
        prop_assert!(r.is_ok() || r == Err(KernelError::OutOfGas));
    }
}

#[test]
fn check_eq_consumes_gas() {
    let mut gas = 10; // far less than needed for 2000-deep structural compare
    assert_eq!(
        check_eq(&Expr::Nat, &nat(2000), &nat(2000), &mut gas),
        Err(KernelError::OutOfGas)
    );
}

#[test]
fn too_deep_input_is_an_error_not_an_abort() {
    let mut e = Expr::Zero;
    for _ in 0..100_000 {
        e = Expr::succ(e);
    }
    let mut gas = 1_000_000;
    assert_eq!(eval(&e, &mut gas), Err(KernelError::TooDeep));
    let mut gas = 1_000_000;
    assert_eq!(
        check_eq(&Expr::Nat, &e, &e, &mut gas),
        Err(KernelError::TooDeep)
    );
}

#[test]
fn alpha_eq_distinguishes_free_from_bound() {
    let ty = Expr::arrow(Expr::Bool, Expr::Bool);
    // lambda x.x vs lambda x.y — bound vs free
    let f = Expr::lam("x", Expr::var("x"));
    let g = Expr::lam("x", Expr::var("y"));
    let mut gas = 100;
    assert!(!check_eq(&ty, &f, &g, &mut gas).unwrap());
    // shadowing: lambda x.lambda x.x ≡α lambda a.lambda b.b
    let h1 = Expr::lam("x", Expr::lam("x", Expr::var("x")));
    let h2 = Expr::lam("a", Expr::lam("b", Expr::var("b")));
    let mut gas = 100;
    assert!(check_eq(&Expr::arrow(Expr::Bool, Expr::arrow(Expr::Bool, Expr::Bool)), &h1, &h2, &mut gas).unwrap());
}

#[test]
fn empty_byte_encoding_is_total_not_a_panic() {
    let mut gas = 100;
    assert!(check_eq(&bytes_type(0), &from_bytes(&[]), &from_bytes(&[]), &mut gas).unwrap());
}

#[test]
fn rec_depth_amplification_is_bounded_not_an_abort() {
    // rec(0; p, acc. succ^3000(acc))(succ^3000(0)) — passes the entry depth
    // guard, then each unrolling step amplifies depth; must error, not abort.
    //
    // The test spawns a thread with 8 MiB stack to match the assumption baked
    // into MAX_DEPTH: `subst` is structurally recursive and safe up to that
    // depth from an 8 MiB stack.  Test-harness threads are smaller by default.
    let result = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let mut body = Expr::var("acc");
            for _ in 0..3000 {
                body = Expr::succ(body);
            }
            let mut target = Expr::Zero;
            for _ in 0..3000 {
                target = Expr::succ(target);
            }
            let e = Expr::rec(Expr::Zero, "p", "acc", body, target);
            let mut gas = 200_000_000;
            eval(&e, &mut gas)
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(
        matches!(result, Err(KernelError::TooDeep) | Err(KernelError::OutOfGas)),
        "expected bounded error, got {result:?}"
    );
}
