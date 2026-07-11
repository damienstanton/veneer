//! ASCII armor for the TOON wire format, shared by `graph` and `state`.
//!
//! Reduces free text to a conservatively safe ASCII subset before it reaches
//! toon_rust, because toon-rust 0.1.3 mishandles many scalar shapes — its
//! decoder miscomputes offsets across multi-byte UTF-8 (one em dash corrupts
//! every later line; some sequences panic outright), empty strings vanish
//! from array/tabular encodings, literal double quotes corrupt document
//! syntax, digit-leading scalars are mistyped as numbers, and structural or
//! escape punctuation (`[`, `|`, `\`, …) corrupts or drops entries depending
//! on context. The rule is a whitelist: the empty string encodes to the
//! sentinel `%` (unambiguous — a real `%` byte encodes to `%25`, so a bare
//! `%` is otherwise impossible output); ASCII letters, digits, space, and
//! `_` pass through, except that a leading digit is escaped (letters-only
//! safety does not cover number mistyping: `0a` breaks even though both
//! bytes are whitelisted); every other byte escapes to uppercase `%XX`.
//! Decode is uniform percent-decoding, so old wire bytes stay decodable —
//! but only once a consumer has decided a document is actually armored:
//! `graph` and `state` each carry a wire `version` field and gate decoding on
//! it, because a pre-armor (version-less) document's raw bytes may contain a
//! `%XX`-shaped substring or a bare `%` that this decoder would misinterpret.
//! Confined to storage — all public shapes and agent-facing JSON carry
//! normal UTF-8; only the wire format is percent-encoded.

pub(crate) fn encode(s: &str) -> String {
    if s.is_empty() {
        return "%".to_string();
    }
    let mut out = String::with_capacity(s.len());
    for (i, b) in s.bytes().enumerate() {
        let safe = (b.is_ascii_alphanumeric() || b == b' ' || b == b'_')
            && !(i == 0 && b.is_ascii_digit());
        if safe {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

pub(crate) fn decode(s: &str) -> String {
    if s == "%" {
        return String::new();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex_byte = if bytes[i] == b'%' && i + 3 <= bytes.len() {
            std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        } else {
            None
        };
        match hex_byte {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    // A well-formed `%XX` escape can still decode to a byte sequence that
    // isn't valid UTF-8 (e.g. a hand-edited "%FF"). Rather than surface that
    // as its own error path, fall back to empty and let it fail naturally:
    // both callers (`graph`/`state` `load`) recompute a content hash over
    // the decoded result and compare it to the document's stored witness, so
    // this can only ever produce a hash mismatch — self-heal for the graph,
    // an explicit Protocol finding for state — never silent data loss.
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn multibyte_maps_to_percent_hex() {
        assert_eq!(encode("a—b"), "a%E2%80%94b");
    }

    #[test]
    fn percent_marker_escapes_for_unambiguous_roundtrip() {
        assert_eq!(encode("100%"), "%3100%25");
        assert_eq!(decode("%3100%25"), "100%");
        assert_eq!(decode("100%25"), "100%"); // old wire bytes still decode
    }

    #[test]
    fn empty_string_maps_to_bare_percent_sentinel() {
        assert_eq!(encode(""), "%");
        assert_eq!(decode("%"), "");
    }

    #[test]
    fn decode_leaves_malformed_escapes_untouched() {
        assert_eq!(decode("%GG"), "%GG");
        assert_eq!(decode("%2"), "%2");
    }

    #[test]
    fn plain_ascii_passes_through_both_ways() {
        assert_eq!(encode("plain text"), "plain text");
        assert_eq!(decode("plain text"), "plain text");
    }

    #[test]
    fn control_bytes_and_quotes_escape() {
        assert_eq!(encode("a\nb"), "a%0Ab");
        assert_eq!(encode("\""), "%22");
        assert_eq!(decode("a%0Ab"), "a\nb");
        assert_eq!(decode("%22"), "\"");
    }

    #[test]
    fn digit_like_leading_byte_escapes() {
        assert_eq!(encode("0A"), "%30A");
        assert_eq!(encode("42"), "%342"); // '4' escapes, '2' passes through
        assert_eq!(encode("-1a"), "%2D1a");
        for s in ["0A", "42", "-1a", "+1", ".5", "a0a"] {
            assert_eq!(decode(&encode(s)), s);
        }
        // Only the leading byte is digit-sensitive.
        assert_eq!(encode("a0a"), "a0a");
    }

    #[test]
    fn everything_outside_the_whitelist_escapes() {
        assert_eq!(encode("["), "%5B");
        assert_eq!(encode("a|b"), "a%7Cb");
        assert_eq!(encode("k: v"), "k%3A v");
        assert_eq!(encode("a\\b"), "a%5Cb");
        assert_eq!(encode("under_score ok"), "under_score ok");
        for s in ["[", "a|b", "k: v", "x[y]", "{a}", "a,b", "#tag", " \\", "fn f(x: u32) -> T"] {
            assert_eq!(decode(&encode(s)), s);
        }
    }

    proptest! {
        #[test]
        fn roundtrip_is_identity(s in any::<String>()) {
            prop_assert_eq!(decode(&encode(&s)), s);
        }
    }
}
