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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    OutOfGas,
    Stuck(String),
    UnboundVar(String),
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

/// succ^k(0)
pub fn nat(k: u64) -> Expr {
    let mut e = Expr::Zero;
    for _ in 0..k {
        e = Expr::succ(e);
    }
    e
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
        Expr::Succ(n) => Ok(Expr::succ(step(n)?)),
        Expr::Pair(a, b) => {
            if !a.is_val() {
                Ok(Expr::pair(step(a)?, (**b).clone()))
            } else {
                Ok(Expr::pair((**a).clone(), step(b)?))
            }
        }
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
pub fn check_eq(ty: &Expr, m1: &Expr, m2: &Expr, gas: &mut u64) -> Result<bool, KernelError> {
    let tyv = eval(ty, gas)?;
    let a = eval(m1, gas)?;
    let b = eval(m2, gas)?;
    match tyv {
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
                Ok(check_eq(&t1, a1, b1, gas)? && check_eq(&t2, a2, b2, gas)?)
            }
            _ => Ok(false),
        },
        Expr::Arrow(_, _) => Ok(alpha_eq(&a, &b, &mut Vec::new())),
        _ => Err(KernelError::Stuck("non-type in type position".into())),
    }
}

/// Big-step evaluation E ⇓ E∘: iterate ↦ until canonical, bounded by gas.
pub fn eval(e: &Expr, gas: &mut u64) -> Result<Expr, KernelError> {
    let mut cur = e.clone();
    loop {
        if cur.is_val() {
            return Ok(cur);
        }
        if *gas == 0 {
            return Err(KernelError::OutOfGas);
        }
        *gas -= 1;
        cur = step(&cur)?;
    }
}
