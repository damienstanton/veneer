# veneer Formal Basis: Computational Type Theory, Distilled

This document grounds veneer in **Computational Type Theory** (Harper, OPLSS 2018,
building on Martin-Löf and Constable et al.) and **algebraic effects and handlers**
(Bauer). It is deliberately minimal: only the judgements and consequences the
harness actually uses. It names no host language, because veneer prescribes
*discipline*, not syntax.

### Key references

- Harper, R. *Computational Type Theory*. OPLSS 2018 lectures.
- Martin-Löf, P. *Constructive Mathematics and Computer Programming*. 1979.
- Constable, R. et al. *Implementing Mathematics with the NuPRL Proof Development System*. 1986.
- Bauer, A. *Algebraic Effects and Handlers*.

---

## 0. The Polymorphism Principle

veneer is language-polymorphic. The basis is defined by behavior — evaluation,
canonical forms, judgemental equality — not by the grammar of any implementation
language. Every law degrades gracefully:

| Capability | Where native | Where absent — emulate with |
|---|---|---|
| Sum types (variants) | ADTs with exhaustive match | Tagged records + total dispatch (every tag handled, no default fallthrough) |
| Product types (composition) | Records / structs / tuples | Maps with fixed, documented key sets |
| Sealed modules | Module signatures / export lists | Directory boundaries + a documented public surface; consumers touch only that surface |
| Errors as data | Result / Option types | Discriminated return values; never naked exceptions across module boundaries |
| Value semantics | Immutable bindings, persistent data | Freeze/copy at boundaries; never share mutable state across them |

Languages that manage type-level design well (ADTs, signatures) give the model
deterministic compiler feedback and will work best. Languages without these
features still admit the full discipline — the agent and the verifier supply the
judgement the compiler does not.

---

## I. Foundational Principle

> Types are specifications of program **behavior**.

Truth is defined by execution, not by formal derivation. A type and a term are
both *programs*; a type is an expression that evaluates to a canonical type value:

$$A\ \text{type} \qquad\qquad M \in A$$

Judgements are expressions of knowledge in the intuitionistic sense: the only way
to constrain facts about infinite structures is via algorithms.

---

## II. The Domain: Abstract Syntax

The universe of discourse, an inductive set of expressions (no syntactic
distinction between terms and types):

$$e ::= x \mid \lambda x.e \mid \text{ap}(e_1, e_2) \mid (e_1, e_2) \mid e.1 \mid e.2$$
$$\quad\;\mid \text{true} \mid \text{false} \mid \text{if}(e_1; e_2)(e)$$
$$\quad\;\mid 0 \mid \text{succ}(e) \mid \text{rec}(e_0;\, a, b.\, e_1)(e)$$
$$\quad\;\mid a\!:\!A_1 \to A_2 \;\mid\; a\!:\!A_1 \times A_2$$

The last line gives the dependent function ($\Pi$) and dependent pair ($\Sigma$)
types; when $a$ is not free in $A_2$ they reduce to the simple $A_1 \to A_2$ and
$A_1 \times A_2$. Because types are expressions, type-level computation is
first-class: $\text{if}(\text{Nat}; \text{Bool})(M)$ is a type when
$M \in \text{Bool}$.

---

## III. The Dynamics: Operational Semantics

A **deterministic** transition system with two judgement forms:

$$E\ \text{val} \qquad\qquad E \mapsto E'$$

**Canonical forms (values):** $\text{true}$, $\text{false}$, $0$,
$\text{succ}(M)$, $\lambda a.M$, $(M_1, M_2)$.

**Transitions (representative rules):**

$$\text{if}(E_1; E_2)(\text{true}) \mapsto E_1 \qquad \text{ap}(\lambda a. M_2,\, M_1) \mapsto M_2[M_1/a] \qquad (M_1, M_2).i \mapsto M_i$$

$$\text{rec}(M_\circ; a,b.M_1)(0) \mapsto M_\circ \qquad \text{rec}(M_\circ; a,b.M_1)(\text{succ}(M)) \mapsto M_1[M,\, R(M)/a,b]$$

with congruence rules stepping the principal argument. **Big-step evaluation** is
the derived notion $E \Downarrow E_\circ \;\stackrel{\text{def}}{=}\; E \mapsto^\star E_\circ\ \text{val}$.

Two properties matter to the harness:

1. **Determinism** — equal inputs produce equal results (equisatisfaction).
2. **Finality** — once $E_\circ\ \text{val}$, no further transitions occur.

---

## IV. The Verifier: Judgemental Equality

The master judgement:

$$\Gamma \vdash M \doteq M' \in A$$

"$M$ and $M'$ are equal inhabitants of type $A$." To verify it: evaluate
$A \Downarrow A_\circ$, $M \Downarrow M_\circ$, $M' \Downarrow M'_\circ$, then
switch on the structure of $A_\circ$:

| $A_\circ$ | $M_\circ \doteq_\circ M'_\circ \in A_\circ$ holds iff |
|---|---|
| $\text{Bool}$ | both evaluate to $\text{true}$, or both to $\text{false}$ (extremal: nothing else) |
| $\text{Nat}$ | both $\Downarrow 0$, or both $\Downarrow \text{succ}(\cdot)$ with equal predecessors (least such relation — the induction principle) |
| $A_1 \to A_2$ | both $\Downarrow \lambda a.\cdot$ and the bodies are equal at $A_2$ for all equal arguments at $A_1$ (extensionality) |
| $A_1 \times A_2$ | both $\Downarrow (\cdot,\cdot)$ with components equal at $A_1$, $A_2$ |
| $a\!:\!A_1 \to A_2$ | as functions, with the result type $A_2[M_1/a]$ tracking the argument |
| $a\!:\!A_1 \times A_2$ | as pairs, with $M_2 \doteq M'_2 \in A_2[M_1/a]$ encoding the dependency |

Typing is closed under **head expansion** (reverse execution): if
$M \mapsto M'$ and $M' \in A$ then $M \in A$.

**Key insight:** *when two things are equal is a property of the type they
inhabit.* $2 \doteq 4 \in \text{Nat}$ is false; $2 \doteq 4 \in \text{Nat}/2$ is
true. Equality at function types has $\forall\exists$ quantifier complexity and
therefore cannot be axiomatized by any formalism (Gödel); semantics is primary,
syntax a useful approximation.

A program $M$ "type checks" precisely when:

$$M \doteq M \in \text{ExpectedType}$$

---

## V. Functionality & Contexts

A context $\Gamma$ is an ordered list of hypotheses $x : A$. The fundamental
structural property is **functionality**: families respect equality of indices.

$$a : A \gg B\ \text{type} \quad\text{means}\quad M \doteq M' \in A \implies B[M/a] \doteq B[M'/a]$$

Example: $\text{seq}(2+2)$ must be the same type as $\text{seq}(4)$ — verified by
reducing index terms to canonical form. Equal indices deterministically produce
equal results; this is why the operational semantics must be deterministic.

---

## VI. Propositions as Types

| Logic | Type | Name |
|---|---|---|
| $\top$ | $1$ | unit |
| $\bot$ | $0$ | void |
| $\Phi_1 \land \Phi_2$ | $\Phi_1 \times \Phi_2$ | product |
| $\Phi_1 \lor \Phi_2$ | $\Phi_1 + \Phi_2$ | sum |
| $\Phi_1 \supset \Phi_2$ | $\Phi_1 \to \Phi_2$ | function |
| $\forall a\!:\!A.\, \Phi_a$ | $a\!:\!A \to \Phi_a$ | dependent function ($\Pi$) |
| $\exists a\!:\!A.\, \Phi_a$ | $a\!:\!A \times \Phi_a$ | dependent pair ($\Sigma$) |

A specification is a type; an inhabitant is its proof. Validation *is* proof of
conformance.

---

## VII. Algebraic Effects

An effect is described by operation symbols with parameter type $B$, continuation
type $C$, and result type $A$:

$$Op : B \times A^C \to A$$

A handler interprets operation symbols by providing transition rules, restoring
deterministic evaluation. Types remain behavioral specifications — an effectful
type specifies input-output behavior *and* the effect protocol. State change is an
explicit, declared operation, never an implicit mutation.

---

## VIII. Implementation Strategy (Any Host Language)

To implement the basis in any language $\mathcal{L}$:

1. **Define the domain as data.** A recursive variant type (or tagged-record
   emulation) for `Expr`, covering all forms of §II.
2. **Total evaluator.** `eval(Expr) -> Result<Expr, Error>` implementing §III,
   with a gas/depth bound to guarantee termination in the presence of general
   recursion.
3. **Equivalence checker.** `check_eq(Env, Type, Term, Term) -> bool`
   implementing §IV: normalize via `eval`, recurse on the shape of the type.
4. **Errors as data.** Stuck terms (applying a non-function, projecting a
   non-pair) are values in an error variant, never crashes.

---

## IX. Design Consequences

The basis is prescriptive. These follow directly from §III–V, independent of
language:

1. **Value semantics are required.** $E \mapsto E'$ produces a new expression;
   canonical forms are final; judgemental equality is atemporal. Implementations
   may use references internally, but observable behavior must be
   indistinguishable from pure value semantics: two expressions equal once are
   equal forever.

2. **Mutative inheritance is incompatible.** Dynamic dispatch makes evaluation
   depend on hidden state (the receiver's runtime class), breaking the
   deterministic transition system; base-class changes break functionality;
   reference identity has no semantic content — CTT has exactly one equality,
   structural, defined by evaluation to canonical forms.

3. **Implicit composition is incompatible.** Mixin/linearization schemes make
   meaning depend on the *order* of composition — a syntactic accident — and
   introduce shared mutable state, a temporal dependency that atemporal equality
   cannot express.

4. **Use the CTT primitives instead:**

| Need | Incompatible idiom | CTT primitive |
|---|---|---|
| Variants | Class hierarchy + runtime type tests | Sum type $A_1 + A_2$, total elimination |
| Composition | Mixins, multiple inheritance | Product type $A_1 \times A_2$ |
| Behavior | Virtual methods, hidden dispatch | Function type $A_1 \to A_2$, extensional |
| Indexed families | Erased generics | Dependent function $a\!:\!A \to B_a$ |
| Existential packaging | Abstract base class | Dependent pair $a\!:\!A \times B_a$ |
| State change | Field mutation, setters | Declared effect $Op : B \times A^C \to A$ |
| Absence | null / sentinel values | Void $0$ inside a sum (an `Option`-shaped type) |

5. **In practice, in any language:** use classes (if present) as namespaces over
   data, not inheritance hierarchies; use the language's closest approximation of
   tagged unions with exhaustive handling; treat mutation as a declared effect at
   module boundaries; prefer immutable domain data; make equality structural,
   never referential. Where the compiler cannot enforce these, the agent and the
   veneer verifier enforce them as protocol.

---

## Appendix: Notation

| Symbol | Meaning |
|---|---|
| $E\ \text{val}$ | $E$ is a canonical form (fully evaluated) |
| $E \mapsto E'$ | one computation step |
| $E \Downarrow E_\circ$ | $E$ evaluates to canonical form $E_\circ$ |
| $A\ \text{type}$ | $A$ is a type ($A \doteq A$) |
| $M \in A$ | $M$ inhabits $A$ ($M \doteq M \in A$) |
| $M \doteq M' \in A$ | $M$, $M'$ are equal elements of $A$ |
| $a : A \gg J$ | hypothetical judgement: $J$ holds assuming $a \in A$ |
| $B[M/a]$ | substitution of $M$ for $a$ in $B$ |
| $\Gamma$ | typing context |
