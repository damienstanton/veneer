# veneer Oxidation: The Lift, the Judgement, and its Soundness

Normative companion to `spec/veneer.md` (harness contract) and `spec/basis.md`
(CTT foundation). This document states precisely what the oxidation pipeline
decides, what the signature lift preserves and erases, and the exact scope of
the resulting guarantee. Its framing follows Patterson, Wagner & Ahmed's
*linking types* [PWA23]: the shadow is a typed mediation between a source
language and Rust's affine discipline.

## 1. Two verifiers, one basis

veneer runs two verifiers, each deciding a different judgement:

- The **CTT kernel** (`kernel.rs`) decides judgemental equality
  $M \doteq M' \in A$ by evaluation to canonical form (basis §IV). It is
  total (gas- and depth-bounded) and verifies idempotency witnesses.
- **Oxidation** (`oxidize.rs`) decides *type and ownership coherence* of a
  Rust program. rustc — its type checker and borrow checker — is the trusted
  decision procedure; veneer adds nothing to the judgement. It transports the
  program in and the diagnostics out, as `Law::Oxidation` findings.

The two meet at effects and resources: basis §VII treats state change as a
declared operation, and Rust's affine types are the strongest widely-deployed
checker of exactly that discipline — a value consumed twice, aliased while
mutated, or dropped while borrowed is a *type error*. Oxidation makes that
checker available to code written in any language.

## 2. The shadow and the judgement

A **shadow** is a self-contained Rust program with `todo!()` bodies whose
signatures model the ownership story of real code — authored by the agent
(`veneer oxidize`), or lifted mechanically from extracted signatures (the
knowledge graph).

The judgement decided:

    ⊢rustc shadow        (accepted: no error diagnostics)

Findings are the constructive refutation: each diagnostic names the line of
the shadow whose ownership story is incoherent.

**Determinism.** For a fixed shadow, edition, and toolchain, `cargo check`
diagnostics are deterministic; `parse_diagnostics` sorts and deduplicates, so
the finding trace is a function of the shadow.
*Witnesses:* `same_shadow_twice_is_byte_identical`,
`findings_are_sorted_for_determinism`, `identical_diagnostics_are_deduplicated`.

## 3. The lift ⌈·⌉

`graph::lift_shadow` (engine: `graph_lift.rs`) turns a file's extracted
`pub fn` signatures into a shadow by *generic erasure*: every
project-specific type identifier becomes a fresh type parameter, consistently
within its signature.

**Preserved** — the linking-relevant structure:

- **P1** Arity and parameter order.
- **P2** The borrow shape of every position: value vs `&` vs `&mut`,
  including nesting through prelude containers (`Vec<_>`, `Option<_>`,
  `Box<_>`, `Result<_,_>`, slices, tuples).
- **P3** Type sharing: two positions of the same source type map to the same
  parameter `Ti` — "returns the same type it borrows" survives erasure.
- **P4** Primitives and prelude types are kept as themselves; `std::`/`core::`
  paths resolve absolutely and are kept.

**Erased** — deliberately not part of the judgement:

- **E1** Concrete names of project types (each becomes some `Ti`).
- **E2** Type arguments of an erased type (`Registry<String>` ⇒ `T0`): a type
  parameter cannot itself take arguments; P2 keeps the outer ownership shape.
- **E3** All bodies, values, and effects: the shadow proves coherence of the
  *interface* ownership story, nothing about behavior.

**Skipped — sound by omission.** Some signatures cannot be erased without
manufacturing errors that are artifacts of the lift rather than properties of
the code. These are skipped: recorded as plain signature facts, contributing
nothing to the canonical form.

- **S1** Signatures that already declare generics or lifetimes
  (`pub fn f<'a>(…)`): merging two parameter lists is not reliably mechanical.
- **S2** Unbalanced fragments of multi-line declarations.
- **S3** Non-function items (`struct`/`enum`/`trait`/`type`): declared, not
  compiled.
- **S4** Trait-position types (`impl Trait`, `dyn Trait`): erasing a *trait*
  to a type parameter is ill-formed, and a lift that guessed a bound would be
  deciding a different judgement than the source states.

The property this buys:

> **Lift soundness (informal).** Every finding the lifted pipeline attaches
> to a module is a genuine rustc-derived property of the module's erased
> public ownership story — never an artifact of the erasure. The lift may
> decline to judge (S1–S4); it does not misjudge.

*Witnesses:* the `lift_shadow_*` tests in `verifier/tests/graph.rs` (P1–P4,
E1–E2, S1–S4), `lifted_primitive_signature_actually_compiles`,
`lifted_ambiguous_lifetime_signature_yields_a_real_oxidation_finding`,
`impl_trait_signature_produces_no_false_semantic_findings`, and
`lift_shadow_is_total_and_deterministic` (`verifier/tests/graph_props.rs`).

## 4. The linking-types reading [PWA23]

Linking types extend a source language's type system just enough to type its
interactions with code from another language, so that reasoning done in the
source language survives linking. veneer runs that mediation in reverse, as a
verification artifact:

- The *target* discipline is Rust's affine type system — the strongest common
  vocabulary for ownership.
- The *lift* is the linking translation: it maps each public signature into
  the target vocabulary preserving exactly the structure the target judgement
  consumes (P1–P4) and nothing else (E1–E3).
- The *faithfulness obligation* (§5, N1) is the linking contract: the verdict
  transfers to the real code exactly when the real code inhabits the lifted
  signature.

Patterson–Wagner–Ahmed's program is *semantic* encapsulation: what a type
means is what its inhabitants may do, across languages. In that spirit the
shadow's meaning is behavioral (basis §I): it specifies what may be owned,
borrowed, and consumed — and rustc is the algorithm that decides inhabitation.

## 5. The guarantee, scoped

**Guaranteed** (each clause mechanically witnessed):

- **G1** Determinism: identical tree + config + shadow ⇒ identical findings.
- **G2** A clean oxidation means rustc found the modeled ownership story
  coherent — type-checked and borrow-checked.
- **G3** Whatever the lift judges, it judges honestly (lift soundness, §3).
- **G4** Totality: every failure is a `Finding`, never a panic; run failures
  (missing cargo, timeout) are Protocol findings, distinguished from
  judgements.

**Not guaranteed** (out of scope, by design):

- **N1** Faithfulness: if the real code's ownership story diverges from the
  shadow, the verdict is about the shadow. Keeping them aligned is the
  agent's stated obligation (skill: "fix the *real* design the shadow models
  — keep the shadow faithful to it").
- **N2** Completeness: skipped signatures (S1–S4) are unjudged; extraction is
  heuristic, single-line, top-level (`spec/veneer.md`, Knowledge graph).
- **N3** Behavior: `todo!()` bodies mean no claim about what the code
  computes — only about what it owns and borrows.

## 6. Trust boundary

Unchanged from `spec/veneer.md`: `cargo check` expands macros at check time;
the shadow is trusted input at the same level as project source. The
`edition` allowlist prevents manifest injection; the shadow body is not
sandboxed.

## References

- **[PWA23]** Daniel Patterson, Andrew Wagner, and Amal J. Ahmed.
  *Semantic Encapsulation using Linking Types.* Proceedings of the 8th ACM
  SIGPLAN International Workshop on Type-Driven Development (TyDe), 2023.
  <https://api.semanticscholar.org/CorpusID:261395954>

      @article{Patterson2023SemanticEU,
        title={Semantic Encapsulation using Linking Types},
        author={Daniel Patterson and Andrew Wagner and Amal J. Ahmed},
        journal={Proceedings of the 8th ACM SIGPLAN International Workshop
                 on Type-Driven Development},
        year={2023},
        url={https://api.semanticscholar.org/CorpusID:261395954}
      }

- Harper, Martin-Löf, Constable et al., Bauer — see `spec/basis.md`.
