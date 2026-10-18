//! Std-only HTTP/1.0 probe for images with no shell and no curl.
//! usage: healthcheck <host:port> <path>     exit 0 = HTTP 200, exit 1 = anything else
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(addr), Some(path)) = (args.next(), args.next()) else {
        eprintln!("usage: healthcheck <host:port> <path>");
        return ExitCode::from(1);
    };
    let timeout = Duration::from_secs(2); // keep below the HEALTHCHECK --timeout
    let Some(sock) = addr.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return ExitCode::from(1);
    };
    let Ok(mut s) = TcpStream::connect_timeout(&sock, timeout) else {
        return ExitCode::from(1);
    };
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let req = format!("GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return ExitCode::from(1);
    }
    let mut head = [0u8; 12]; // "HTTP/1.x 200"
    if s.read_exact(&mut head).is_err() {
        return ExitCode::from(1);
    }
    if &head[9..12] == b"200" { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
