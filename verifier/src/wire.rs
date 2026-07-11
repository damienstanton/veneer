//! ASCII armor for the TOON wire format, shared by `graph` and `state`.
//!
//! Percent-encodes any byte ≥ 0x80 (plus the `%` marker itself, for an
//! unambiguous round-trip) so free text is pure ASCII before it reaches
//! toon_rust. Necessary: toon-rust 0.1.3's decoder miscomputes offsets across
//! multi-byte UTF-8 characters — a single em dash corrupts parsing of every
//! subsequent line in the document, and some multi-byte sequences (e.g. CJK
//! text) make it panic on a char-boundary slice outright. Plain ASCII
//! (including embedded `\n`, which toon_rust's own escaping already
//! round-trips correctly) is left untouched. Confined to storage — all
//! public shapes and agent-facing JSON carry normal UTF-8; only the wire
//! format is percent-encoded.

pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b == b'%' || b >= 0x80 {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

pub(crate) fn decode(s: &str) -> String {
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
        assert_eq!(encode("100%"), "100%25");
        assert_eq!(decode("100%25"), "100%");
    }

    #[test]
    fn decode_leaves_malformed_escapes_untouched() {
        assert_eq!(decode("%GG"), "%GG");
        assert_eq!(decode("%2"), "%2");
        assert_eq!(decode("%"), "%");
    }

    #[test]
    fn plain_ascii_passes_through_both_ways() {
        assert_eq!(encode("plain: text\n"), "plain: text\n");
        assert_eq!(decode("plain: text\n"), "plain: text\n");
    }

    proptest! {
        #[test]
        fn roundtrip_is_identity(s in any::<String>()) {
            prop_assert_eq!(decode(&encode(&s)), s);
        }
    }
}
