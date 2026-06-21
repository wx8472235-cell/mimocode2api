use super::*;
use serde_json::json;

#[test]
fn system_string_becomes_openai_system_message() {
    let req = json!({
        "model": "claude-x",
        "max_tokens": 100,
        "system": "be brief",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let oai = anthropic_to_openai(&req);
    let msgs = oai["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "be brief");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "hi");
    assert_eq!(oai["max_tokens"], 100);
    assert_eq!(oai["model"], "claude-x");
}

#[test]
fn system_array_is_concatenated() {
    let req = json!({
        "system": [{"type":"text","text":"a"},{"type":"text","text":"b"}],
        "messages": []
    });
    let oai = anthropic_to_openai(&req);
    assert_eq!(oai["messages"][0]["role"], "system");
    assert_eq!(oai["messages"][0]["content"], "a\nb");
}

#[test]
fn single_text_block_becomes_string() {
    let req = json!({
        "messages": [{"role":"user","content":[{"type":"text","text":"hello"}]}]
    });
    let oai = anthropic_to_openai(&req);
    assert_eq!(oai["messages"][0]["content"], "hello");
}

#[test]
fn image_base64_block_to_image_url() {
    let req = json!({
        "messages": [{"role":"user","content":[
            {"type":"text","text":"look"},
            {"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AAAA"}}
        ]}]
    });
    let oai = anthropic_to_openai(&req);
    let parts = oai["messages"][0]["content"].as_array().unwrap();
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(parts[1]["image_url"]["url"], "data:image/jpeg;base64,AAAA");
}

#[test]
fn image_url_block_to_image_url() {
    let req = json!({
        "messages": [{"role":"user","content":[
            {"type":"image","source":{"type":"url","url":"https://x/y.png"}}
        ]}]
    });
    let oai = anthropic_to_openai(&req);
    let parts = oai["messages"][0]["content"].as_array().unwrap();
    assert_eq!(parts[0]["image_url"]["url"], "https://x/y.png");
}

#[test]
fn assistant_tool_use_to_tool_calls() {
    let req = json!({
        "messages": [{"role":"assistant","content":[
            {"type":"text","text":"let me check"},
            {"type":"tool_use","id":"tu_1","name":"get_weather","input":{"city":"SF"}}
        ]}]
    });
    let oai = anthropic_to_openai(&req);
    let m = &oai["messages"][0];
    assert_eq!(m["role"], "assistant");
    assert_eq!(m["content"], "let me check");
    let calls = m["tool_calls"].as_array().unwrap();
    assert_eq!(calls[0]["id"], "tu_1");
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "get_weather");
    let args = calls[0]["function"]["arguments"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(args).unwrap();
    assert_eq!(parsed["city"], "SF");
}

#[test]
fn user_tool_result_expands_to_tool_message() {
    let msg = json!({"role":"user","content":[
        {"type":"tool_result","tool_use_id":"tu_1","content":"72F"}
    ]});
    let mut msgs = Vec::new();
    convert_message(&msg, &mut msgs);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "tool");
    assert_eq!(msgs[0]["tool_call_id"], "tu_1");
    assert_eq!(msgs[0]["content"], "72F");
}

#[test]
fn user_text_plus_tool_result_keeps_tool_then_text() {
    let req = json!({
        "messages": [{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"tu_1","content":[{"type":"text","text":"ok"}]},
            {"type":"text","text":"thanks"}
        ]}]
    });
    let oai = anthropic_to_openai(&req);
    let msgs = oai["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "tool");
    assert_eq!(msgs[0]["content"], "ok");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "thanks");
}

#[test]
fn tools_converted_to_openai_functions() {
    let req = json!({
        "messages": [],
        "tools": [{
            "name":"get_weather",
            "description":"get weather",
            "input_schema":{"type":"object","properties":{"city":{"type":"string"}}}
        }]
    });
    let oai = anthropic_to_openai(&req);
    let t = &oai["tools"][0];
    assert_eq!(t["type"], "function");
    assert_eq!(t["function"]["name"], "get_weather");
    assert_eq!(t["function"]["description"], "get weather");
    assert_eq!(t["function"]["parameters"]["type"], "object");
}

#[test]
fn tool_choice_variants() {
    assert_eq!(
        convert_tool_choice(&json!({"type":"auto"})).unwrap(),
        json!("auto")
    );
    assert_eq!(
        convert_tool_choice(&json!({"type":"any"})).unwrap(),
        json!("required")
    );
    assert_eq!(
        convert_tool_choice(&json!({"type":"none"})).unwrap(),
        json!("none")
    );
    assert_eq!(
        convert_tool_choice(&json!({"type":"tool","name":"f"})).unwrap(),
        json!({"type":"function","function":{"name":"f"}})
    );
}

#[test]
fn anthropic_only_fields_are_dropped() {
    let req = json!({
        "messages": [{"role":"user","content":"hi"}],
        "thinking": {"type":"adaptive"},
        "top_k": 5,
        "metadata": {"user_id":"x"}
    });
    let oai = anthropic_to_openai(&req);
    assert!(oai.get("thinking").is_none());
    assert!(oai.get("top_k").is_none());
    assert!(oai.get("metadata").is_none());
}

#[test]
fn stream_request_adds_include_usage() {
    let req = json!({"messages":[],"stream":true});
    let oai = anthropic_to_openai(&req);
    assert_eq!(oai["stream"], true);
    assert_eq!(oai["stream_options"]["include_usage"], true);
}

#[test]
fn whitelist_injected_after_conversion() {
    let req = json!({
        "system": "be brief",
        "messages": [{"role":"user","content":"hi"}]
    });
    let mut oai = anthropic_to_openai(&req);
    crate::proxy::inject_whitelist_system(&mut oai);
    let sys = oai["messages"][0]["content"].as_str().unwrap();
    assert!(sys.starts_with(crate::proxy::WHITELIST_PREFIX));
    assert!(sys.contains("be brief"));
}

#[test]
fn whitelist_injected_when_no_system() {
    let req = json!({"messages":[{"role":"user","content":"hi"}]});
    let mut oai = anthropic_to_openai(&req);
    crate::proxy::inject_whitelist_system(&mut oai);
    assert_eq!(oai["messages"][0]["role"], "system");
    assert!(
        oai["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with(crate::proxy::WHITELIST_PREFIX)
    );
}

#[test]
fn nonstream_text_response() {
    let oai = json!({
        "choices":[{"message":{"role":"assistant","content":"hello there"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":10,"completion_tokens":5}
    });
    let a = openai_to_anthropic(&oai, "claude-x");
    assert_eq!(a["type"], "message");
    assert_eq!(a["role"], "assistant");
    assert_eq!(a["model"], "claude-x");
    assert_eq!(a["content"][0]["type"], "text");
    assert_eq!(a["content"][0]["text"], "hello there");
    assert_eq!(a["stop_reason"], "end_turn");
    assert_eq!(a["usage"]["input_tokens"], 10);
    assert_eq!(a["usage"]["output_tokens"], 5);
    assert!(a["id"].as_str().unwrap().starts_with("msg_"));
}

#[test]
fn nonstream_tool_call_response() {
    let oai = json!({
        "choices":[{"message":{"role":"assistant","content":null,"tool_calls":[
            {"id":"call_1","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}
        ]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":3,"completion_tokens":4}
    });
    let a = openai_to_anthropic(&oai, "m");
    assert_eq!(a["stop_reason"], "tool_use");
    let block = &a["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "call_1");
    assert_eq!(block["name"], "f");
    assert_eq!(block["input"]["a"], 1);
}

#[test]
fn stop_reason_mapping() {
    assert_eq!(map_stop_reason(Some("stop")), "end_turn");
    assert_eq!(map_stop_reason(Some("length")), "max_tokens");
    assert_eq!(map_stop_reason(Some("tool_calls")), "tool_use");
    assert_eq!(map_stop_reason(Some("content_filter")), "end_turn");
    assert_eq!(map_stop_reason(None), "end_turn");
}

fn run_stream(lines: &[&str]) -> String {
    let mut conv = StreamConverter::new("msg_test".into(), "claude-x".into());
    let mut out = String::new();
    for e in conv.start() {
        out.push_str(&e);
    }
    for line in lines {
        for e in conv.push_line(line) {
            out.push_str(&e);
        }
    }
    for e in conv.finish() {
        out.push_str(&e);
    }
    out
}

#[test]
fn stream_text_sequence() {
    let out = run_stream(&[
        r#"data: {"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"lo"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}"#,
        "data: [DONE]",
    ]);
    assert!(out.contains("event: message_start"));
    assert!(out.contains("event: content_block_start"));
    assert!(out.contains(r#""type":"text""#));
    assert!(out.contains(r#""text":"Hel""#));
    assert!(out.contains(r#""text":"lo""#));
    assert!(out.contains("event: content_block_stop"));
    assert!(out.contains("event: message_delta"));
    assert!(out.contains(r#""stop_reason":"end_turn""#));
    assert!(out.contains(r#""output_tokens":3"#));
    assert!(out.contains("event: message_stop"));
    assert!(out.find("message_start").unwrap() < out.find("content_block_start").unwrap());
    assert!(out.find("message_delta").unwrap() < out.find("message_stop").unwrap());
}

#[test]
fn stream_tool_use_sequence() {
    let out = run_stream(&[
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SF\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
    ]);
    assert!(out.contains(r#""type":"tool_use""#));
    assert!(out.contains(r#""name":"get_weather""#));
    assert!(out.contains(r#""id":"call_1""#));
    assert!(out.contains(r#""type":"input_json_delta""#));
    assert!(out.contains(r#""partial_json":"{\"city\":""#));
    assert!(out.contains(r#""stop_reason":"tool_use""#));
}

#[test]
fn stream_text_after_tool_opens_fresh_text_block() {
    let out = run_stream(&[
        r#"data: {"choices":[{"delta":{"content":"A"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"B"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
    ]);
    assert_eq!(out.matches(r#""content_block":{"type":"text""#).count(), 2);
    assert!(out.contains(r#""index":2,"delta":{"type":"text_delta","text":"B"}"#));
}

#[test]
fn stream_push_line_tolerates_padding() {
    let mut conv = StreamConverter::new("id".into(), "m".into());
    let evts = conv.push_line(r#"data:  {"choices":[{"delta":{"content":"x"}}]}  "#);
    assert!(evts.iter().any(|e| e.contains(r#""text":"x""#)));
}
