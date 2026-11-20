//! Small loopback transport for the same attachment-scoped API used by MCP.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use apr_runtime::service::ServiceClient;
use serde_json::{Value, json};

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
const HTML: &str = include_str!("../assets/inspector.html");
const JAVASCRIPT: &[u8] = include_bytes!("../assets/inspector.js");
const CSS: &[u8] = include_bytes!("../assets/inspector.css");
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";

struct Context {
    client: ServiceClient,
    session: String,
    authority: String,
    origin: String,
}
struct ConnectionCount(Arc<AtomicUsize>);
impl Drop for ConnectionCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn serve(attachment: &Path, port: u16) -> apr_runtime::Result<()> {
    let client = ServiceClient::from_file(attachment)?;
    let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), port))?;
    let authority = listener.local_addr()?.to_string();
    let origin = format!("http://{authority}");
    let context = Arc::new(Context {
        client,
        authority,
        origin,
        session: uuid::Uuid::new_v4().simple().to_string(),
    });
    crate::write_json(
        json!({"url":format!("{}/#session={}", context.origin, context.session),"scope":"attached_assignment"}),
    )?;
    io::stdout().flush()?;
    let active = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < 8).then_some(value + 1)
            })
            .is_err()
        {
            drop(stream);
            continue;
        }
        let guard = ConnectionCount(active.clone());
        let context = context.clone();
        let _ = std::thread::Builder::new()
            .name("apr-inspector-http".into())
            .spawn(move || {
                let _guard = guard;
                handle(stream, &context);
            });
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn handle(mut stream: TcpStream, context: &Context) {
    let response = match read_request(&mut stream) {
        Ok(request) => route(request, context),
        Err(status) => Response::error(status, "INVALID_HTTP_REQUEST"),
    };
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        _ => "Conflict",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: {CSP}\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nCross-Origin-Resource-Policy: same-origin\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    let _ = stream.write_all(&response.body);
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}
impl Response {
    fn asset(content_type: &'static str, body: &[u8]) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.to_vec(),
        }
    }
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }
    fn error(status: u16, code: &str) -> Self {
        Self::json(
            status,
            json!({"error":{"code":code,"message":"The request could not be completed."}}),
        )
    }
}

fn route(request: Request, context: &Context) -> Response {
    if request.headers.get("host") != Some(&context.authority)
        || request
            .headers
            .get("origin")
            .is_some_and(|origin| origin != &context.origin)
    {
        return Response::error(403, "FORBIDDEN");
    }
    if request.method == "GET" && request.body.is_empty() {
        return match request.path.as_str() {
            "/" => Response::asset(
                "text/html; charset=utf-8",
                HTML.replace(
                    "%APR_MODE%",
                    if context.client.mode() == "author" {
                        "author"
                    } else {
                        "run"
                    },
                )
                .as_bytes(),
            ),
            "/inspector.js" => Response::asset("text/javascript; charset=utf-8", JAVASCRIPT),
            "/inspector.css" => Response::asset("text/css; charset=utf-8", CSS),
            _ => Response::error(404, "NOT_FOUND"),
        };
    }
    if request.path != "/api" {
        return Response::error(404, "NOT_FOUND");
    }
    if request.method != "POST" {
        return Response::error(405, "METHOD_NOT_ALLOWED");
    }
    if request.headers.get("origin") != Some(&context.origin)
        || !request
            .headers
            .get("x-apr-session")
            .is_some_and(|token| equal_token(token, &context.session))
        || request.headers.get("content-type").map(String::as_str) != Some("application/json")
    {
        return Response::error(403, "FORBIDDEN");
    }
    let Ok(Value::Object(mut value)) = serde_json::from_slice(&request.body) else {
        return Response::error(400, "INVALID_INPUT");
    };
    if value
        .keys()
        .any(|key| !["action", "args", "detail"].contains(&key.as_str()))
    {
        return Response::error(400, "INVALID_INPUT");
    }
    let Some(Value::String(action)) = value.remove("action") else {
        return Response::error(400, "INVALID_INPUT");
    };
    let Some(args @ Value::Object(_)) = value.remove("args") else {
        return Response::error(400, "INVALID_INPUT");
    };
    let budget = match value.remove("detail") {
        None | Some(Value::Bool(false)) => 8192,
        Some(Value::Bool(true)) => MAX_BODY,
        _ => return Response::error(400, "INVALID_INPUT"),
    };
    if !["discover", "run", "inspect", "resume", "manage"].contains(&action.as_str())
        || (action == "manage" && context.client.mode() != "author")
    {
        return Response::error(403, "FORBIDDEN");
    }
    match context.client.call(&action, args, budget) {
        Ok(value) => Response::json(200, json!({"result":value})),
        Err(error) => {
            let code = error.code();
            let code = if code.len() <= 64
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            {
                code
            } else {
                "INTERNAL_ERROR"
            };
            Response::error(
                if ["FORBIDDEN", "UNAUTHORIZED"].contains(&code) {
                    403
                } else {
                    409
                },
                code,
            )
        }
    }
}

fn equal_token(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |value, (left, right)| value | (left ^ right))
            == 0
}

fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    stream.set_nonblocking(true).map_err(|_| 400_u16)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() >= MAX_HEADERS {
            return Err(413);
        }
        read_more(stream, &mut bytes, deadline)?;
    };
    if header_end > MAX_HEADERS {
        return Err(413);
    }
    let header = std::str::from_utf8(&bytes[..header_end - 4]).map_err(|_| 400_u16)?;
    let mut lines = header.split("\r\n");
    let mut start = lines.next().ok_or(400_u16)?.split(' ');
    let method = start.next().ok_or(400_u16)?.to_string();
    let path = start.next().ok_or(400_u16)?.to_string();
    if start.next() != Some("HTTP/1.1")
        || start.next().is_some()
        || path.len() > 128
        || !path.starts_with('/')
    {
        return Err(400);
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte.is_ascii_control())
            || headers.len() >= 64
        {
            return Err(400);
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return Err(400);
        }
    }
    if !headers.contains_key("host")
        || headers.contains_key("transfer-encoding")
        || headers.contains_key("expect")
    {
        return Err(400);
    }
    let length = match headers.get("content-length") {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| 413_u16)?
        }
        Some(_) => return Err(400),
        None if method == "GET" => 0,
        None => return Err(400),
    };
    if length > MAX_BODY {
        return Err(413);
    }
    while bytes.len() < header_end + length {
        read_more(stream, &mut bytes, deadline)?;
    }
    if bytes.len() != header_end + length {
        return Err(400);
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

fn read_more(stream: &mut TcpStream, bytes: &mut Vec<u8>, deadline: Instant) -> Result<(), u16> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(408);
        };
        let mut buffer = [0_u8; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => return Err(400),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let mut poll = libc::pollfd {
                    fd: stream.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: one live stream descriptor and initialized poll entry.
                let result =
                    unsafe { libc::poll(&mut poll, 1, remaining.as_millis().max(1) as i32) };
                if result == 0 {
                    return Err(408);
                }
                if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    return Err(400);
                }
            }
            Err(_) => return Err(400),
        }
    }
}
