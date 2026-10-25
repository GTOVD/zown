//! TCP streams on `[::1]` (SPEC §14). Same frames and name file as `zown/stream.py`.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use super::error::{BOUNDS, CAP_DENIED, TYPE_MISMATCH};
use super::value::{Conn, Value};
use super::{RunError, Vm};

const BUF_MAX: usize = 1 << 20;
const WAIT: Duration = Duration::from_secs(1);

fn grant_hint(cap: &str) -> &'static str {
    match cap {
        "t" => "grant it: `t [ ... ] gr",
        _ => "grant it: `r [ ... ] gr",
    }
}

impl Vm {
    fn deny_cap(&self, op: &str, cap: &str) -> RunError {
        let mut err = self.sec(
            CAP_DENIED,
            op,
            format!("`{op}` needs `{cap}"),
            grant_hint(cap),
        );
        err.cap = Some(cap.to_string());
        err
    }

    fn need_cap(&self, op: &str, cap: &str) -> Result<(), RunError> {
        if self.caps.contains(cap) {
            Ok(())
        } else {
            Err(self.deny_cap(op, cap))
        }
    }

    fn pop_port(&mut self, op: &str) -> Result<u16, RunError> {
        match self.pop(op)? {
            Value::Int(n) if (1..=65535).contains(&n) => Ok(n as u16),
            Value::Int(n) => Err(self.err(
                BOUNDS,
                format!("`{op}` port {n} is outside 1..65535"),
                Some(op.to_string()),
                "",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a port, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn pop_word(&mut self, op: &str) -> Result<String, RunError> {
        match self.pop(op)? {
            Value::Str(s) if !s.is_empty() && !s.chars().any(|c| c.is_whitespace()) => Ok(s),
            Value::Str(_) => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a single-word name, got a name with whitespace"),
                Some(op.to_string()),
                "",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a single-word name, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn pop_stream(&mut self, op: &str) -> Result<Value, RunError> {
        match self.pop(op)? {
            Value::Stm { conn, exclusive } => {
                if conn.stream.borrow().is_none() {
                    return Err(self.err(
                        BOUNDS,
                        format!("`{op}` stream is closed"),
                        Some(op.to_string()),
                        "the exclusive stream was dropped with `,`",
                    ));
                }
                Ok(Value::Stm { conn, exclusive })
            }
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a stream, got {tn}"),
                    Some(op.to_string()),
                    "open one with ~o, ~l, or ~c",
                ))
            }
        }
    }

    fn net_err(&self, op: &str, err: std::io::Error) -> RunError {
        let msg = if err.kind() == std::io::ErrorKind::TimedOut
            || err.kind() == std::io::ErrorKind::WouldBlock
        {
            format!("`{op}` timed out")
        } else {
            format!("`{op}` {err}")
        };
        self.err(BOUNDS, msg, Some(op.to_string()), "the peer did not answer on [::1]")
    }

    pub(super) fn op_open_stream(&mut self) -> Result<(), RunError> {
        self.need_cap("~o", "t")?;
        let port = self.pop_port("~o")?;
        let host = match self.pop("~o")? {
            Value::Str(s) => s,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`~o` expected a host string, got {tn}"),
                    Some("~o".into()),
                    "",
                ));
            }
        };
        if host != "::1" {
            return Err(self.err(
                BOUNDS,
                "`~o` stays on [::1]".into(),
                Some("~o".into()),
                "this slice does not dial off the machine",
            ));
        }
        let addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
        let sock = TcpStream::connect_timeout(&addr, WAIT).map_err(|e| self.net_err("~o", e))?;
        arm_mut(&sock);
        self.push(Value::Stm {
            conn: Rc::new(Conn {
                stream: std::cell::RefCell::new(Some(sock)),
            }),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn op_listen(&mut self) -> Result<(), RunError> {
        self.need_cap("~l", "r")?;
        let port = self.pop_port("~l")?;
        let proto = self.pop_word("~l")?;
        let name = self.pop_word("~l")?;
        let listener = listen_on(port).map_err(|e| self.net_err("~l", e))?;
        register(&name, &proto, port).map_err(|e| self.net_err("~l", e))?;
        let (sock, _) = accept_one(&listener).map_err(|e| self.net_err("~l", e))?;
        drop(listener);
        self.push(Value::Stm {
            conn: Rc::new(Conn {
                stream: std::cell::RefCell::new(Some(sock)),
            }),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn op_connect_name(&mut self) -> Result<(), RunError> {
        self.need_cap("~c", "t")?;
        let proto = self.pop_word("~c")?;
        let name = self.pop_word("~c")?;
        let found = lookup(&name).map_err(|e| self.net_err("~c", e))?;
        let Some((offered, port)) = found else {
            return Err(self.err(
                BOUNDS,
                format!("`~c` has no peer named {name}"),
                Some("~c".into()),
                "the listener registers the name with ~l",
            ));
        };
        if offered != proto {
            return Err(self.err(
                TYPE_MISMATCH,
                format!("`~c` protocol {proto} does not match {offered} for {name}"),
                Some("~c".into()),
                "",
            ));
        }
        let addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
        let sock = TcpStream::connect_timeout(&addr, WAIT).map_err(|e| self.net_err("~c", e))?;
        arm_mut(&sock);
        self.push(Value::Stm {
            conn: Rc::new(Conn {
                stream: std::cell::RefCell::new(Some(sock)),
            }),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn op_send_stream(&mut self) -> Result<(), RunError> {
        self.need_cap("~s", "t")?;
        let text = match self.pop("~s")? {
            Value::Str(s) => s,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`~s` expected a string, got {tn}"),
                    Some("~s".into()),
                    "",
                ));
            }
        };
        let data = text.into_bytes();
        if data.len() > BUF_MAX {
            return Err(self.err(
                BOUNDS,
                "`~s` message is too large".into(),
                Some("~s".into()),
                "",
            ));
        }
        let stm = self.pop_stream("~s")?;
        let sent = if let Value::Stm { conn, .. } = &stm {
            let result = (|| {
                let mut guard = conn.stream.borrow_mut();
                let sock = guard.as_mut().ok_or_else(|| {
                    self.err(
                        BOUNDS,
                        "`~s` stream is closed".into(),
                        Some("~s".into()),
                        "",
                    )
                })?;
                write_frame(sock, &data).map_err(|e| self.net_err("~s", e))
            })();
            result
        } else {
            Ok(())
        };
        self.push(stm);
        sent
    }

    pub(super) fn op_recv_stream(&mut self) -> Result<(), RunError> {
        self.need_cap("~r", "r")?;
        let stm = self.pop_stream("~r")?;
        let read = if let Value::Stm { conn, .. } = &stm {
            let mut guard = conn.stream.borrow_mut();
            let sock = match guard.as_mut() {
                Some(sock) => sock,
                None => {
                    drop(guard);
                    self.push(stm);
                    return Err(self.err(
                        BOUNDS,
                        "`~r` stream is closed".into(),
                        Some("~r".into()),
                        "",
                    ));
                }
            };
            match read_frame(sock) {
                Ok(data) => Ok(data),
                Err(e) => Err(self.net_err("~r", e)),
            }
        } else {
            Ok(Vec::new())
        };
        let data = match read {
            Ok(data) => data,
            Err(e) => {
                self.push(stm);
                return Err(e);
            }
        };
        let text = match String::from_utf8(data) {
            Ok(text) => text,
            Err(_) => {
                self.push(stm);
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`~r` frame is not UTF-8".into(),
                    Some("~r".into()),
                    "",
                ));
            }
        };
        self.push(stm);
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn op_relay(&mut self) -> Result<(), RunError> {
        self.need_cap("~w", "r")?;
        self.need_cap("~w", "t")?;
        let port = self.pop_port("~w")?;
        let listener = listen_on(port).map_err(|e| self.net_err("~w", e))?;
        let (mut left, _) = accept_one(&listener).map_err(|e| self.net_err("~w", e))?;
        let (mut right, _) = accept_one(&listener).map_err(|e| self.net_err("~w", e))?;
        drop(listener);
        arm_mut(&left);
        arm_mut(&right);
        let forward = (|| {
            let frame = read_frame(&mut left).map_err(|e| self.net_err("~w", e))?;
            write_frame(&mut right, &frame).map_err(|e| self.net_err("~w", e))?;
            let back = read_frame(&mut right).map_err(|e| self.net_err("~w", e))?;
            write_frame(&mut left, &back).map_err(|e| self.net_err("~w", e))?;
            Ok(())
        })();
        drop(left);
        drop(right);
        forward
    }
}

fn arm_mut(sock: &TcpStream) {
    sock.set_read_timeout(Some(WAIT)).ok();
    sock.set_write_timeout(Some(WAIT)).ok();
}

fn listen_on(port: u16) -> std::io::Result<TcpListener> {
    let addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn accept_one(listener: &TcpListener) -> std::io::Result<(TcpStream, SocketAddr)> {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false)?;
                arm_mut(&pair.0);
                return Ok(pair);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= WAIT {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_exact(sock: &mut TcpStream, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    sock.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_frame(sock: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let hdr = read_exact(sock, 4)?;
    let n = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
    if n > BUF_MAX {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame",
        ));
    }
    read_exact(sock, n)
}

fn write_frame(sock: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    let n = (data.len() as u32).to_be_bytes();
    sock.write_all(&n)?;
    sock.write_all(data)?;
    Ok(())
}

fn mesh_dir() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("ZOWN_MESH") {
        if !path.is_empty() {
            return std::path::PathBuf::from(path);
        }
    }
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(tmp).join("zown-mesh")
}

fn register(name: &str, proto: &str, port: u16) -> std::io::Result<()> {
    with_names(|lines| {
        lines.retain(|ln| ln.split(' ').next() != Some(name));
        lines.push(format!("{name} {proto} {port}"));
    })
}

fn lookup(name: &str) -> std::io::Result<Option<(String, u16)>> {
    let path = mesh_dir().join("names");
    let start = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(&path) {
            for ln in text.lines() {
                let parts: Vec<&str> = ln.split(' ').collect();
                if parts.len() == 3 && parts[0] == name {
                    if let Ok(port) = parts[2].parse::<u16>() {
                        return Ok(Some((parts[1].to_string(), port)));
                    }
                }
            }
        }
        if start.elapsed() >= WAIT {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn with_names(edit: impl FnOnce(&mut Vec<String>)) -> std::io::Result<()> {
    let dir = mesh_dir();
    fs::create_dir_all(&dir)?;
    let lock = dir.join("names.lock");
    let path = dir.join("names");
    let start = Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if start.elapsed() >= WAIT {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "lock"));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
    let result = (|| {
        let text = fs::read_to_string(&path).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().filter(|ln| !ln.is_empty()).map(|s| s.to_string()).collect();
        edit(&mut lines);
        let body = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        let tmp = dir.join("names.tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, &path)?;
        Ok(())
    })();
    let _ = fs::remove_file(lock);
    result
}
