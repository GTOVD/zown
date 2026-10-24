//! Runtime the Cranelift backend calls. Values and operators go through the
//! reference `Vm`, so a native block and the tree-walker agree. Block *calls*
//! are native function pointers registered with [`Rt::set_fn`].

use std::io::{self, Write};
use std::process::abort;

pub use zown_vm::NatOp;
use zown_vm::{RunError, Value, Vm, STACK_OVERFLOW};

const MAX_DEPTH: u32 = 128;

pub struct Rt {
    vm: Vm,
    err: Option<RunError>,
    funcs: Vec<unsafe extern "C" fn(*mut Rt)>,
    /// Instruction shape of each compiled block, so `??` can read patterns.
    descs: Vec<Vec<NatOp>>,
    depth: u32,
}

impl Rt {
    pub fn new(file: &str) -> Self {
        Rt {
            vm: Vm::new(Some(file.to_string())),
            err: None,
            funcs: Vec::new(),
            descs: Vec::new(),
            depth: 0,
        }
    }

    pub fn set_fn(&mut self, id: usize, f: unsafe extern "C" fn(*mut Rt)) {
        if self.funcs.len() <= id {
            self.funcs.resize(id + 1, rt_missing);
        }
        self.funcs[id] = f;
    }

    pub fn set_desc(&mut self, id: usize, ops: Vec<NatOp>) {
        if self.descs.len() <= id {
            self.descs.resize(id + 1, Vec::new());
        }
        self.descs[id] = ops;
    }

    fn desc(&self, id: u32) -> Vec<NatOp> {
        self.descs.get(id as usize).cloned().unwrap_or_default()
    }

    pub fn result(self) -> (i32, String, String) {
        let stdout = self.vm.out;
        match self.err {
            Some(e) => (1, stdout, format!("{}\n{}", e.to_json(), e.render_human())),
            None => (0, stdout, String::new()),
        }
    }

    /// `(exit, stdout, json packet, human packet)`.
    pub fn emit(self) -> (i32, String, String, String) {
        let stdout = self.vm.out;
        match self.err {
            Some(e) => {
                let json = e.to_json();
                let human = e.render_human();
                (1, stdout, json, human)
            }
            None => (0, stdout, String::new(), String::new()),
        }
    }
}

unsafe extern "C" fn rt_missing(_rt: *mut Rt) {
    abort();
}

fn fail(rt: &mut Rt, err: RunError) -> i32 {
    if rt.err.is_none() {
        rt.err = Some(err);
    }
    1
}

fn str_at(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(bytes).into_owned()
}

#[no_mangle]
pub unsafe extern "C" fn rt_failed(rt: *mut Rt) -> i32 {
    if rt.is_null() || (*rt).err.is_some() { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_int(rt: *mut Rt, n: i64) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    (*rt).vm.stack.push(Value::Int(n));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_float(rt: *mut Rt, bits: i64) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    (*rt).vm.stack.push(Value::Float(f64::from_bits(bits as u64)));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_str(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    (*rt).vm.stack.push(Value::Str(str_at(ptr, len)));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_func(rt: *mut Rt, id: i32, nodes: i32) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    (*rt).vm.stack.push(Value::Func {
        id: id as u32,
        nodes: nodes as u32,
    });
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_cap(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    (*rt).vm.stack.push(Value::Cap(str_at(ptr, len)));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_bind(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    let name = str_at(ptr, len);
    match (*rt).vm.apply_bind(&name) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_load(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    let name = str_at(ptr, len);
    if !(*rt).vm.is_bound(&name) {
        match name.as_str() {
            "gr" => return grant(rt),
            "f4" => return vec_ctor(rt, "f4", 4, true, true, 32),
            "d2" => return vec_ctor(rt, "d2", 2, true, true, 64),
            "i4" => return vec_ctor(rt, "i4", 4, false, true, 32),
            "b16" => return vec_ctor(rt, "b16", 16, false, false, 8),
            _ => {}
        }
    }
    match (*rt).vm.apply_name(&name) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_move(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    let name = str_at(ptr, len);
    match (*rt).vm.apply_move(&name) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_op(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    let op = str_at(ptr, len);
    match (*rt).vm.apply_op(&op) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_select(rt: *mut Rt) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    match (*rt).vm.apply_select() {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

/// Pop a block id. Returns the id, or `-1` after recording an error.
#[no_mangle]
pub unsafe extern "C" fn rt_pop_func(rt: *mut Rt, op_at: i32) -> i64 {
    if rt_failed(rt) != 0 {
        return -1;
    }
    let op = if op_at == 0 { "@" } else { ";" };
    match (*rt).vm.pop_func_id(op) {
        Ok(id) => id as i64,
        Err(e) => {
            fail(&mut *rt, e);
            -1
        }
    }
}

/// Pop a value and report truthiness: `0` false, `1` true, `2` error.
#[no_mangle]
pub unsafe extern "C" fn rt_pop_truthy(rt: *mut Rt) -> i32 {
    if rt_failed(rt) != 0 {
        return 2;
    }
    match (*rt).vm.pop_truthy(";") {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => {
            fail(&mut *rt, e);
            2
        }
    }
}

/// Call native block `id`. `which` is `0` for `@` and `1` for `;`.
#[no_mangle]
pub unsafe extern "C" fn rt_call(rt: *mut Rt, id: i32, which: i32) -> i32 {
    let op = if which == 0 { "@" } else { ";" };
    call_block(rt, id as u32, op)
}

/// `[body] at` — run the body on a fresh stack and push `ok` or `err`.
#[no_mangle]
pub unsafe extern "C" fn rt_attempt(rt: *mut Rt) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    if (*rt).vm.is_bound("at") {
        return match (*rt).vm.apply_name("at") {
            Ok(()) => 0,
            Err(e) => fail(&mut *rt, e),
        };
    }
    let id = match (*rt).vm.pop_func_id("at") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let saved = (*rt).vm.take_stack();
    let code = call_block(rt, id, "at");
    let produced = (*rt).vm.take_stack();
    (*rt).vm.set_stack(saved);
    let failed = if code != 0 { (*rt).err.take() } else { None };
    if code != 0 && failed.is_none() {
        return 1;
    }
    match (*rt).vm.seal_attempt(produced, failed) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

/// `subject [ arms ] ??`.
#[no_mangle]
pub unsafe extern "C" fn rt_match(rt: *mut Rt) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    let arms_id = match (*rt).vm.pop_func_id("??") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let subject = match (*rt).vm.pop_value("??") {
        Ok(v) => v,
        Err(e) => return fail(&mut *rt, e),
    };
    let arms = (*rt).desc(arms_id);
    if arms.len() % 2 != 0 {
        let e = (*rt).vm.bad_arm_count(arms.len());
        return fail(&mut *rt, e);
    }
    let mut i = 0;
    while i < arms.len() {
        let (pat_id, body_id) = match (&arms[i], &arms[i + 1]) {
            (NatOp::Block(p), NatOp::Block(b)) => (*p, *b),
            _ => {
                let e = (*rt).vm.bad_arm_shape();
                return fail(&mut *rt, e);
            }
        };
        let pat = (*rt).desc(pat_id);
        match (*rt).vm.match_native(&subject, &pat) {
            Ok(None) => {}
            Ok(Some(v)) => {
                (*rt).vm.push_value(v);
                return call_block(rt, body_id, "??");
            }
            Err(e) => return fail(&mut *rt, e),
        }
        i += 2;
    }
    let e = (*rt).vm.unmatched(&subject);
    fail(&mut *rt, e)
}

unsafe fn call_block(rt: *mut Rt, id: u32, op: &str) -> i32 {
    if rt.is_null() || (*rt).err.is_some() {
        return 1;
    }
    if (*rt).depth >= MAX_DEPTH {
        let err = RunError {
            code: STACK_OVERFLOW,
            msg: format!("block invocations nested deeper than the limit ({MAX_DEPTH})"),
            op: Some(op.to_string()),
            hint: "unbounded recursion? give it a base case, or iterate with `;`",
            stack: (*rt).vm.stack.iter().map(|v| v.snapshot_json()).collect(),
            file: (*rt).vm.file.clone(),
            pos: None,
            kind: "run",
            cap: None,
        };
        return fail(&mut *rt, err);
    }
    let f = {
        let funcs = &(*rt).funcs;
        match funcs.get(id as usize).copied() {
            Some(f) => f,
            None => {
                let err = RunError {
                    code: "UNSUPPORTED",
                    msg: format!("native block {id} was not registered"),
                    op: Some(op.to_string()),
                    hint: "",
                    stack: Vec::new(),
                    file: (*rt).vm.file.clone(),
                    pos: None,
                    kind: "run",
                    cap: None,
                };
                return fail(&mut *rt, err);
            }
        }
    };
    (*rt).depth += 1;
    f(rt);
    (*rt).depth -= 1;
    rt_failed(rt)
}

unsafe fn grant(rt: *mut Rt) -> i32 {
    let id = match (*rt).vm.pop_func_id("gr") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let name = match (*rt).vm.pop_cap_name("gr") {
        Ok(name) => name,
        Err(e) => return fail(&mut *rt, e),
    };
    let fresh = (*rt).vm.hold_cap(name.clone());
    let code = call_block(rt, id, "gr");
    if fresh {
        (*rt).vm.drop_cap(&name);
    }
    code
}

unsafe fn vec_ctor(
    rt: *mut Rt,
    name: &'static str,
    count: usize,
    is_float: bool,
    signed: bool,
    bits: u32,
) -> i32 {
    let id = match (*rt).vm.pop_func_id(name) {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let saved = (*rt).vm.take_stack();
    let code = call_block(rt, id, name);
    let produced = (*rt).vm.take_stack();
    (*rt).vm.set_stack(saved);
    if code != 0 {
        return 1;
    }
    match (*rt).vm.build_vec(name, count, is_float, signed, bits, produced) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_finish(rt: *mut Rt) -> i32 {
    if rt.is_null() {
        return 1;
    }
    let owned = Box::from_raw(rt);
    let (code, stdout, _json, human) = owned.emit();
    print!("{stdout}");
    if !human.is_empty() {
        eprintln!("{human}");
    }
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    code
}
