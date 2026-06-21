use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use crate::clock;
use crate::proxy::{AppState, inject_whitelist_system};
use crate::session;

pub async fn messages(State(state): State<AppState>, body: Bytes) -> Response {
    let anthropic_req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &e.to_string(),
            );
        }
    };

    let model = anthropic_req
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("claude")
        .to_string();
    let stream = anthropic_req
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut openai_req = anthropic_to_openai(&anthropic_req);
    inject_whitelist_system(&mut openai_req);

    let body_bytes = match serde_json::to_vec(&openai_req) {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                &e.to_string(),
            );
        }
    };

    let session_id = session::gen_session_affinity(clock::now_ms());
    tracing::debug!(%session_id, stream, "正在将 anthropic /v1/messages 转发到上游");

    let upstream_resp = match state
        .upstream
        .chat(Bytes::from(body_bytes), &session_id)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("上游错误: {e:#}");
            return error_response(StatusCode::BAD_GATEWAY, "api_error", &format!("{e:#}"));
        }
    };

    let status = upstream_resp.status();
    if !status.is_success() {
        let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let text = upstream_resp.text().await.unwrap_or_default();
        return error_response(code, "api_error", &format!("上游返回 {status}: {text}"));
    }

    if stream {
        stream_response(upstream_resp, model)
    } else {
        nonstream_response(upstream_resp, model).await
    }
}

pub async fn count_tokens(body: Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &e.to_string(),
            );
        }
    };

    let mut chars = 0usize;
    if let Some(sys) = req.get("system") {
        chars += estimate_chars(sys);
    }
    if let Some(arr) = req.get("messages").and_then(|m| m.as_array()) {
        for msg in arr {
            if let Some(c) = msg.get("content") {
                chars += estimate_chars(c);
            }
        }
    }
    let tokens = (chars / 4).max(1) as i64;
    (
        StatusCode::OK,
        axum::Json(json!({ "input_tokens": tokens })),
    )
        .into_response()
}

async fn nonstream_response(upstream: reqwest::Response, model: String) -> Response {
    let openai: Value = match upstream.json().await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, "api_error", &e.to_string()),
    };
    let anthropic = openai_to_anthropic(&openai, &model);
    (StatusCode::OK, axum::Json(anthropic)).into_response()
}

fn stream_response(upstream: reqwest::Response, model: String) -> Response {
    let id = gen_message_id();
    let body = Body::from_stream(async_stream::stream! {
        let mut conv = StreamConverter::new(id, model);
        for evt in conv.start() {
            yield Ok::<Bytes, std::io::Error>(Bytes::from(evt));
        }

        let mut stream = Box::pin(upstream.bytes_stream());
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("上游流读取错误: {e}");
                    break;
                }
            };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                for evt in conv.push_line(line.as_ref()) {
                    yield Ok(Bytes::from(evt));
                }
            }
        }
        if !buf.is_empty() {
            let line = String::from_utf8_lossy(&buf);
            for evt in conv.push_line(line.as_ref()) {
                yield Ok(Bytes::from(evt));
            }
        }

        for evt in conv.finish() {
            yield Ok(Bytes::from(evt));
        }
    });

    let mut resp = Response::new(body);
    resp.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/event-stream"),
    );
    resp.headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-cache"));
    resp
}

fn error_response(status: StatusCode, err_type: &str, message: &str) -> Response {
    let body = json!({
        "type": "error",
        "error": { "type": err_type, "message": message }
    });
    (status, axum::Json(body)).into_response()
}

fn anthropic_to_openai(req: &Value) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(sys) = req.get("system") {
        let text = collect_text(sys);
        if !text.is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }

    if let Some(arr) = req.get("messages").and_then(|m| m.as_array()) {
        for msg in arr {
            convert_message(msg, &mut messages);
        }
    }

    let mut out = Map::new();
    if let Some(model) = req.get("model") {
        out.insert("model".into(), model.clone());
    }
    out.insert("messages".into(), Value::Array(messages));
    if let Some(v) = req.get("max_tokens") {
        out.insert("max_tokens".into(), v.clone());
    }
    if let Some(v) = req.get("temperature") {
        out.insert("temperature".into(), v.clone());
    }
    if let Some(v) = req.get("top_p") {
        out.insert("top_p".into(), v.clone());
    }
    if let Some(v) = req.get("stop_sequences") {
        out.insert("stop".into(), v.clone());
    }
    if let Some(stream) = req.get("stream").and_then(|v| v.as_bool()) {
        out.insert("stream".into(), Value::Bool(stream));
        if stream {
            out.insert("stream_options".into(), json!({ "include_usage": true }));
        }
    }
    if let Some(tools) = req.get("tools").and_then(|t| t.as_array()) {
        let converted: Vec<Value> = tools.iter().map(convert_tool).collect();
        if !converted.is_empty() {
            out.insert("tools".into(), Value::Array(converted));
        }
    }
    if let Some(tc) = req.get("tool_choice")
        && let Some(v) = convert_tool_choice(tc)
    {
        out.insert("tool_choice".into(), v);
    }

    Value::Object(out)
}

fn collect_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn convert_message(msg: &Value, out: &mut Vec<Value>) {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
    let content = msg.get("content");

    if let Some(s) = content.and_then(|c| c.as_str()) {
        out.push(json!({ "role": role, "content": s }));
        return;
    }

    let empty = Vec::new();
    let blocks = content.and_then(|c| c.as_array()).unwrap_or(&empty);

    let mut parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut tool_results: Vec<Value> = Vec::new();

    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    parts.push(json!({ "type": "text", "text": t }));
                }
            }
            Some("image") => {
                if let Some(url) = image_url(block) {
                    parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
            }
            Some("tool_use") => {
                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let args = serde_json::to_string(&input).unwrap_or_else(|_| "{}".into());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args }
                }));
            }
            Some("tool_result") => {
                let id = block
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let text = block.get("content").map(collect_text).unwrap_or_default();
                tool_results.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": text
                }));
            }
            _ => {}
        }
    }

    if role == "assistant" {
        let mut m = Map::new();
        m.insert("role".into(), Value::String("assistant".into()));
        m.insert("content".into(), collapse_parts(parts, Value::Null));
        if !tool_calls.is_empty() {
            m.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        out.push(Value::Object(m));
        return;
    }

    let pushed_tools = !tool_results.is_empty();
    for tr in tool_results {
        out.push(tr);
    }
    if !parts.is_empty() {
        let content = collapse_parts(parts, Value::String(String::new()));
        out.push(json!({ "role": role, "content": content }));
    } else if !pushed_tools {
        out.push(json!({ "role": role, "content": "" }));
    }
}

fn collapse_parts(parts: Vec<Value>, empty: Value) -> Value {
    if parts.is_empty() {
        empty
    } else if parts.len() == 1 && parts[0].get("type").and_then(|t| t.as_str()) == Some("text") {
        parts[0].get("text").cloned().unwrap_or(empty)
    } else {
        Value::Array(parts)
    }
}

fn image_url(block: &Value) -> Option<String> {
    let source = block.get("source")?;
    match source.get("type").and_then(|t| t.as_str()) {
        Some("base64") => {
            let media = source
                .get("media_type")
                .and_then(|m| m.as_str())
                .unwrap_or("image/png");
            let data = source.get("data").and_then(|d| d.as_str())?;
            Some(format!("data:{media};base64,{data}"))
        }
        Some("url") => source.get("url").and_then(|u| u.as_str()).map(String::from),
        _ => None,
    }
}

fn convert_tool(tool: &Value) -> Value {
    let mut func = Map::new();
    func.insert(
        "name".into(),
        tool.get("name")
            .cloned()
            .unwrap_or(Value::String(String::new())),
    );
    if let Some(desc) = tool.get("description") {
        func.insert("description".into(), desc.clone());
    }
    if let Some(schema) = tool.get("input_schema") {
        func.insert("parameters".into(), schema.clone());
    }
    json!({ "type": "function", "function": Value::Object(func) })
}

fn convert_tool_choice(tc: &Value) -> Option<Value> {
    match tc.get("type").and_then(|t| t.as_str()) {
        Some("auto") => Some(json!("auto")),
        Some("any") => Some(json!("required")),
        Some("none") => Some(json!("none")),
        Some("tool") => {
            let name = tc.get("name").and_then(|n| n.as_str())?;
            Some(json!({ "type": "function", "function": { "name": name } }))
        }
        _ => None,
    }
}

fn openai_to_anthropic(resp: &Value, model: &str) -> Value {
    let choice = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());
    let message = choice.and_then(|c| c.get("message"));
    let finish = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|f| f.as_str());

    let mut content: Vec<Value> = Vec::new();
    if let Some(text) = message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        && !text.is_empty()
    {
        content.push(json!({ "type": "text", "text": text }));
    }
    if let Some(calls) = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(|c| c.as_array())
    {
        for call in calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let func = call.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args_str = func
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
            content.push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
        }
    }

    let usage = resp.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    json!({
        "id": gen_message_id(),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": map_stop_reason(finish),
        "stop_sequence": null,
        "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens }
    })
}

fn map_stop_reason(finish: Option<&str>) -> &'static str {
    match finish {
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        _ => "end_turn",
    }
}

fn gen_message_id() -> String {
    format!("msg_{}", crate::upstream::random_hex(16))
}

fn estimate_chars(content: &Value) -> usize {
    match content {
        Value::String(s) => s.chars().count(),
        Value::Array(arr) => arr
            .iter()
            .map(|b| match b {
                Value::String(s) => s.chars().count(),
                _ => b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.chars().count())
                    .or_else(|| b.get("content").map(estimate_chars))
                    .unwrap_or(0),
            })
            .sum(),
        _ => 0,
    }
}

enum OpenBlock {
    Text(usize),
    Tool(usize),
}

struct StreamConverter {
    id: String,
    model: String,
    next_index: usize,
    tool_index_map: HashMap<i64, usize>,
    open_block: Option<OpenBlock>,
    finish_reason: Option<String>,
    output_tokens: i64,
}

impl StreamConverter {
    fn new(id: String, model: String) -> Self {
        Self {
            id,
            model,
            next_index: 0,
            tool_index_map: HashMap::new(),
            open_block: None,
            finish_reason: None,
            output_tokens: 0,
        }
    }

    fn start(&mut self) -> Vec<String> {
        let evt = json!({
            "type": "message_start",
            "message": {
                "id": self.id,
                "type": "message",
                "role": "assistant",
                "model": self.model,
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": { "input_tokens": 0, "output_tokens": 0 }
            }
        });
        vec![sse("message_start", &evt)]
    }

    fn push_line(&mut self, line: &str) -> Vec<String> {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            return vec![];
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return vec![];
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        self.push_chunk(&chunk)
    }

    fn push_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut out = Vec::new();

        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null())
            && let Some(c) = usage.get("completion_tokens").and_then(|v| v.as_i64())
        {
            self.output_tokens = c;
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
        else {
            return out;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(|c| c.as_str())
                && !text.is_empty()
            {
                let (events, idx) = self.ensure_text_open();
                out.extend(events);
                out.push(sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": idx,
                        "delta": { "type": "text_delta", "text": text }
                    }),
                ));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                for call in calls {
                    out.extend(self.handle_tool_call(call));
                }
            }
        }

        if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            self.finish_reason = Some(fr.to_string());
        }

        out
    }

    fn ensure_text_open(&mut self) -> (Vec<String>, usize) {
        if let Some(OpenBlock::Text(idx)) = self.open_block {
            return (vec![], idx);
        }
        let mut out = self.close_open_block();
        let idx = self.next_index;
        self.next_index += 1;
        self.open_block = Some(OpenBlock::Text(idx));
        out.push(sse(
            "content_block_start",
            &json!({
                "type": "content_block_start",
                "index": idx,
                "content_block": { "type": "text", "text": "" }
            }),
        ));
        (out, idx)
    }

    fn handle_tool_call(&mut self, call: &Value) -> Vec<String> {
        let oai_idx = call.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut out = Vec::new();

        let block_idx = match self.tool_index_map.get(&oai_idx).copied() {
            Some(idx) => idx,
            None => {
                out.extend(self.close_open_block());
                let idx = self.next_index;
                self.next_index += 1;
                self.tool_index_map.insert(oai_idx, idx);
                self.open_block = Some(OpenBlock::Tool(idx));
                let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push(sse(
                    "content_block_start",
                    &json!({
                        "type": "content_block_start",
                        "index": idx,
                        "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} }
                    }),
                ));
                idx
            }
        };

        if let Some(args) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
            && !args.is_empty()
        {
            out.push(sse(
                "content_block_delta",
                &json!({
                    "type": "content_block_delta",
                    "index": block_idx,
                    "delta": { "type": "input_json_delta", "partial_json": args }
                }),
            ));
        }

        out
    }

    fn close_open_block(&mut self) -> Vec<String> {
        match self.open_block.take() {
            Some(OpenBlock::Text(idx)) | Some(OpenBlock::Tool(idx)) => vec![sse(
                "content_block_stop",
                &json!({ "type": "content_block_stop", "index": idx }),
            )],
            None => vec![],
        }
    }

    fn finish(&mut self) -> Vec<String> {
        let mut out = self.close_open_block();
        let stop_reason = map_stop_reason(self.finish_reason.as_deref());
        out.push(sse(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": { "output_tokens": self.output_tokens }
            }),
        ));
        out.push(sse("message_stop", &json!({ "type": "message_stop" })));
        out
    }
}

fn sse(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

#[cfg(test)]
#[path = "../tests/anthropic.rs"]
mod tests;
