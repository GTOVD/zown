//! Host I/O (SPEC §23). Same path rules as `zown/hostio.py`.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::buf::BUF_MAX;
use super::error::{BOUNDS, CAP_DENIED, INTEGRITY_FAIL, OVERFLOW, STACK_UNDERFLOW, TYPE_MISMATCH};
use super::store::try_snapshot;
use super::value::{BufAlloc, Value};
use super::{RunError, Vm};

const LINK_HOPS: usize = 40;

enum PathFail {
    Type,
    Bounds,
}

fn parts(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .map(str::to_string)
        .collect()
}

fn resolve(path: &str) -> Result<String, PathFail> {
    if path.is_empty() || path.contains('\0') {
        return Err(PathFail::Type);
    }
    let mut pending = if path.starts_with('/') {
        Vec::new()
    } else {
        let cwd = std::env::current_dir().map_err(|_| PathFail::Bounds)?;
        parts(&cwd.to_string_lossy())
    };
    pending.extend(parts(path));
    let mut out: Vec<String> = Vec::new();
    let mut hops = 0usize;
    let mut i = 0usize;
    while i < pending.len() {
        let part = pending[i].clone();
        i += 1;
        if part == ".." {
            out.pop();
            continue;
        }
        let cur = if out.is_empty() {
            format!("/{part}")
        } else {
            format!("/{}/{}", out.join("/"), part)
        };
        let meta = fs::symlink_metadata(&cur).ok();
        if meta.is_some_and(|m| m.file_type().is_symlink()) {
            hops += 1;
            if hops > LINK_HOPS {
                return Err(PathFail::Bounds);
            }
            let target = fs::read_link(&cur).map_err(|_| PathFail::Bounds)?;
            let text = target.to_string_lossy();
            let extra = parts(&text);
            if text.starts_with('/') {
                out.clear();
            }
            let mut next = extra;
            next.extend(pending[i..].iter().cloned());
            pending = next;
            i = 0;
            continue;
        }
        out.push(part);
    }
    if out.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", out.join("/")))
    }
}

fn allowed(root: &str, candidate: &str) -> bool {
    if candidate == root {
        return true;
    }
    if fs::metadata(root).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let prefix = if root.ends_with('/') {
        root.to_string()
    } else {
        format!("{root}/")
    };
    candidate.starts_with(&prefix)
}

fn pinned(name: &str) -> Result<Option<i64>, ()> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    match raw.parse::<i64>() {
        Ok(n) => Ok(Some(n)),
        Err(_) => {
            let body = raw.strip_prefix('-').unwrap_or(raw);
            if !body.is_empty() && body.bytes().all(|c| c.is_ascii_digit()) {
                Err(())
            } else {
                Ok(None)
            }
        }
    }
}

impl Vm {
    pub fn set_args(&mut self, args: Vec<String>) {
        self.args = args;
    }

    pub fn pop_host_str(&mut self, op: &str) -> Result<String, RunError> {
        match self.pop(op)? {
            Value::Str(s) => Ok(s),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a string, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn path_fail(&self, op: &str, fail: PathFail) -> RunError {
        match fail {
            PathFail::Type => self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a path string"),
                Some(op.to_string()),
                "the path is a non-empty string with no NUL",
            ),
            PathFail::Bounds => self.err(
                BOUNDS,
                format!("`{op}` path has too many symlinks"),
                Some(op.to_string()),
                "a symlink loop cannot be resolved",
            ),
        }
    }

    fn deny(&self, op: &str, cap: &str, msg: String, hint: &'static str) -> RunError {
        let mut e = self.err(CAP_DENIED, msg, Some(op.to_string()), hint);
        e.kind = "sec";
        e.cap = Some(cap.to_string());
        e
    }

    /// Resolve `path`, push it as a disk root, and grant `d`. The bool is whether
    /// this call newly added the capability.
    pub fn begin_disk(&mut self, path: &str) -> Result<bool, RunError> {
        let root = resolve(path).map_err(|f| self.path_fail("gd", f))?;
        self.roots.push(root);
        Ok(self.hold_cap("d".to_string()))
    }

    pub fn end_disk(&mut self, fresh: bool) {
        self.roots.pop();
        if fresh {
            self.drop_cap("d");
        }
    }

    fn under_disk(&self, op: &str, path: &str) -> Result<String, RunError> {
        if !self.caps.contains("d") || self.roots.is_empty() {
            return Err(self.deny(
                op,
                "d",
                format!("`{op}` needs `d and a granted path"),
                "grant a directory: $path$ [ ... ] gd",
            ));
        }
        let candidate = resolve(path).map_err(|f| self.path_fail(op, f))?;
        if self.roots.iter().any(|root| allowed(root, &candidate)) {
            return Ok(candidate);
        }
        Err(self.deny(
            op,
            "d",
            format!("`{op}` path is outside the granted root"),
            "the grant is a directory or one file; `..` and symlinks are resolved first",
        ))
    }

    fn buf_bytes(&self, op: &str) -> Result<Vec<u8>, RunError> {
        let Some(top) = self.stack.last() else {
            return Err(self.err(
                STACK_UNDERFLOW,
                format!("`{op}` needs a buf under the path"),
                Some(op.to_string()),
                "the stack is buf, then path",
            ));
        };
        match top {
            Value::Buf { alloc, .. } => {
                let g = alloc.borrow();
                match &g.data {
                    Some(data) => Ok(data.clone()),
                    None => {
                        drop(g);
                        Err(self.err(
                            BOUNDS,
                            format!("`{op}` buffer was freed"),
                            Some(op.to_string()),
                            "the exclusive owner was dropped with `,`",
                        ))
                    }
                }
            }
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a buf, got {tn}"),
                    Some(op.to_string()),
                    "read with fr, or allocate with bf",
                ))
            }
        }
    }

    pub(super) fn b_gd(&mut self) -> Result<(), RunError> {
        let blk = self.pop_block("gd")?;
        let path = self.pop_host_str("gd")?;
        let fresh = self.begin_disk(&path)?;
        let res = self.invoke(blk, "gd");
        self.end_disk(fresh);
        res
    }

    fn map_bytes(&self, op: &str, path: &str) -> Result<Vec<u8>, RunError> {
        let src = self.under_disk(op, path)?;
        let meta = match fs::metadata(&src) {
            Ok(m) => m,
            Err(_) => {
                return Err(self.err(
                    BOUNDS,
                    format!("`{op}` file is not there"),
                    Some(op.to_string()),
                    "write it with mm first",
                ))
            }
        };
        if meta.is_dir() {
            return Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` path is a directory"),
                Some(op.to_string()),
                "the map is one file",
            ));
        }
        if meta.len() > BUF_MAX as u64 {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` file is larger than {BUF_MAX} bytes"),
                Some(op.to_string()),
                "",
            ));
        }
        let data = fs::read(&src).map_err(|_| {
            self.err(
                BOUNDS,
                format!("`{op}` file is not there"),
                Some(op.to_string()),
                "",
            )
        })?;
        let text = std::str::from_utf8(&data).map_err(|_| {
            self.sec(
                INTEGRITY_FAIL,
                op,
                format!("`{op}` file is not a snapshot"),
                "a crash can leave a short file; mm writes the whole snapshot",
            )
        })?;
        if try_snapshot(text).is_none() {
            return Err(self.sec(
                INTEGRITY_FAIL,
                op,
                format!("`{op}` file is not a snapshot"),
                "a crash can leave a short file; mm writes the whole snapshot",
            ));
        }
        Ok(data)
    }

    pub(super) fn b_mm(&mut self) -> Result<(), RunError> {
        let path = self.pop_host_str("mm")?;
        let dest = self.under_disk("mm", &path)?;
        if fs::metadata(&dest).is_ok_and(|m| m.is_dir()) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`mm` path is a directory".into(),
                Some("mm".into()),
                "mm writes one file",
            ));
        }
        if let Some(parent) = std::path::Path::new(&dest).parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                return Err(self.err(
                    BOUNDS,
                    "`mm` the host refused the write".into(),
                    Some("mm".into()),
                    "the parent directory has to exist",
                ));
            }
        }
        let text = self.graph.snapshot();
        let tmp = format!("{dest}.tmp");
        let wrote = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&tmp)?;
            use std::io::Write;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &dest)?;
            if let Some(parent) = std::path::Path::new(&dest).parent() {
                let dir = if parent.as_os_str().is_empty() {
                    std::path::Path::new(".")
                } else {
                    parent
                };
                fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
        })();
        if wrote.is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(self.err(
                BOUNDS,
                "`mm` the host refused the write".into(),
                Some("mm".into()),
                "",
            ));
        }
        Ok(())
    }

    pub(super) fn b_mo(&mut self) -> Result<(), RunError> {
        let path = self.pop_host_str("mo")?;
        let data = self.map_bytes("mo", &path)?;
        let n = data.len();
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(data),
                n,
            })),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn b_ml(&mut self) -> Result<(), RunError> {
        let path = self.pop_host_str("ml")?;
        let data = self.map_bytes("ml", &path)?;
        let text = std::str::from_utf8(&data).map_err(|_| {
            self.sec(
                INTEGRITY_FAIL,
                "ml",
                "`ml` file is not a snapshot".into(),
                "a crash can leave a short file; mm writes the whole snapshot",
            )
        })?;
        let Some(graph) = try_snapshot(text) else {
            return Err(self.sec(
                INTEGRITY_FAIL,
                "ml",
                "`ml` file is not a snapshot".into(),
                "a crash can leave a short file; mm writes the whole snapshot",
            ));
        };
        self.graph = graph;
        Ok(())
    }

    pub(super) fn b_fr(&mut self) -> Result<(), RunError> {
        let path = self.pop_host_str("fr")?;
        let src = self.under_disk("fr", &path)?;
        let meta = match fs::metadata(&src) {
            Ok(m) => m,
            Err(_) => {
                return Err(self.err(
                    BOUNDS,
                    "`fr` file is not there".into(),
                    Some("fr".into()),
                    "the path is under the grant, and the file exists",
                ))
            }
        };
        if meta.is_dir() {
            return Err(self.err(
                TYPE_MISMATCH,
                "`fr` path is a directory".into(),
                Some("fr".into()),
                "fr reads one file",
            ));
        }
        if meta.len() > BUF_MAX as u64 {
            return Err(self.err(
                BOUNDS,
                format!("`fr` file is larger than {BUF_MAX} bytes"),
                Some("fr".into()),
                "the buffer budget is 1048576 bytes",
            ));
        }
        let data = match fs::read(&src) {
            Ok(d) => d,
            Err(_) => {
                return Err(self.err(
                    BOUNDS,
                    "`fr` the host refused the read".into(),
                    Some("fr".into()),
                    "",
                ))
            }
        };
        if data.len() > BUF_MAX as usize {
            return Err(self.err(
                BOUNDS,
                format!("`fr` file is larger than {BUF_MAX} bytes"),
                Some("fr".into()),
                "the buffer budget is 1048576 bytes",
            ));
        }
        let n = data.len();
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(data),
                n,
            })),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn b_fw(&mut self) -> Result<(), RunError> {
        let path = self.pop_host_str("fw")?;
        let data = self.buf_bytes("fw")?;
        let dest = self.under_disk("fw", &path)?;
        if fs::metadata(&dest).is_ok_and(|m| m.is_dir()) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`fw` path is a directory".into(),
                Some("fw".into()),
                "fw writes one file",
            ));
        }
        if let Some(parent) = std::path::Path::new(&dest).parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                return Err(self.err(
                    BOUNDS,
                    "`fw` the host refused the write".into(),
                    Some("fw".into()),
                    "the parent directory has to exist",
                ));
            }
        }
        if fs::write(&dest, &data).is_err() {
            return Err(self.err(
                BOUNDS,
                "`fw` the host refused the write".into(),
                Some("fw".into()),
                "",
            ));
        }
        Ok(())
    }

    pub(super) fn b_ac(&mut self) -> Result<(), RunError> {
        let n = self.args.len();
        if n > i64::MAX as usize {
            return Err(self.err(
                OVERFLOW,
                "`ac` argument count does not fit int".into(),
                Some("ac".into()),
                "",
            ));
        }
        self.push(Value::Int(n as i64));
        Ok(())
    }

    pub(super) fn b_ag(&mut self) -> Result<(), RunError> {
        let idx = match self.pop("ag")? {
            Value::Int(n) => n,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`ag` expected an int, got {tn}"),
                    Some("ag".into()),
                    "",
                ));
            }
        };
        if idx < 0 || idx as usize >= self.args.len() {
            return Err(self.err(
                BOUNDS,
                "`ag` index is outside the argument vector".into(),
                Some("ag".into()),
                "ac pushes the count",
            ));
        }
        let s = self.args[idx as usize].clone();
        self.push(Value::Str(s));
        Ok(())
    }

    pub(super) fn b_en(&mut self) -> Result<(), RunError> {
        if !self.caps.contains("v") {
            return Err(self.deny(
                "en",
                "v",
                "`en` needs `v".into(),
                "grant it: `v [ ... ] gr",
            ));
        }
        let name = self.pop_host_str("en")?;
        if name.is_empty() || name.contains('\0') {
            return Err(self.err(
                TYPE_MISMATCH,
                "`en` expected a variable name".into(),
                Some("en".into()),
                "the name is a non-empty string with no NUL",
            ));
        }
        match std::env::var(&name) {
            Ok(v) => {
                self.push(Value::Str(v));
                Ok(())
            }
            Err(_) => Err(self.err(
                BOUNDS,
                "`en` that variable is not set".into(),
                Some("en".into()),
                "en looks up one name; it does not list the environment",
            )),
        }
    }

    fn clock_number(&self, name: &str, real: u128) -> Result<i64, RunError> {
        match pinned(name) {
            Ok(Some(n)) => Ok(n),
            Err(()) => Err(self.err(
                OVERFLOW,
                "`tm` pin does not fit int".into(),
                Some("tm".into()),
                "",
            )),
            Ok(None) if real > i64::MAX as u128 => Err(self.err(
                OVERFLOW,
                "`tm` result does not fit int".into(),
                Some("tm".into()),
                "",
            )),
            Ok(None) => Ok(real as i64),
        }
    }

    pub(super) fn b_tm(&mut self) -> Result<(), RunError> {
        if !self.caps.contains("c") {
            return Err(self.deny(
                "tm",
                "c",
                "`tm` needs `c".into(),
                "grant it: `c [ ... ] gr",
            ));
        }
        // Unpinned values differ from Python's monotonic clock. Goldens set
        // ZOWN_TICK and ZOWN_WALL, and those are the numbers both VMs print.
        let tick = self.clock_number("ZOWN_TICK", monotonic_ns())?;
        let wall_real = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u128)
            .unwrap_or(0);
        let wall = self.clock_number("ZOWN_WALL", wall_real)?;
        self.push(Value::Int(tick));
        self.push(Value::Int(wall));
        Ok(())
    }
}

fn monotonic_ns() -> u128 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos()
}
