use std::io::Cursor;
use tiny_http::{Header, Response, StatusCode};

pub fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

pub fn respond_json<T: serde::Serialize>(request: tiny_http::Request, status: u16, body: &T) {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let response = Response::new(
        StatusCode(status),
        vec![json_header()],
        Cursor::new(json.clone()),
        Some(json.len()),
        None,
    );
    let _ = request.respond(response);
}

pub fn respond_error(request: tiny_http::Request, status: u16, message: &str) {
    let body = serde_json::json!({ "error": message });
    respond_json(request, status, &body);
}
