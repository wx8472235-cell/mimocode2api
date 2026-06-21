use super::*;

const HEX_START: usize = 4;
const HEX_END: usize = 16;

#[test]
fn format_matches_spec() {
    let s = gen_session_affinity(1_750_000_000_000);
    assert!(s.starts_with("ses_"), "实际为 {s}");
    assert_eq!(s.len(), 30, "实际为 {s}");
    let hex = &s[HEX_START..HEX_END];
    let b62 = &s[HEX_END..];
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "hex 必须为小写,实际为 {hex}"
    );
    assert!(
        b62.chars().all(|c| c.is_ascii_alphanumeric()),
        "suffix 必须为 base62,实际为 {b62}"
    );
}

#[test]
fn last_three_hex_are_ffe() {
    for ms in [1i64, 1_000, 1_750_000_000_000, 9_999_999_999_999] {
        let s = gen_session_affinity(ms);
        let hex = &s[HEX_START..HEX_END];
        assert!(hex.ends_with("ffe"), "ms={ms} hex={hex}");
    }
}

#[test]
fn hex_prefix_is_deterministic() {
    let a = gen_session_affinity(1_750_000_000_000);
    let b = gen_session_affinity(1_750_000_000_000);
    assert_eq!(&a[..HEX_END], &b[..HEX_END]);
    assert_ne!(&a[HEX_END..], &b[HEX_END..]);
}
