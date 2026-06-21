use super::*;
use serde_json::json;

#[test]
fn injects_when_no_system() {
    let mut req = json!({"model":"mimo-auto","messages":[{"role":"user","content":"hi"}]});
    inject_whitelist_system(&mut req);
    let m = req["messages"].as_array().unwrap();
    assert_eq!(m[0]["role"], "system");
    assert!(
        m[0]["content"]
            .as_str()
            .unwrap()
            .starts_with(WHITELIST_PREFIX)
    );
    assert_eq!(m[1]["role"], "user");
}

#[test]
fn prepends_when_system_present() {
    let mut req =
        json!({"messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}]});
    inject_whitelist_system(&mut req);
    let c = req["messages"][0]["content"].as_str().unwrap();
    assert!(c.starts_with(WHITELIST_PREFIX));
    assert!(c.ends_with("be brief"));
}

#[test]
fn idempotent_when_already_whitelisted() {
    let mut req = json!({"messages":[{"role":"system","content":WHITELIST_PREFIX}]});
    inject_whitelist_system(&mut req);
    assert_eq!(
        req["messages"][0]["content"].as_str().unwrap(),
        WHITELIST_PREFIX
    );
}

#[test]
fn prepends_text_part_for_array_content() {
    let mut req = json!({"messages":[
        {"role":"system","content":[{"type":"text","text":"be brief"}]},
        {"role":"user","content":"hi"}
    ]});
    inject_whitelist_system(&mut req);
    let parts = req["messages"][0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["type"], "text");
    assert!(
        parts[0]["text"]
            .as_str()
            .unwrap()
            .starts_with(WHITELIST_PREFIX)
    );
    assert_eq!(parts[1]["text"], "be brief");
}

#[test]
fn idempotent_for_array_content() {
    let mut req = json!({"messages":[
        {"role":"system","content":[{"type":"text","text":"be brief"}]}
    ]});
    inject_whitelist_system(&mut req);
    inject_whitelist_system(&mut req);
    let parts = req["messages"][0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 2, "不应重复注入");
    assert!(
        parts[0]["text"]
            .as_str()
            .unwrap()
            .starts_with(WHITELIST_PREFIX)
    );
    assert_eq!(parts[1]["text"], "be brief");
}

#[test]
fn body_otherwise_passthrough_unchanged() {
    let mut req = json!({
        "model": "mimo/mimo-auto",
        "temperature": 0.7,
        "tools": [{"type":"function","function":{"name":"x"}}],
        "messages": [{"role":"user","content":"hi"}]
    });
    inject_whitelist_system(&mut req);
    assert_eq!(req["model"], "mimo/mimo-auto");
    assert_eq!(req["temperature"], 0.7);
    assert_eq!(req["tools"][0]["function"]["name"], "x");
    assert_eq!(req["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn strips_hop_by_hop_response_headers() {
    assert!(should_strip_response_header("connection"));
    assert!(should_strip_response_header("transfer-encoding"));
    assert!(should_strip_response_header("content-length"));
    assert!(should_strip_response_header("content-encoding"));
    assert!(should_strip_response_header("access-control-allow-origin"));
    assert!(!should_strip_response_header("content-type"));
    assert!(!should_strip_response_header("x-request-id"));
}
