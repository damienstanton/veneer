//! Property tests for the graph wire format and the lift: the mechanical
//! witnesses for the preservation/totality claims in spec/oxidation.md.

use proptest::prelude::*;
use std::collections::BTreeMap;
use veneer::graph::{Graph, GraphEntry};
use veneer::laws::{Finding, Law};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn graph_store_load_roundtrips_arbitrary_text(
        doc in any::<String>(),
        msg in any::<String>(),
        sig in any::<String>(),
        key in "[a-zA-Z0-9_./-]{1,24}",
        built_from in any::<u64>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = BTreeMap::new();
        entries.insert(key.clone(), GraphEntry {
            path: key,
            signatures: vec![sig],
            doc_summary: if doc.is_empty() { None } else { Some(doc) },
            loc: 1,
            complexity: 0,
            canonical_form: None,
            semantic_findings: vec![Finding::error(Law::Oxidation, "m.rs", None, &msg, None)],
        });
        let g = Graph { entries, built_from };
        veneer::graph::store(dir.path(), &g).unwrap();
        prop_assert_eq!(veneer::graph::load(dir.path()).unwrap(), g);
    }

    #[test]
    fn lift_shadow_is_total_and_deterministic(
        lines in proptest::collection::vec(any::<String>(), 0..8)
    ) {
        let a = veneer::graph::lift_shadow(&lines);
        let b = veneer::graph::lift_shadow(&lines);
        prop_assert_eq!(a, b);
    }
}
