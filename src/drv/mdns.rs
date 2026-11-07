//! One-shot mDNS browsing: a PTR question with the unicast-response bit, answers collected for
//! a short time. Enough to find WLED, Nanoleaf and Hue bridges without a resolver service.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    /// instance label ("WLED-Desk")
    pub name: String,
    pub ip: Ipv4Addr,
    pub port: u16,
}

fn question(service: &str) -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for l in service.split('.').filter(|x| !x.is_empty()) {
        q.push(l.len() as u8);
        q.extend_from_slice(l.as_bytes());
    }
    q.extend([0, 0, 12, 0x80, 1]); // PTR, IN with "unicast response"
    q
}

/// Reads a (possibly compressed) name at `i`; returns it and the offset after it.
fn name(p: &[u8], mut i: usize) -> Option<(String, usize)> {
    let mut out = Vec::new();
    let mut end = None;
    for _ in 0..32 {
        let n = *p.get(i)? as usize;
        if n == 0 {
            return Some((out.join("."), end.unwrap_or(i + 1)));
        }
        if n & 0xC0 == 0xC0 {
            let off = ((n & 0x3F) << 8) | *p.get(i + 1)? as usize;
            end.get_or_insert(i + 2);
            i = off;
            continue;
        }
        out.push(String::from_utf8_lossy(p.get(i + 1..i + 1 + n)?).to_string());
        i += 1 + n;
    }
    None
}

#[derive(Default)]
struct Records {
    ptr: Vec<String>,
    srv: HashMap<String, (String, u16)>,
    a: HashMap<String, Ipv4Addr>,
}

fn parse(p: &[u8], r: &mut Records) -> Option<()> {
    let counts: Vec<usize> = (0..4).map(|k| u16::from_be_bytes([p[4 + 2 * k], p[5 + 2 * k]]) as usize).collect();
    let mut i = 12;
    for _ in 0..counts[0] {
        i = name(p, i)?.1 + 4;
    }
    for _ in 0..counts[1] + counts[2] + counts[3] {
        let (n, j) = name(p, i)?;
        let typ = u16::from_be_bytes([*p.get(j)?, *p.get(j + 1)?]);
        let len = u16::from_be_bytes([*p.get(j + 8)?, *p.get(j + 9)?]) as usize;
        let d = j + 10;
        match typ {
            12 => r.ptr.push(name(p, d)?.0),
            33 => {
                let port = u16::from_be_bytes([*p.get(d + 4)?, *p.get(d + 5)?]);
                r.srv.insert(n.to_lowercase(), (name(p, d + 6)?.0.to_lowercase(), port));
            }
            1 if len == 4 => {
                r.a.insert(n.to_lowercase(), Ipv4Addr::new(p[d], p[d + 1], p[d + 2], p[d + 3]));
            }
            _ => {}
        }
        i = d + len;
    }
    Some(())
}

/// Asks for `service` ("_wled._tcp.local") and collects answers for `wait`.
pub fn browse(service: &str, wait: Duration) -> Vec<Service> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return Vec::new() };
    let _ = sock.set_multicast_ttl_v4(255);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let q = question(service);
    let dst: SocketAddr = "224.0.0.251:5353".parse().unwrap();
    let mut r = Records::default();
    let mut from_ip: HashMap<String, Ipv4Addr> = HashMap::new();
    let t = Instant::now();
    let mut sent = 0;
    let mut buf = [0u8; 4096];
    while t.elapsed() < wait {
        if sent < 2 && t.elapsed() >= Duration::from_millis(sent * 600) {
            let _ = sock.send_to(&q, dst);
            sent += 1;
        }
        if let Ok((n, SocketAddr::V4(src))) = sock.recv_from(&mut buf)
            && n > 12
        {
            let before = r.ptr.len();
            let _ = parse(&buf[..n], &mut r);
            for p in &r.ptr[before..] {
                from_ip.insert(p.to_lowercase(), *src.ip());
            }
        }
    }
    let suffix = format!(".{}", service.trim_end_matches('.').to_lowercase());
    let mut out: Vec<Service> = Vec::new();
    for p in &r.ptr {
        let key = p.to_lowercase();
        if !key.ends_with(&suffix) {
            continue;
        }
        let label = p[..p.len() - suffix.len()].to_string();
        let (host, port) = r.srv.get(&key).cloned().unwrap_or_default();
        let Some(ip) = r.a.get(&host).copied().or_else(|| from_ip.get(&key).copied()) else { continue };
        if !out.iter().any(|s| s.ip == ip) {
            out.push(Service { name: label, ip, port });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_answer() {
        // PTR _wled._tcp.local → desk._wled._tcp.local, SRV port 80 → desk.local, A 192.168.1.42
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 3, 0, 0, 0, 0];
        let svc: &[u8] = b"\x05_wled\x04_tcp\x05local\x00";
        p.extend(svc);
        p.extend([0, 12, 0, 1, 0, 0, 0, 120, 0, 7, 4, b'd', b'e', b's', b'k', 0xC0, 12]);
        let inst = p.len();
        p.extend([0xC0, (inst - 7) as u8]);
        // SRV owner: pointer to "desk…" label at inst-7
        p.extend([0, 33, 0, 1, 0, 0, 0, 120, 0, 13, 0, 0, 0, 0, 0, 80, 4, b'd', b'e', b's', b'k', 0xC0, 23]);
        p.extend([4, b'd', b'e', b's', b'k', 0xC0, 23]);
        p.extend([0, 1, 0, 1, 0, 0, 0, 120, 0, 4, 192, 168, 1, 42]);
        let mut r = Records::default();
        parse(&p, &mut r).unwrap();
        assert_eq!(r.ptr, vec!["desk._wled._tcp.local".to_string()]);
        assert_eq!(r.srv.get("desk._wled._tcp.local"), Some(&("desk.local".to_string(), 80)));
        assert_eq!(r.a.get("desk.local"), Some(&Ipv4Addr::new(192, 168, 1, 42)));
    }
}
