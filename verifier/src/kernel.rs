//! CTT kernel: the Expr ADT (basis §II), gas-bounded deterministic evaluation
//! (basis §III), and judgemental equality (basis §IV, Task 3).

/// The universe of discourse. Types are expressions (Bool, Nat, Arrow, Prod
/// are canonical type values); there is no term/type distinction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Var(String),
    Lam(String, Box<Expr>),
    Ap(Box<Expr>, Box<Expr>),
    Pair(Box<Expr>, Box<Expr>),
    Fst(Box<Expr>),
    Snd(Box<Expr>),
    True,
    False,
    /// if(then; els)(scrutinee)
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Zero,
    Succ(Box<Expr>),
    /// rec(base; pred, acc. step)(target)
    Rec {
        base: Box<Expr>,
        pred: String,
        acc: String,
        step: Box<Expr>,
        target: Box<Expr>,
    },
    Bool,
    Nat,
    Arrow(Box<Expr>, Box<Expr>),
    Prod(Box<Expr>, Box<Expr>),
}

/// Maximum expression depth accepted by `eval` and `check_eq`. Inputs (and
/// intermediate expressions) deeper than this produce `KernelError::TooDeep`
/// rather than a native stack overflow.  The frame-stack in `eval` de-recurses
/// the driver loop only; `is_val`, `subst`, and `step` remain structurally
/// recursive and are therefore bounded by this limit.
pub const MAX_DEPTH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    OutOfGas,
    Stuck(String),
    UnboundVar(String),
    TooDeep,
}

impl Expr {
    pub fn var(x: &str) -> Expr { Expr::Var(x.into()) }
    pub fn lam(x: &str, body: Expr) -> Expr { Expr::Lam(x.into(), Box::new(body)) }
    pub fn ap(f: Expr, a: Expr) -> Expr { Expr::Ap(Box::new(f), Box::new(a)) }
    pub fn pair(a: Expr, b: Expr) -> Expr { Expr::Pair(Box::new(a), Box::new(b)) }
    pub fn fst(e: Expr) -> Expr { Expr::Fst(Box::new(e)) }
    pub fn snd(e: Expr) -> Expr { Expr::Snd(Box::new(e)) }
    pub fn if_(then: Expr, els: Expr, scrutinee: Expr) -> Expr {
        Expr::If(Box::new(then), Box::new(els), Box::new(scrutinee))
    }
    pub fn succ(e: Expr) -> Expr { Expr::Succ(Box::new(e)) }
    pub fn rec(base: Expr, pred: &str, acc: &str, step: Expr, target: Expr) -> Expr {
        Expr::Rec {
            base: Box::new(base),
            pred: pred.into(),
            acc: acc.into(),
            step: Box::new(step),
            target: Box::new(target),
        }
    }
    pub fn arrow(a: Expr, b: Expr) -> Expr { Expr::Arrow(Box::new(a), Box::new(b)) }
    pub fn prod(a: Expr, b: Expr) -> Expr { Expr::Prod(Box::new(a), Box::new(b)) }

    /// Canonical forms (basis §III): final, no further transitions.
    pub fn is_val(&self) -> bool {
        match self {
            Expr::True | Expr::False | Expr::Zero | Expr::Lam(..)
                | Expr::Bool | Expr::Nat => true,
            Expr::Succ(n) => n.is_val(),
            Expr::Pair(a, b) => a.is_val() && b.is_val(),
            Expr::Arrow(a, b) | Expr::Prod(a, b) => a.is_val() && b.is_val(),
            _ => false,
        }
    }
}

/// Iterative `Drop` for `Expr` so that deeply-nested terms (e.g. succ^100_000(0))
/// can be freed without overflowing the native stack.  The default recursive
/// drop would fault at ≈8 k depth on a typical 8 MiB stack.
impl Drop for Expr {
    fn drop(&mut self) {
        // Collect owned children into a worklist; avoid recursion entirely.
        let mut worklist: Vec<Expr> = Vec::new();

        // Drain `self` in place so that Rust's implicit drop of fields is a no-op.
        fn drain(e: &mut Expr, wl: &mut Vec<Expr>) {
            use std::mem;
            match e {
                Expr::Lam(_, b) => {
                    let child = mem::replace(b.as_mut(), Expr::Zero);
                    wl.push(child);
                }
                Expr::Ap(f, a) | Expr::Pair(f, a) | Expr::Arrow(f, a) | Expr::Prod(f, a) => {
                    let cf = mem::replace(f.as_mut(), Expr::Zero);
                    let ca = mem::replace(a.as_mut(), Expr::Zero);
                    wl.push(cf);
                    wl.push(ca);
                }
                Expr::Fst(a) | Expr::Snd(a) | Expr::Succ(a) => {
                    let child = mem::replace(a.as_mut(), Expr::Zero);
                    wl.push(child);
                }
                Expr::If(t, f, s) => {
                    let ct = mem::replace(t.as_mut(), Expr::Zero);
                    let cf = mem::replace(f.as_mut(), Expr::Zero);
                    let cs = mem::replace(s.as_mut(), Expr::Zero);
                    wl.push(ct);
                    wl.push(cf);
                    wl.push(cs);
                }
                Expr::Rec { base, step, target, .. } => {
                    let cb = mem::replace(base.as_mut(), Expr::Zero);
                    let cs = mem::replace(step.as_mut(), Expr::Zero);
                    let ct = mem::replace(target.as_mut(), Expr::Zero);
                    wl.push(cb);
                    wl.push(cs);
                    wl.push(ct);
                }
                // Leaf variants: nothing to drain.
                Expr::Var(_) | Expr::True | Expr::False | Expr::Zero
                | Expr::Bool | Expr::Nat => {}
            }
        }

        drain(self, &mut worklist);
        while let Some(mut node) = worklist.pop() {
            drain(&mut node, &mut worklist);
            // `node` is dropped here but its children were already drained into
            // the worklist, so the implicit drop of `node` is cheap (leaf only).
        }
    }
}

/// succ^k(0)
pub fn nat(k: u64) -> Expr {
    let mut e = Expr::Zero;
    for _ in 0..k {
        e = Expr::succ(e);
    }
    e
}

/// Enumerate the immediate sub-expressions of `e`.
fn children(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Lam(_, b) => vec![b],
        Expr::Ap(f, a) => vec![f, a],
        Expr::Pair(a, b) => vec![a, b],
        Expr::Fst(a) | Expr::Snd(a) | Expr::Succ(a) => vec![a],
        Expr::If(t, f, s) => vec![t, f, s],
        Expr::Rec { base, step, target, .. } => vec![base, step, target],
        Expr::Arrow(a, b) | Expr::Prod(a, b) => vec![a, b],
        _ => vec![],
    }
}

/// Iterative (worklist-based) depth measurement — no recursion, no stack risk.
fn depth(e: &Expr) -> usize {
    let mut worklist: Vec<(&Expr, usize)> = vec![(e, 1)];
    let mut max = 0usize;
    while let Some((node, d)) = worklist.pop() {
        if d > max {
            max = d;
        }
        for child in children(node) {
            worklist.push((child, d + 1));
        }
    }
    max
}

/// Substitution e[v/x]. We only evaluate closed terms, so v is closed and no
/// capture-avoidance is needed; binder shadowing must still be respected.
fn subst(e: &Expr, x: &str, v: &Expr) -> Expr {
    match e {
        Expr::Var(y) if y == x => v.clone(),
        Expr::Var(_) | Expr::True | Expr::False | Expr::Zero | Expr::Bool | Expr::Nat => e.clone(),
        Expr::Lam(y, _) if y == x => e.clone(),
        Expr::Lam(y, b) => Expr::Lam(y.clone(), Box::new(subst(b, x, v))),
        Expr::Ap(f, a) => Expr::ap(subst(f, x, v), subst(a, x, v)),
        Expr::Pair(a, b) => Expr::pair(subst(a, x, v), subst(b, x, v)),
        Expr::Fst(a) => Expr::fst(subst(a, x, v)),
        Expr::Snd(a) => Expr::snd(subst(a, x, v)),
        Expr::If(t, f, s) => Expr::if_(subst(t, x, v), subst(f, x, v), subst(s, x, v)),
        Expr::Succ(a) => Expr::succ(subst(a, x, v)),
        Expr::Rec { base, pred, acc, step, target } => {
            let step2 = if pred == x || acc == x { (**step).clone() } else { subst(step, x, v) };
            Expr::Rec {
                base: Box::new(subst(base, x, v)),
                pred: pred.clone(),
                acc: acc.clone(),
                step: Box::new(step2),
                target: Box::new(subst(target, x, v)),
            }
        }
        Expr::Arrow(a, b) => Expr::arrow(subst(a, x, v), subst(b, x, v)),
        Expr::Prod(a, b) => Expr::prod(subst(a, x, v), subst(b, x, v)),
    }
}

/// One deterministic transition step (basis §III). Returns the stepped
/// expression; calling on a value is a kernel bug surfaced as Stuck.
fn step(e: &Expr) -> Result<Expr, KernelError> {
    match e {
        Expr::Var(x) => Err(KernelError::UnboundVar(x.clone())),
        Expr::If(t, f, s) => match &**s {
            Expr::True => Ok((**t).clone()),
            Expr::False => Ok((**f).clone()),
            s_ if s_.is_val() => Err(KernelError::Stuck("if on non-bool".into())),
            _ => Ok(Expr::if_((**t).clone(), (**f).clone(), step(s)?)),
        },
        Expr::Ap(f, a) => match &**f {
            Expr::Lam(x, body) => Ok(subst(body, x, a)),
            f_ if f_.is_val() => Err(KernelError::Stuck("apply non-function".into())),
            _ => Ok(Expr::ap(step(f)?, (**a).clone())),
        },
        Expr::Fst(p) => match &**p {
            Expr::Pair(a, _) => Ok((**a).clone()),
            p_ if p_.is_val() => Err(KernelError::Stuck("project non-pair".into())),
            _ => Ok(Expr::fst(step(p)?)),
        },
        Expr::Snd(p) => match &**p {
            Expr::Pair(_, b) => Ok((**b).clone()),
            p_ if p_.is_val() => Err(KernelError::Stuck("project non-pair".into())),
            _ => Ok(Expr::snd(step(p)?)),
        },
        // Succ/Pair stepping is handled in eval's context loop to avoid deep recursion.
        Expr::Succ(_) | Expr::Pair(..) => Err(KernelError::Stuck("step on canonical form".into())),
        Expr::Rec { base, pred, acc, step: st, target } => match &**target {
            Expr::Zero => Ok((**base).clone()),
            Expr::Succ(n) => {
                let rec_n = Expr::Rec {
                    base: base.clone(),
                    pred: pred.clone(),
                    acc: acc.clone(),
                    step: st.clone(),
                    target: n.clone(),
                };
                Ok(subst(&subst(st, pred, n), acc, &rec_n))
            }
            t_ if t_.is_val() => Err(KernelError::Stuck("rec on non-nat".into())),
            _ => Ok(Expr::Rec {
                base: base.clone(),
                pred: pred.clone(),
                acc: acc.clone(),
                step: st.clone(),
                target: Box::new(step(target)?),
            }),
        },
        _ => Err(KernelError::Stuck("step on canonical form".into())),
    }
}

/// Alpha-equivalence of canonical forms, used as the kernel's sound
/// approximation of extensional function equality (full extensionality has
/// ∀∃ quantifier complexity and is not decidable — basis §IV).
fn alpha_eq(a: &Expr, b: &Expr, env: &mut Vec<(String, String)>) -> bool {
    match (a, b) {
        (Expr::Var(x), Expr::Var(y)) => env
            .iter()
            .rev()
            .find(|(l, _)| l == x)
            .map(|(_, r)| r == y)
            .unwrap_or(x == y),
        (Expr::Lam(x, ba), Expr::Lam(y, bb)) => {
            env.push((x.clone(), y.clone()));
            let r = alpha_eq(ba, bb, env);
            env.pop();
            r
        }
        (Expr::Ap(f1, a1), Expr::Ap(f2, a2)) => alpha_eq(f1, f2, env) && alpha_eq(a1, a2, env),
        (Expr::Pair(a1, b1), Expr::Pair(a2, b2)) => alpha_eq(a1, a2, env) && alpha_eq(b1, b2, env),
        (Expr::Fst(p1), Expr::Fst(p2)) | (Expr::Snd(p1), Expr::Snd(p2)) => alpha_eq(p1, p2, env),
        (Expr::If(t1, f1, s1), Expr::If(t2, f2, s2)) => {
            alpha_eq(t1, t2, env) && alpha_eq(f1, f2, env) && alpha_eq(s1, s2, env)
        }
        (Expr::Succ(n1), Expr::Succ(n2)) => alpha_eq(n1, n2, env),
        (
            Expr::Rec { base: b1, pred: p1, acc: c1, step: s1, target: t1 },
            Expr::Rec { base: b2, pred: p2, acc: c2, step: s2, target: t2 },
        ) => {
            let base_t = alpha_eq(b1, b2, env) && alpha_eq(t1, t2, env);
            env.push((p1.clone(), p2.clone()));
            env.push((c1.clone(), c2.clone()));
            let s = alpha_eq(s1, s2, env);
            env.pop();
            env.pop();
            base_t && s
        }
        (Expr::Arrow(a1, b1), Expr::Arrow(a2, b2)) | (Expr::Prod(a1, b1), Expr::Prod(a2, b2)) => {
            alpha_eq(a1, a2, env) && alpha_eq(b1, b2, env)
        }
        _ => a == b,
    }
}

/// The verifier judgement M ≐ M' ∈ A (basis §IV): evaluate the type and both
/// terms to canonical form, then switch on the structure of the type.
/// Structural recursion (Nat/Prod cases) shares the finite gas budget; each
/// recursive call costs at least 1 gas unit so the budget is always consumed.
pub fn check_eq(ty: &Expr, m1: &Expr, m2: &Expr, gas: &mut u64) -> Result<bool, KernelError> {
    if *gas == 0 {
        return Err(KernelError::OutOfGas);
    }
    *gas -= 1;

    if depth(ty) > MAX_DEPTH || depth(m1) > MAX_DEPTH || depth(m2) > MAX_DEPTH {
        return Err(KernelError::TooDeep);
    }

    let tyv = eval(ty, gas)?;
    let a = eval(m1, gas)?;
    let b = eval(m2, gas)?;
    match &tyv {
        Expr::Bool => Ok(matches!(
            (&a, &b),
            (Expr::True, Expr::True) | (Expr::False, Expr::False)
        )),
        Expr::Nat => match (&a, &b) {
            (Expr::Zero, Expr::Zero) => Ok(true),
            (Expr::Succ(n1), Expr::Succ(n2)) => check_eq(&Expr::Nat, n1, n2, gas),
            _ => Ok(false),
        },
        Expr::Prod(t1, t2) => match (&a, &b) {
            (Expr::Pair(a1, a2), Expr::Pair(b1, b2)) => {
                Ok(check_eq(t1, a1, b1, gas)? && check_eq(t2, a2, b2, gas)?)
            }
            _ => Ok(false),
        },
        Expr::Arrow(_, _) => Ok(alpha_eq(&a, &b, &mut Vec::new())),
        _ => Err(KernelError::Stuck("non-type in type position".into())),
    }
}

/// Evaluation context frames for iterative handling of Succ/Pair nesting.
/// The frame stack de-recurses the eval driver loop only; `is_val`, `subst`,
/// and `step` remain structurally recursive and are safe because all inputs
/// (and intermediate working expressions) are bounded by MAX_DEPTH.
#[derive(Debug)]
enum Frame {
    Succ,
    PairLeft(Expr),   // waiting for left; right already known
    PairRight(Expr),  // left is a value; waiting for right
}

/// Extract the inner `Box<Expr>` from a `Succ` without pattern-destructuring
/// (which is forbidden when `Expr` implements `Drop`).
fn take_succ_inner(e: &mut Expr) -> Box<Expr> {
    match e {
        Expr::Succ(inner) => std::mem::replace(inner, Box::new(Expr::Zero)),
        _ => panic!("take_succ_inner called on non-Succ"),
    }
}

/// Extract both `Box<Expr>` children from a `Pair`.
fn take_pair(e: &mut Expr) -> (Box<Expr>, Box<Expr>) {
    match e {
        Expr::Pair(a, b) => (
            std::mem::replace(a, Box::new(Expr::Zero)),
            std::mem::replace(b, Box::new(Expr::Zero)),
        ),
        _ => panic!("take_pair called on non-Pair"),
    }
}

fn plug(frames: Vec<Frame>, mut v: Expr) -> Expr {
    for frame in frames.into_iter().rev() {
        v = match frame {
            Frame::Succ => Expr::succ(v),
            Frame::PairLeft(right) => Expr::pair(v, right),
            Frame::PairRight(left) => Expr::pair(left, v),
        };
    }
    v
}

/// Big-step evaluation E ⇓ E∘: iterate ↦ until canonical, bounded by gas.
/// The frame stack de-recurses the eval driver for Succ/Pair contexts; structural
/// recursion in `is_val`, `subst`, and `step` is bounded by the MAX_DEPTH check
/// at entry and the unconditional per-step depth check below.
pub fn eval(e: &Expr, gas: &mut u64) -> Result<Expr, KernelError> {
    if depth(e) > MAX_DEPTH {
        return Err(KernelError::TooDeep);
    }

    let mut cur = e.clone();
    let mut frames: Vec<Frame> = Vec::new();

    loop {
        // Peel off Succ/Pair wrappers into frames until we reach the active redex.
        loop {
            // We cannot destructure `cur` by move while it implements Drop, so
            // we use helper functions that take the Box fields out explicitly.
            let needs_peel = match &cur {
                Expr::Succ(inner) if !inner.is_val() => 1,
                Expr::Pair(a, _) if !a.is_val() => 2,
                Expr::Pair(_, b) if !b.is_val() => 3,
                _ => 0,
            };
            if needs_peel == 0 { break; }

            // Drain children out of `cur` in place (our iterative Drop helper
            // does the same), then reassemble what we need.
            match needs_peel {
                1 => {
                    // cur = Succ(inner), inner not val
                    let inner_box = take_succ_inner(&mut cur);
                    frames.push(Frame::Succ);
                    cur = *inner_box;
                }
                2 => {
                    // cur = Pair(a, b), a not val
                    let (a_box, b_box) = take_pair(&mut cur);
                    frames.push(Frame::PairLeft(*b_box));
                    cur = *a_box;
                }
                3 => {
                    // cur = Pair(a, b), a is val, b not val
                    let (a_box, b_box) = take_pair(&mut cur);
                    frames.push(Frame::PairRight(*a_box));
                    cur = *b_box;
                }
                _ => unreachable!(),
            }
        }

        if cur.is_val() {
            // Rewrap with frames and continue if there are more frames.
            if frames.is_empty() {
                return Ok(cur);
            }
            let frame = frames.pop().unwrap();
            cur = match frame {
                Frame::Succ => Expr::succ(cur),
                Frame::PairLeft(right) => {
                    // Left is now a val; push PairRight frame, evaluate right.
                    frames.push(Frame::PairRight(cur));
                    right
                }
                Frame::PairRight(left) => Expr::pair(left, cur),
            };
            continue;
        }

        if *gas == 0 {
            return Err(KernelError::OutOfGas);
        }
        *gas -= 1;
        let stepped = step(&cur)?;
        cur = plug(frames, stepped);
        frames = Vec::new();

        // Unconditional per-step depth check. A single rec-on-Succ step can
        // amplify expression depth significantly (subst of a deep step body into
        // a deep rec_n term), so checking only periodically is unsound.
        //
        // Invariant: whenever a step begins, depth(cur) <= MAX_DEPTH, which means
        // each step's internal recursion (subst, is_val) operates on terms of
        // depth at most MAX_DEPTH. subst is structurally recursive with frames of
        // ~2 KiB in debug builds; callers must provide ≥ 16 MiB of stack to
        // keep MAX_DEPTH (4096) levels safe in all build profiles.
        if depth(&cur) > MAX_DEPTH {
            return Err(KernelError::TooDeep);
        }
    }
}

/// Encode bytes as a right-nested pair of Nats: (b0, (b1, ... bn-1)).
/// Total: the empty slice encodes as Zero (a degenerate but valid canonical
/// form at type Nat, matching bytes_type(0) = Nat). Used to lift content
/// hashes into canonical forms for check_eq.
pub fn from_bytes(bs: &[u8]) -> Expr {
    let Some((last, rest)) = bs.split_last() else {
        return Expr::Zero;
    };
    let mut e = nat(u64::from(*last));
    for b in rest.iter().rev() {
        e = Expr::pair(nat(u64::from(*b)), e);
    }
    e
}

/// The type of `from_bytes` output for a given length: Nat × (Nat × ... Nat).
/// Total: len 0 yields Nat (the type of the degenerate empty encoding).
pub fn bytes_type(len: usize) -> Expr {
    let mut ty = Expr::Nat;
    for _ in 1..len {
        ty = Expr::prod(Expr::Nat, ty);
    }
    ty
}
