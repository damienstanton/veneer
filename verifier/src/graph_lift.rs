//! The generic-erasure lift engine used by `graph::build` for Rust files:
//! takes heuristically-extracted `pub fn` signatures and renders a
//! self-contained, generic-parameterized Rust program preserving their
//! ownership/borrowing shape, with concrete project-specific types erased.
//! No dependency on `graph`'s persistence or extraction concerns — pure
//! string-to-string transformation, independently testable.

/// Types resolvable in a bare scratch crate without an extra `use`: the Rust
/// prelude plus primitives. Kept as-is during erasure; anything else
/// starting with an uppercase letter is treated as a project-specific type.
fn is_prelude_or_primitive(tok: &str) -> bool {
    matches!(
        tok,
        "Vec" | "Option" | "Box" | "Result" | "String" | "Self" | "Some" | "None" | "Ok" | "Err"
            | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "f32" | "f64" | "bool" | "char" | "str"
    )
}

/// Split into (token, is_identifier) runs: identifier runs are `[A-Za-z0-9_]+`;
/// everything else (whitespace, punctuation, `&`, `<`, `>`, etc.) passes
/// through untouched in its own run. Lets erasure rewrite identifiers in
/// place without re-deriving Rust's grammar.
fn tokenize(s: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_is_ident = false;
    for c in s.chars() {
        let is_ident_char = c.is_alphanumeric() || c == '_';
        if !cur.is_empty() && is_ident_char != cur_is_ident {
            out.push((std::mem::take(&mut cur), cur_is_ident));
        }
        cur_is_ident = is_ident_char;
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push((cur, cur_is_ident));
    }
    out
}

/// Merge `ident :: ident :: ... :: ident` runs from `tokenize`'s output into
/// a single identifier token holding the full path text (e.g.
/// `"serde_json::Value"`). Lets erasure treat a qualified path as one opaque
/// unit instead of mistaking its trailing segment for a bare type name —
/// `crate::laws::Finding` is one type, not three identifiers.
fn coalesce_paths(tokens: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let (tok, is_ident) = &tokens[i];
        if !is_ident {
            out.push((tok.clone(), false));
            i += 1;
            continue;
        }
        let mut path = tok.clone();
        let mut j = i + 1;
        while j + 1 < tokens.len() && tokens[j].0 == "::" && tokens[j + 1].1 {
            path.push_str("::");
            path.push_str(&tokens[j + 1].0);
            j += 2;
        }
        out.push((path, true));
        i = j;
    }
    out
}

/// Generic-erase one `pub fn` signature (no body, as produced by
/// `extract_signatures`): every project-specific type identifier
/// (uppercase-leading, not prelude/primitive, not the function name) becomes
/// a fresh generic parameter, consistently within this signature — the same
/// source type always maps to the same generic, preserving whether two
/// parameters share a type. Returns a self-contained, generic-parameterized
/// signature with a `todo!()` body: compilable on its ownership/borrowing
/// shape alone, with concrete types erased. `None` if `sig` is not a
/// `pub fn` line.
///
/// Known limit (heuristic, not a parser): a signature that already declares
/// its own generics/lifetimes (`pub fn f<'a>(...)`) is skipped (`None`)
/// rather than lifted — appending an erasure-generated `<T0>` after an
/// existing `<'a>` produces invalid syntax (`f<'a><T0>`), and this function
/// cannot reliably merge the two lists, so it does not try. The signature
/// still appears in `signatures`; it just isn't part of `canonical_form`.
/// `where`-clauses are not specially handled and may still fail to compile —
/// that surfaces as a finding like any other.
fn lift_fn_signature(sig: &str) -> Option<String> {
    let rest = sig.strip_prefix("pub fn ")?;
    let name_end = rest.find('(')?;
    let name = &rest[..name_end];
    if name.contains('<') {
        return None;
    }
    // A multi-line declaration truncates to an unclosed `(` (and possibly an
    // unclosed `<...>` from a multi-line where-clause/generic list) — lifting
    // it as-is would corrupt the whole shadow with an unclosed-delimiter
    // error. Skip rather than emit unbalanced syntax. The `->` return-type
    // arrow is stripped first — its `>` is not a generic close bracket.
    let without_arrows = rest.replace("->", "");
    let balanced = |open: char, close: char| without_arrows.matches(open).count() == without_arrows.matches(close).count();
    if !balanced('(', ')') || !balanced('<', '>') {
        return None;
    }
    let params_and_ret = &rest[name_end..];

    let mut erased = String::new();
    let mut generics: Vec<String> = Vec::new();
    let coalesced = coalesce_paths(tokenize(params_and_ret));
    let mut i = 0;
    while i < coalesced.len() {
        let (tok, is_ident) = &coalesced[i];
        let should_erase = if !is_ident {
            false
        } else if let Some(first_segment) = tok.split("::").next().filter(|_| tok.contains("::")) {
            // `std::`/`core::` resolve by absolute path in any crate, no
            // `use` required; any other qualified path (another crate, or
            // this project's own `crate::`/`self::`) cannot resolve in the
            // bare scratch crate, so the whole path is erased as one unit.
            !matches!(first_segment, "std" | "core")
        } else {
            tok.starts_with(|c: char| c.is_ascii_uppercase()) && !is_prelude_or_primitive(tok)
        };
        if should_erase {
            let idx = generics.iter().position(|g| g == tok).unwrap_or_else(|| {
                generics.push(tok.clone());
                generics.len() - 1
            });
            erased.push_str(&format!("T{idx}"));
            i += 1;
            // Drop the erased type's own generic-argument span, if any:
            // `BTreeMap<String, String>` erases to `T0`, not `T0<String,
            // String>` — a bare generic parameter cannot itself take type
            // arguments. Depth-tracked to handle nesting. The closing `>`
            // may be glued to unrelated text in the same non-ident run (e.g.
            // `String>)`  tokenizes as one run) — when depth closes mid-token,
            // only the consumed prefix is dropped; anything after the
            // closing `>` in that same token (the `)` here) is re-emitted.
            if i < coalesced.len() && !coalesced[i].1 && coalesced[i].0.contains('<') {
                let mut depth = 0i32;
                'skip: while i < coalesced.len() {
                    let (t, ident) = coalesced[i].clone();
                    if ident {
                        i += 1;
                        continue;
                    }
                    for (byte_idx, ch) in t.char_indices() {
                        match ch {
                            '<' => depth += 1,
                            '>' => {
                                depth -= 1;
                                if depth <= 0 {
                                    erased.push_str(&t[byte_idx + ch.len_utf8()..]);
                                    i += 1;
                                    break 'skip;
                                }
                            }
                            _ => {}
                        }
                    }
                    i += 1;
                }
            }
        } else {
            erased.push_str(tok);
            i += 1;
        }
    }

    let generic_list =
        if generics.is_empty() { String::new() } else { format!("<{}>", (0..generics.len()).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", ")) };
    Some(format!("pub fn {name}{generic_list}{erased} {{ todo!() }}"))
}

/// Lift a file's extracted `pub fn` signatures into one self-contained Rust
/// program: concrete project-specific types are generic-erased per
/// `lift_fn_signature`, preserving each signature's ownership/borrowing
/// shape. This *is* the artifact worth keeping, not just an intermediate
/// step — a per-module, language-uniform rendering of "what this module's
/// public contract owns and borrows," dense enough to stand in for the
/// source. Running it through the existing `oxidize::oxidize()` is how real
/// rustc-grade findings (lifetime ambiguity, trait bound failures, anything
/// rustc itself would catch) get attached to that module's graph entry.
/// Non-function signatures (struct/enum/trait/type) are not lifted in V1 —
/// their declarations are recorded as plain signature facts, not compiled.
pub fn lift_shadow(signatures: &[String]) -> String {
    let bodies: Vec<String> = signatures.iter().filter_map(|s| lift_fn_signature(s)).collect();
    format!("#![allow(unused, dead_code)]\n\n{}\n", bodies.join("\n\n"))
}
