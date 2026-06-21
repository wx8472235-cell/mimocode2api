use super::*;

#[test]
fn client_id_is_64_lowercase_hex() {
    let id = random_hex(32);
    assert!(is_valid_client(&id));
    assert_eq!(id.len(), 64);
}

#[test]
fn parses_jwt_exp() {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1700000000}"#);
    let jwt = format!("header.{payload}.sig");
    assert_eq!(jwt_exp_ms(&jwt), Some(1_700_000_000_000));
}

#[test]
fn invalid_client_rejected() {
    assert!(!is_valid_client("abc"));
    assert!(!is_valid_client(&"A".repeat(64)));
    assert!(!is_valid_client(&"z".repeat(64)));
    assert!(is_valid_client(&"a".repeat(64)));
}
