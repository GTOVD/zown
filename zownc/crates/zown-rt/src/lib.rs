//! Runtime the Cranelift backend calls. Values and operators go through the
//! reference `Vm`, so a native block and the tree-walker agree. Block *calls*
//! are native function pointers registered with [`Rt::set_fn`].

use std::io::{self, Write};
use std::process::abort;

pub use zown_vm::NatOp;
use zown_vm::{RunError, Slice, Value, Vm, STACK_OVERFLOW};

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

    pub fn clear_desc(&mut self, id: usize) {
        if self.descs.len() <= id {
            self.descs.resize(id + 1, Vec::new());
        }
        self.descs[id].clear();
    }

    pub fn push_desc(&mut self, id: usize, op: NatOp) {
        if self.descs.len() <= id {
            self.descs.resize(id + 1, Vec::new());
        }
        self.descs[id].push(op);
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
    if rt.is_null() || (*rt).err.is_some() {
        1
    } else {
        0
    }
}

/// Pop the top two values when both are `int`. Writes the deeper one to `a_out`
/// and the former top to `b_out`. Returns 0 after popping, or 1 when the pair
/// is not two ints (the stack is left as it was, so the VM operator can run).
#[no_mangle]
pub unsafe extern "C" fn rt_take_ints(rt: *mut Rt, a_out: *mut i64, b_out: *mut i64) -> i32 {
    if rt.is_null() || a_out.is_null() || b_out.is_null() || rt_failed(rt) != 0 {
        return 1;
    }
    let stack = &(*rt).vm.stack;
    if stack.len() < 2 {
        return 1;
    }
    let n = stack.len();
    match (&stack[n - 2], &stack[n - 1]) {
        (Value::Int(a), Value::Int(b)) => {
            let (a, b) = (*a, *b);
            (*rt).vm.stack.pop();
            (*rt).vm.stack.pop();
            *a_out = a;
            *b_out = b;
            0
        }
        _ => 1,
    }
}

/// Pop the top value when it is an `int`. Returns 0 after writing it, or 1
/// when the top is not an int (stack unchanged).
#[no_mangle]
pub unsafe extern "C" fn rt_take_int(rt: *mut Rt, a_out: *mut i64) -> i32 {
    if rt.is_null() || a_out.is_null() || rt_failed(rt) != 0 {
        return 1;
    }
    match (*rt).vm.stack.last() {
        Some(Value::Int(n)) => {
            let n = *n;
            (*rt).vm.stack.pop();
            *a_out = n;
            0
        }
        _ => 1,
    }
}

/// Pop two matching SIMD vectors for `word` (`va`/`vs`/`vm`) into 16-byte
/// buffers. Returns the kind, or 0 when the word should run on the VM.
#[no_mangle]
pub unsafe extern "C" fn rt_take_simd(
    rt: *mut Rt,
    ptr: *const u8,
    len: usize,
    a_out: *mut u8,
    b_out: *mut u8,
) -> i32 {
    if rt.is_null() || a_out.is_null() || b_out.is_null() || rt_failed(rt) != 0 {
        return 0;
    }
    let word = str_at(ptr, len);
    let mut a = [0u8; 16];
    let mut b = [0u8; 16];
    let kind = (*rt).vm.prepare_simd(&word, &mut a, &mut b);
    if kind == 0 {
        return 0;
    }
    std::ptr::copy_nonoverlapping(a.as_ptr(), a_out, 16);
    std::ptr::copy_nonoverlapping(b.as_ptr(), b_out, 16);
    kind
}

/// Push the 16-byte result of an inlined SIMD op.
#[no_mangle]
pub unsafe extern "C" fn rt_push_simd(rt: *mut Rt, kind: i32, ptr: *const u8) -> i32 {
    if rt.is_null() || ptr.is_null() || rt_failed(rt) != 0 {
        return 1;
    }
    let bytes = std::slice::from_raw_parts(ptr, 16);
    (*rt).vm.push_simd(kind, bytes);
    0
}

/// Record `OVERFLOW` for an inlined int operation. The operands are already gone.
#[no_mangle]
pub unsafe extern "C" fn rt_overflow(rt: *mut Rt, ptr: *const u8, len: usize) -> i32 {
    if rt.is_null() {
        return 1;
    }
    let op = str_at(ptr, len);
    let err = (*rt).vm.overflow_int(&op);
    fail(&mut *rt, err)
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
    (*rt)
        .vm
        .stack
        .push(Value::Float(f64::from_bits(bits as u64)));
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
            "rn" => return exec_rn(rt),
            "ts" => return exec_ts(rt),
            "pp" => return exec_pp(rt),
            "vf" => return vec_ctor(rt, "vf", 4, true, true, 32),
            "vd" => return vec_ctor(rt, "vd", 2, true, true, 64),
            "vi" => return vec_ctor(rt, "vi", 4, false, true, 32),
            "vu" => return vec_ctor(rt, "vu", 16, false, false, 8),
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

/// `[body] ts` on the native path. The body is a compiled block.
unsafe fn exec_ts(rt: *mut Rt) -> i32 {
    let id = match (*rt).vm.pop_func_id("ts") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let depth = (*rt).vm.stack.len();
    if call_block(rt, id, "ts") != 0 {
        return 1;
    }
    match (*rt).vm.finish_test("ts", depth) {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

/// `k [body] pp` on the native path.
unsafe fn exec_pp(rt: *mut Rt) -> i32 {
    let id = match (*rt).vm.pop_func_id("pp") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let k = match (*rt).vm.pop_count("pp") {
        Ok(k) => k,
        Err(e) => return fail(&mut *rt, e),
    };
    for _ in 0..k {
        let depth = (*rt).vm.stack.len();
        if call_block(rt, id, "pp") != 0 {
            return 1;
        }
        if let Err(e) = (*rt).vm.finish_test("pp", depth) {
            return fail(&mut *rt, e);
        }
    }
    0
}

/// `k rn` on the native path. The tree-walker has the same steps in `Vm::b_rn`.
/// Lane bodies here are compiled functions, so the calls stay in this runtime
/// where no `&mut Vm` is held across `call_block`.
unsafe fn exec_rn(rt: *mut Rt) -> i32 {
    let k = match (*rt).vm.begin_rn() {
        Ok(k) => k,
        Err(e) => return fail(&mut *rt, e),
    };
    for _ in 0..k {
        let names = (*rt).vm.lane_schedule();
        for name in names {
            let Some(body) = (*rt).vm.copy_body(&name) else {
                continue;
            };
            (*rt).vm.enter_lane(&name);
            let op = format!("~{name}");
            let code = dispatch_slice(rt, body, &op);
            (*rt).vm.leave_lane();
            if code != 0 {
                (*rt).vm.end_rn();
                return 1;
            }
            (*rt).vm.mark_trace(&name);
        }
        if let Some(job) = (*rt).vm.take_job() {
            let code = dispatch_slice(rt, job, "^");
            if code != 0 {
                (*rt).vm.end_rn();
                return 1;
            }
            (*rt).vm.mark_trace("c");
        }
    }
    (*rt).vm.end_rn();
    0
}

unsafe fn dispatch_slice(rt: *mut Rt, body: Slice, op: &str) -> i32 {
    match body {
        Slice::Native(id) => call_block(rt, id, op),
        Slice::Ast(b) => match (*rt).vm.call_ast(b, op) {
            Ok(()) => 0,
            Err(e) => fail(&mut *rt, e),
        },
    }
}

/// Call native block `id`. `which` is `0` for `@` and `1` for `;`.
#[no_mangle]
pub unsafe extern "C" fn rt_call(rt: *mut Rt, id: i32, which: i32) -> i32 {
    let op = if which == 0 { "@" } else { ";" };
    call_block(rt, id as u32, op)
}

/// `[body] ty` — run the body on a fresh stack and push `ok` or `err`.
#[no_mangle]
pub unsafe extern "C" fn rt_attempt(rt: *mut Rt) -> i32 {
    if rt_failed(rt) != 0 {
        return 1;
    }
    if (*rt).vm.is_bound("ty") {
        return match (*rt).vm.apply_name("ty") {
            Ok(()) => 0,
            Err(e) => fail(&mut *rt, e),
        };
    }
    let id = match (*rt).vm.pop_func_id("ty") {
        Ok(id) => id,
        Err(e) => return fail(&mut *rt, e),
    };
    let saved = (*rt).vm.take_stack();
    let code = call_block(rt, id, "ty");
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
    match (*rt)
        .vm
        .build_vec(name, count, is_float, signed, bits, produced)
    {
        Ok(()) => 0,
        Err(e) => fail(&mut *rt, e),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_new(ptr: *const u8, len: usize) -> *mut Rt {
    let file = str_at(ptr, len);
    Box::into_raw(Box::new(Rt::new(&file)))
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_fn(rt: *mut Rt, id: u64, f: unsafe extern "C" fn(*mut Rt)) {
    if !rt.is_null() {
        (*rt).set_fn(id as usize, f);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_desc_begin(rt: *mut Rt, id: u64) {
    if !rt.is_null() {
        (*rt).clear_desc(id as usize);
    }
}

/// Append one instruction to block `id`'s pattern shape.
/// `kind`: 0 int (`n`), 1 float (`bits`), 2 str, 3 name, 4 op, 5 block (`n`),
/// 6 cap, 7 bind, 8 move. Text kinds read `ptr`/`len`.
#[no_mangle]
pub unsafe extern "C" fn rt_desc_push(
    rt: *mut Rt,
    id: u64,
    kind: u32,
    ptr: *const u8,
    len: usize,
    n: i64,
    bits: u64,
) {
    if rt.is_null() {
        return;
    }
    let text = str_at(ptr, len);
    let op = match kind {
        0 => NatOp::Int(n),
        1 => NatOp::Float(f64::from_bits(bits)),
        2 => NatOp::Str(text),
        3 => NatOp::Name(text),
        4 => NatOp::Op(text),
        5 => NatOp::Block(n as u32),
        6 => NatOp::Cap(text),
        7 => NatOp::Bind(text),
        8 => NatOp::Move(text),
        _ => return,
    };
    (*rt).push_desc(id as usize, op);
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
