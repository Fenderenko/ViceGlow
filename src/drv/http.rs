//! A minimal HTTP/1.1 client for lamps on the local network (WLED, Nanoleaf, Hue): one request
//! per connection, JSON bodies, short timeouts.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

use serde_json::Value;

const TIMEOUT: Duration = Duration::from_millis(1500);

fn addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    (host, port).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or_else(|| format!("{host}: no address"))
}

/// Sends a request and returns (status, body).
pub fn request(host: &str, port: u16, method: &str, path: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let mut s = TcpStream::connect_timeout(&addr(host, port)?, TIMEOUT).map_err(|e| format!("{host}: {e}"))?;
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let _ = s.set_write_timeout(Some(TIMEOUT));
    let _ = s.set_nodelay(true);
    let b = body.unwrap_or("");
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    parse(&raw)
}

fn parse(raw: &[u8]) -> Result<(u16, String), String> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no HTTP answer")?;
    let status = head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).ok_or("bad HTTP status")?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { dechunk(body) } else { body.to_string() };
    Ok((status, body))
}

fn dechunk(mut b: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = b.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        b = rest[n..].trim_start_matches("\r\n");
    }
    out
}

/// A JSON request; non-2xx answers are errors.
pub fn json(host: &str, port: u16, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let b = body.map(|v| v.to_string());
    let (st, text) = request(host, port, method, path, b.as_deref())?;
    if !(200..300).contains(&st) {
        return Err(format!("HTTP {st}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("bad JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_chunked() {
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap(), (200, "{}".into()));
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n").unwrap().1, "{\"a\":1}");
    }
}
