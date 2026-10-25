//! Cooperative fast lanes (SPEC §22). Same rules as `zown/lanes.py`.
//!
//! A tick runs each registered lane, then at most one compute job. Native
//! blocks are `Slice::Native` and are invoked by `zown-rt`, which owns the
//! function table. The tree-walker only sees `Slice::Ast`. Calling a native
//! function from inside a `&mut Vm` would alias the VM, so `rn` on the native
//! path is `exec_rn` in the runtime, using the helpers below.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use zown_ast::Node;

use super::error::{BOUNDS, NOT_CALLABLE, TYPE_MISMATCH, UNSUPPORTED};
use super::{RunError, Value, Vm};

pub const PIPE_CAP: usize = 8;
const LANE_ORDER: [&str; 2] = ["m", "n"];

/// A block the scheduler can run. `Ast` is the tree-walker; `Native` is a
/// compiled block id.
#[derive(Clone)]
pub enum Slice {
    Ast(Rc<Vec<Node>>),
    Native(u32),
}

pub struct LaneState {
    pub cpus_override: Option<i64>,
    requested: Option<i64>,
    bodies: HashMap<String, Slice>,
    pipes: HashMap<String, VecDeque<Value>>,
    jobs: Vec<VecDeque<Slice>>,
    steals: i64,
    trace: String,
    lane: Option<String>,
    in_tick: bool,
    rr: u64,
}

impl LaneState {
    pub(super) fn new() -> Self {
        let mut pipes = HashMap::new();
        for name in LANE_ORDER {
            pipes.insert(name.to_string(), VecDeque::new());
        }
        LaneState {
            cpus_override: None,
            requested: None,
            bodies: HashMap::new(),
            pipes,
            jobs: Vec::new(),
            steals: 0,
            trace: String::new(),
            lane: None,
            in_tick: false,
            rr: 0,
        }
    }
}

pub(super) fn cpu_count(override_cpus: Option<i64>) -> i64 {
    cpus_from(
        override_cpus,
        std::env::var("ZOWN_CPUS").ok().as_deref(),
        hardware_cpus(),
    )
}

fn hardware_cpus() -> i64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i64)
        .unwrap_or(1)
}

/// `override`, then an explicit env string, then the host count. A non-numeric
/// env value is ignored. Split out so tests do not touch process state.
pub(super) fn cpus_from(override_cpus: Option<i64>, env: Option<&str>, hardware: i64) -> i64 {
    if let Some(n) = override_cpus {
        if n >= 1 {
            return n;
        }
    }
    if let Some(raw) = env {
        let raw = raw.trim();
        if !raw.is_empty() {
            if let Ok(n) = raw.parse::<i64>() {
                if n >= 1 {
                    return n;
                }
            }
        }
    }
    hardware.max(1)
}

pub(super) fn room_for(cpus: i64) -> i64 {
    (cpus - 1).max(1)
}

pub(super) fn grant_for(cpus: i64, requested: Option<i64>) -> i64 {
    let room = room_for(cpus);
    match requested {
        None => 2.min(room),
        Some(n) if n < 0 => 0,
        Some(n) => n.min(room),
    }
}

pub(super) fn workers_for(cpus: i64, granted: i64) -> usize {
    (cpus - granted).max(1) as usize
}

impl Vm {
    fn cpus(&self) -> i64 {
        cpu_count(self.lanes.cpus_override)
    }

    fn granted(&self) -> i64 {
        grant_for(self.cpus(), self.lanes.requested)
    }

    fn ensure_workers(&mut self) {
        let n = workers_for(self.cpus(), self.granted());
        while self.lanes.jobs.len() < n {
            self.lanes.jobs.push(VecDeque::new());
        }
        while self.lanes.jobs.len() > n {
            let extra = self.lanes.jobs.pop().unwrap();
            self.lanes.jobs.last_mut().unwrap().extend(extra);
        }
    }

    fn pop_plain_int(&mut self, op: &str) -> Result<i64, RunError> {
        match self.pop(op)? {
            Value::Int(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected an int, got {tn}"),
                    Some(op.to_string()),
                    "the count is a plain int",
                ))
            }
        }
    }

    fn pop_slice(&mut self, op: &str) -> Result<Slice, RunError> {
        match self.pop(op)? {
            Value::Block(b) => Ok(Slice::Ast(b)),
            Value::Func { id, .. } => Ok(Slice::Native(id)),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    NOT_CALLABLE,
                    format!("`{op}` expected a block, got {tn}"),
                    Some(op.to_string()),
                    "wrap the code in [ ... ] to make a block",
                ))
            }
        }
    }

    pub(super) fn op_pipe(&mut self) -> Result<(), RunError> {
        let Some(lane) = self.lanes.lane.clone() else {
            return Err(self.err(
                TYPE_MISMATCH,
                "`|` sends only inside a running lane".into(),
                Some("|".into()),
                "register a lane with ~m or ~n; pull from outside with pm or pn",
            ));
        };
        let v = self.pop("|")?;
        let full = self
            .lanes
            .pipes
            .get(&lane)
            .map(|p| p.len() >= PIPE_CAP)
            .unwrap_or(false);
        if full {
            return Err(self.err(
                BOUNDS,
                format!("lane {lane} pipe is full ({PIPE_CAP})"),
                Some("|".into()),
                "pull with pm or pn before the next send",
            ));
        }
        self.lanes.pipes.get_mut(&lane).unwrap().push_back(v);
        Ok(())
    }

    pub(super) fn op_spawn(&mut self) -> Result<(), RunError> {
        let body = self.pop_slice("^")?;
        self.ensure_workers();
        self.lanes.jobs[0].push_back(body);
        Ok(())
    }

    pub(super) fn op_lane(&mut self, name: &str) -> Result<(), RunError> {
        let op = format!("~{name}");
        if !LANE_ORDER.contains(&name) {
            return Err(self.err(
                UNSUPPORTED,
                format!("operator `{op}` is not implemented"),
                Some(op),
                "~m and ~n are lanes; ~o ~s ~r ~c ~l ~w are streams; ~p publishes and ~f fetches",
            ));
        }
        let body = self.pop_slice(&op)?;
        self.lanes.bodies.insert(name.to_string(), body);
        Ok(())
    }

    pub(super) fn b_hw(&mut self) -> Result<(), RunError> {
        let n = self.cpus();
        self.push(Value::Int(n));
        Ok(())
    }

    pub(super) fn b_lc(&mut self) -> Result<(), RunError> {
        let n = self.pop_plain_int("lc")?;
        if n < 0 {
            return Err(self.err(
                BOUNDS,
                "`lc` requested slots must be >= 0".into(),
                Some("lc".into()),
                "ask for 0 to run every lane as a fiber",
            ));
        }
        self.lanes.requested = Some(n);
        self.ensure_workers();
        let g = self.granted();
        self.push(Value::Int(g));
        Ok(())
    }

    pub(super) fn b_pm(&mut self) -> Result<(), RunError> {
        self.pull("m", "pm")
    }

    pub(super) fn b_pn(&mut self) -> Result<(), RunError> {
        self.pull("n", "pn")
    }

    fn pull(&mut self, lane: &str, word: &str) -> Result<(), RunError> {
        let empty = self
            .lanes
            .pipes
            .get(lane)
            .map(|p| p.is_empty())
            .unwrap_or(true);
        if empty {
            return Err(self.err(
                BOUNDS,
                format!("lane {lane} pipe is empty"),
                Some(word.to_string()),
                "a lane sends with `|` inside its body; the pipe holds 8 values",
            ));
        }
        let v = self.lanes.pipes.get_mut(lane).unwrap().pop_front().unwrap();
        self.push(v);
        Ok(())
    }

    pub(super) fn b_ws(&mut self) -> Result<(), RunError> {
        let n = self.lanes.steals;
        self.push(Value::Int(n));
        Ok(())
    }

    pub(super) fn b_sg(&mut self) -> Result<(), RunError> {
        self.push(Value::Str(self.lanes.trace.clone()));
        Ok(())
    }

    /// Pop `k` and mark the scheduler busy. The caller must [`end_rn`](Self::end_rn).
    pub fn begin_rn(&mut self) -> Result<i64, RunError> {
        if self.lanes.in_tick {
            return Err(self.err(
                TYPE_MISMATCH,
                "`rn` cannot re-enter while a tick is running".into(),
                Some("rn".into()),
                "a lane or a job runs to the end of its block; start ticks from outside",
            ));
        }
        let k = self.pop_plain_int("rn")?;
        if k < 0 {
            return Err(self.err(
                BOUNDS,
                "`rn` tick count must be >= 0".into(),
                Some("rn".into()),
                "",
            ));
        }
        self.lanes.in_tick = true;
        Ok(k)
    }

    pub fn end_rn(&mut self) {
        self.lanes.in_tick = false;
    }

    /// Registered lanes this tick: the pinned prefix, then the fiber rest.
    pub fn lane_schedule(&mut self) -> Vec<String> {
        self.ensure_workers();
        let granted = self.granted().max(0) as usize;
        let live: Vec<String> = LANE_ORDER
            .into_iter()
            .filter(|n| self.lanes.bodies.contains_key(*n))
            .map(|s| s.to_string())
            .collect();
        let mut order = Vec::with_capacity(live.len());
        order.extend(live.iter().take(granted).cloned());
        order.extend(live.iter().skip(granted).cloned());
        order
    }

    pub fn copy_body(&self, name: &str) -> Option<Slice> {
        self.lanes.bodies.get(name).cloned()
    }

    pub fn enter_lane(&mut self, name: &str) {
        self.lanes.lane = Some(name.to_string());
    }

    pub fn leave_lane(&mut self) {
        self.lanes.lane = None;
    }

    pub fn mark_trace(&mut self, letter: &str) {
        self.lanes.trace.push_str(letter);
    }

    /// The compute slice for this tick, if any worker has a job.
    pub fn take_job(&mut self) -> Option<Slice> {
        self.ensure_workers();
        let n = self.lanes.jobs.len();
        if n == 0 {
            return None;
        }
        let idx = (self.lanes.rr % n as u64) as usize;
        self.lanes.rr = self.lanes.rr.wrapping_add(1);
        if let Some(job) = self.lanes.jobs[idx].pop_front() {
            return Some(job);
        }
        for step in 1..n {
            let j = (idx + step) % n;
            if let Some(job) = self.lanes.jobs[j].pop_back() {
                self.lanes.steals += 1;
                return Some(job);
            }
        }
        None
    }

    pub fn call_ast(&mut self, body: Rc<Vec<Node>>, op: &str) -> Result<(), RunError> {
        self.invoke(body, op)
    }

    /// Tree-walker ticks. Native `rn` is `exec_rn` in `zown-rt` (same helpers).
    pub(super) fn b_rn(&mut self) -> Result<(), RunError> {
        let k = self.begin_rn()?;
        let mut result = Ok(());
        for _ in 0..k {
            if let Err(e) = self.one_tick_ast() {
                result = Err(e);
                break;
            }
        }
        self.end_rn();
        result
    }

    fn one_tick_ast(&mut self) -> Result<(), RunError> {
        let names = self.lane_schedule();
        for name in names {
            let Some(body) = self.copy_body(&name) else {
                continue;
            };
            self.enter_lane(&name);
            let op = format!("~{name}");
            let res = match body {
                Slice::Ast(b) => self.call_ast(b, &op),
                Slice::Native(_) => Err(self.err(
                    UNSUPPORTED,
                    "a compiled lane ran on the tree-walker".into(),
                    Some(op),
                    "",
                )),
            };
            self.leave_lane();
            res?;
            self.mark_trace(&name);
        }
        if let Some(job) = self.take_job() {
            let res = match job {
                Slice::Ast(b) => self.call_ast(b, "^"),
                Slice::Native(_) => Err(self.err(
                    UNSUPPORTED,
                    "a compiled job ran on the tree-walker".into(),
                    Some("^".into()),
                    "",
                )),
            };
            res?;
            self.mark_trace("c");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_cpus(src: &str, cpus: i64) -> Vm {
        let mut vm = Vm::new(Some("<test>".into()));
        vm.lanes.cpus_override = Some(cpus);
        vm.run_str(src).unwrap();
        vm
    }

    fn code_of(src: &str) -> &'static str {
        let mut vm = Vm::new(None);
        vm.run_str(src).unwrap_err().code()
    }

    #[test]
    fn grant_math() {
        assert_eq!(room_for(1), 1);
        assert_eq!(grant_for(1, None), 1);
        assert_eq!(workers_for(1, 1), 1);
        assert_eq!(grant_for(8, Some(1)), 1);
        assert_eq!(grant_for(8, Some(99)), 7);
        assert_eq!(workers_for(8, 2), 6);
        assert_eq!(cpus_from(Some(4), Some("9"), 2), 4);
        assert_eq!(cpus_from(None, Some("3"), 8), 3);
        assert_eq!(cpus_from(None, Some("nope"), 5), 5);
    }

    #[test]
    fn demo_trace_is_stable() {
        let vm = run_cpus("1 lc\n[ 7 | ] ~m\n[ 9 | ] ~n\n[ ] ^\n3 rn\nsg .", 8);
        assert_eq!(vm.out, "mncmnmn\n");
    }

    #[test]
    fn load_does_not_skip_a_lane() {
        let vm = run_cpus(
            "1 lc ,\n[ 1 | ] ~m\n[ 2 | ] ~n\n[ ] ^ [ ] ^ [ ] ^ [ ] ^ [ ] ^\n3 rn\nsg .\npm .\npn .",
            2,
        );
        assert_eq!(vm.out, "mncmncmnc\n1\n2\n");
    }

    #[test]
    fn steal_when_another_worker_is_idle() {
        let vm = run_cpus("0 lc , [ 1 ] ^ [ 2 ] ^ 2 rn ws .", 4);
        assert_eq!(vm.out, "1\n");
        assert!(matches!(
            vm.stack.as_slice(),
            [Value::Int(1), Value::Int(2)]
        ));
    }

    #[test]
    fn no_steal_with_one_worker() {
        let vm = run_cpus("0 lc , [ 1 ] ^ [ 2 ] ^ 2 rn ws .", 1);
        assert_eq!(vm.out, "0\n");
        assert!(matches!(
            vm.stack.as_slice(),
            [Value::Int(1), Value::Int(2)]
        ));
    }

    #[test]
    fn faults() {
        assert_eq!(code_of("pm"), BOUNDS);
        assert_eq!(code_of("1 |"), TYPE_MISMATCH);
        assert_eq!(code_of("[ ] ~z"), UNSUPPORTED);
        assert_eq!(code_of("1 ^"), NOT_CALLABLE);
        assert_eq!(code_of("1 _ lc"), BOUNDS);
        assert_eq!(code_of("1 lc\n[ 1 rn ] ~m\n1 rn"), TYPE_MISMATCH);
    }
}
