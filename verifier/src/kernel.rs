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
