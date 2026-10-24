//! Cranelift backend for Zown.
//!
//! Each IR block becomes a native function. Calls and loops are native.
//! `+ - * _` and the comparisons run in Cranelift when every operand is a
//! plain `int` (overflow is `OVERFLOW`); every other value, and every other
//! operator, still goes through `zown-rt` so the result matches the VM.
//!
//! `??`, `ty`, `gr`, and the vector constructors call back into the native
//! function table. Each block's instruction shape is registered so `??` can
//! read patterns. `write_exe` emits an object and links it with the host
//! `rustc`, whose `main` initializes the allocator and stdout.

use std::collections::HashMap;
use std::fs;
use std::mem::transmute;
use std::path::{Path, PathBuf};
use std::process::Command;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlags, StackSlot, StackSlotData, StackSlotKind, Type, UserFuncName,
    Value as Clif, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::DataDescription;
use cranelift_module::{DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

mod llvm_ir;

use zown_ir::{Instr, IrBlock, IrProgram, lower};
use zown_parser::{ParseError, parse};
use zown_rt::{
    NatOp, Rt, rt_attempt, rt_bind, rt_call, rt_failed, rt_load, rt_match, rt_move, rt_op,
    rt_overflow, rt_pop_func, rt_pop_truthy, rt_push_cap, rt_push_float, rt_push_func, rt_push_int,
    rt_push_simd, rt_push_str, rt_select, rt_take_int, rt_take_ints, rt_take_simd,
};

pub struct ExecResult {
    pub exit: i32,
    pub stdout: String,
    /// JSON `.zerr` when the program compiled and then failed. Empty on success.
    pub stderr: String,
    /// The same failure in the human form `zownc run` prints. Empty on success.
    pub human: String,
}

#[derive(Debug)]
pub enum NativeError {
    Parse(ParseError),
    Unsupported(String),
    /// The object was produced; the host linker (`rustc`) failed.
    Link(String),
}

pub fn execute(src: &str, file: &str) -> Result<ExecResult, NativeError> {
    execute_tuned(src, file, false)
}

/// `fast` selects Cranelift `opt_level=speed` (`zownc native -O`).
pub fn execute_tuned(src: &str, file: &str, fast: bool) -> Result<ExecResult, NativeError> {
    let nodes = parse(src).map_err(NativeError::Parse)?;
    let ir = lower(&nodes);
    run_jit(&ir, file, fast).map_err(NativeError::Unsupported)
}

/// Compile `src` to a native executable at `out`. The binary links the same
/// runtime the JIT uses, so a v0.1 program behaves the same when you run it.
pub use llvm_ir::write_llvm_exe;

pub fn write_exe(src: &str, file: &str, out: &Path) -> Result<(), NativeError> {
    write_exe_tuned(src, file, out, false)
}

pub fn write_exe_tuned(src: &str, file: &str, out: &Path, fast: bool) -> Result<(), NativeError> {
    let nodes = parse(src).map_err(NativeError::Parse)?;
    let ir = lower(&nodes);
    let bytes = compile_object(&ir, fast).map_err(NativeError::Unsupported)?;
    link_exe(&bytes, out, &ir, file).map_err(NativeError::Link)
}

struct Imports {
    push_int: FuncId,
    push_float: FuncId,
    push_str: FuncId,
    push_func: FuncId,
    push_cap: FuncId,
    bind: FuncId,
    load: FuncId,
    mov: FuncId,
    op: FuncId,
    select: FuncId,
    pop_func: FuncId,
    pop_truthy: FuncId,
    call: FuncId,
    failed: FuncId,
    attempt: FuncId,
    mat: FuncId,
    take_ints: FuncId,
    take_int: FuncId,
    overflow: FuncId,
    take_simd: FuncId,
    push_simd: FuncId,
}

fn host_isa(pic: bool, fast: bool) -> Result<cranelift_codegen::isa::OwnedTargetIsa, String> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| e.to_string())?;
    // The in-process JIT rejects PIC. A linked executable is a PIE, and a
    // non-PIC object cannot call the runtime (`R_X86_64_64` against `rt_*`).
    flag_builder
        .set("is_pic", if pic { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    flag_builder
        .set("opt_level", if fast { "speed" } else { "none" })
        .map_err(|e| e.to_string())?;
    let isa_builder = cranelift_native::builder().map_err(|e| e.to_string())?;
    isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| e.to_string())
}

fn compile_program<M: Module>(
    mut module: M,
    ir: &IrProgram,
) -> Result<(M, Vec<FuncId>), String> {
    let ptr = types::I64;
    let strings = declare_strings(&mut module, ir)?;
    let imports = declare_imports(&mut module, ptr)?;
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(ptr));
    let mut ids = Vec::with_capacity(ir.blocks.len());
    for i in 0..ir.blocks.len() {
        let id = module
            .declare_function(&format!("zown_b{i}"), Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        ids.push(id);
    }
    let lens: Vec<u32> = ir.blocks.iter().map(|b| b.0.len() as u32).collect();
    let mut ctx = module.make_context();
    let mut fn_ctx = FunctionBuilderContext::new();
    for (i, block) in ir.blocks.iter().enumerate() {
        compile_block(
            &mut module,
            &mut ctx,
            &mut fn_ctx,
            &imports,
            &strings,
            &lens,
            ids[i],
            block,
            &sig,
        )?;
    }
    Ok((module, ids))
}

fn run_jit(ir: &IrProgram, file: &str, fast: bool) -> Result<ExecResult, String> {
    let isa = host_isa(false, fast)?;
    let mut jb = JITBuilder::with_isa(isa, default_libcall_names());
    for (name, sym) in host_symbols() {
        jb.symbol(name, sym);
    }
    let module = JITModule::new(jb);
    let (mut module, ids) = compile_program(module, ir)?;
    module.finalize_definitions().map_err(|e| e.to_string())?;

    let rt = Box::into_raw(Box::new(Rt::new(file)));
    for (i, id) in ids.iter().enumerate() {
        let p = module.get_finalized_function(*id);
        let f = unsafe { transmute::<_, unsafe extern "C" fn(*mut Rt)>(p) };
        unsafe { (*rt).set_fn(i, f) };
    }
    for (i, block) in ir.blocks.iter().enumerate() {
        unsafe { (*rt).set_desc(i, nats(&block.0)) };
    }
    let main_ptr = module.get_finalized_function(ids[ir.main]);
    let main: unsafe extern "C" fn(*mut Rt) = unsafe { transmute(main_ptr) };
    unsafe { main(rt) };
    let rt = unsafe { Box::from_raw(rt) };
    drop(module);
    let (exit, stdout, stderr, human) = rt.emit();
    Ok(ExecResult { exit, stdout, stderr, human })
}

fn host_symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        ("rt_push_int", rt_push_int as *const u8),
        ("rt_push_float", rt_push_float as *const u8),
        ("rt_push_str", rt_push_str as *const u8),
        ("rt_push_func", rt_push_func as *const u8),
        ("rt_push_cap", rt_push_cap as *const u8),
        ("rt_bind", rt_bind as *const u8),
        ("rt_load", rt_load as *const u8),
        ("rt_move", rt_move as *const u8),
        ("rt_op", rt_op as *const u8),
        ("rt_select", rt_select as *const u8),
        ("rt_pop_func", rt_pop_func as *const u8),
        ("rt_pop_truthy", rt_pop_truthy as *const u8),
        ("rt_call", rt_call as *const u8),
        ("rt_failed", rt_failed as *const u8),
        ("rt_attempt", rt_attempt as *const u8),
        ("rt_match", rt_match as *const u8),
        ("rt_take_ints", rt_take_ints as *const u8),
        ("rt_take_int", rt_take_int as *const u8),
        ("rt_overflow", rt_overflow as *const u8),
        ("rt_take_simd", rt_take_simd as *const u8),
        ("rt_push_simd", rt_push_simd as *const u8),
    ]
}

fn declare_strings<M: Module>(
    module: &mut M,
    ir: &IrProgram,
) -> Result<HashMap<String, (DataId, i64)>, String> {
    let mut texts = Vec::new();
    for block in &ir.blocks {
        for instr in &block.0 {
            match instr {
                Instr::ConstStr(s)
                | Instr::Bind(s)
                | Instr::Load(s)
                | Instr::Move(s)
                | Instr::ConstCap(s)
                | Instr::Op(s) => texts.push(s.clone()),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for text in texts {
        if out.contains_key(&text) {
            continue;
        }
        let n = out.len();
        let id = module
            .declare_data(&format!("zown_s{n}"), Linkage::Local, false, false)
            .map_err(|e| e.to_string())?;
        let mut desc = DataDescription::new();
        let len = text.len() as i64;
        desc.define(text.as_bytes().to_vec().into_boxed_slice());
        module.define_data(id, &desc).map_err(|e| e.to_string())?;
        out.insert(text, (id, len));
    }
    Ok(out)
}

fn declare_imports<M: Module>(module: &mut M, ptr: Type) -> Result<Imports, String> {
    let i32t = types::I32;
    let i64t = types::I64;
    let decl = |module: &mut M,
                name: &str,
                params: &[Type],
                ret: Option<Type>|
     -> Result<FuncId, String> {
        let mut sig = module.make_signature();
        for ty in params {
            sig.params.push(AbiParam::new(*ty));
        }
        if let Some(ty) = ret {
            sig.returns.push(AbiParam::new(ty));
        }
        module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| e.to_string())
    };
    Ok(Imports {
        push_int: decl(module, "rt_push_int", &[ptr, i64t], Some(i32t))?,
        push_float: decl(module, "rt_push_float", &[ptr, i64t], Some(i32t))?,
        push_str: decl(module, "rt_push_str", &[ptr, ptr, i64t], Some(i32t))?,
        push_func: decl(module, "rt_push_func", &[ptr, i32t, i32t], Some(i32t))?,
        push_cap: decl(module, "rt_push_cap", &[ptr, ptr, i64t], Some(i32t))?,
        bind: decl(module, "rt_bind", &[ptr, ptr, i64t], Some(i32t))?,
        load: decl(module, "rt_load", &[ptr, ptr, i64t], Some(i32t))?,
        mov: decl(module, "rt_move", &[ptr, ptr, i64t], Some(i32t))?,
        op: decl(module, "rt_op", &[ptr, ptr, i64t], Some(i32t))?,
        select: decl(module, "rt_select", &[ptr], Some(i32t))?,
        pop_func: decl(module, "rt_pop_func", &[ptr, i32t], Some(i64t))?,
        pop_truthy: decl(module, "rt_pop_truthy", &[ptr], Some(i32t))?,
        call: decl(module, "rt_call", &[ptr, i32t, i32t], Some(i32t))?,
        failed: decl(module, "rt_failed", &[ptr], Some(i32t))?,
        attempt: decl(module, "rt_attempt", &[ptr], Some(i32t))?,
        mat: decl(module, "rt_match", &[ptr], Some(i32t))?,
        take_ints: decl(module, "rt_take_ints", &[ptr, ptr, ptr], Some(i32t))?,
        take_int: decl(module, "rt_take_int", &[ptr, ptr], Some(i32t))?,
        overflow: decl(module, "rt_overflow", &[ptr, ptr, i64t], Some(i32t))?,
        take_simd: decl(module, "rt_take_simd", &[ptr, ptr, i64t, ptr, ptr], Some(i32t))?,
        push_simd: decl(module, "rt_push_simd", &[ptr, i32t, ptr], Some(i32t))?,
    })
}

struct Cx<'a, M: Module> {
    module: &'a mut M,
    b: FunctionBuilder<'a>,
    imports: &'a Imports,
    strings: &'a HashMap<String, (DataId, i64)>,
    lens: &'a [u32],
    rt: cranelift_frontend::Variable,
    fail: cranelift_codegen::ir::Block,
    /// Two `i64` slots. Inlined int ops stage operands here.
    pair: StackSlot,
    /// Two 16-byte SIMD payloads, 16-byte aligned.
    simd: StackSlot,
}

fn compile_block<M: Module>(
    module: &mut M,
    ctx: &mut cranelift_codegen::Context,
    fn_ctx: &mut FunctionBuilderContext,
    imports: &Imports,
    strings: &HashMap<String, (DataId, i64)>,
    lens: &[u32],
    func_id: FuncId,
    block: &IrBlock,
    sig: &cranelift_codegen::ir::Signature,
) -> Result<(), String> {
    ctx.func.signature = sig.clone();
    ctx.func.name = UserFuncName::user(0, func_id.as_u32());
    let mut b = FunctionBuilder::new(&mut ctx.func, fn_ctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let rt_val = b.block_params(entry)[0];
    let rt = b.declare_var(types::I64);
    b.def_var(rt, rt_val);
    let fail = b.create_block();
    let pair = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
    let simd = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 32, 4));
    let mut cx = Cx { module, b, imports, strings, lens, rt, fail, pair, simd };
    cx.prologue();
    for instr in &block.0 {
        cx.emit(instr);
    }
    cx.b.ins().return_(&[]);
    cx.b.switch_to_block(fail);
    cx.b.ins().return_(&[]);
    cx.b.seal_all_blocks();
    cx.b.finalize();
    module.define_function(func_id, ctx).map_err(|e| e.to_string())?;
    module.clear_context(ctx);
    Ok(())
}

impl<'a, M: Module> Cx<'a, M> {
    fn rt(&mut self) -> Clif {
        self.b.use_var(self.rt)
    }

    fn prologue(&mut self) {
        let rt = self.rt();
        let fref = self.module.declare_func_in_func(self.imports.failed, &mut self.b.func);
        let call = self.b.ins().call(fref, &[rt]);
        let status = self.b.inst_results(call)[0];
        let ok = self.b.create_block();
        self.b.ins().brif(status, self.fail, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    fn check(&mut self, status: Clif) {
        let ok = self.b.create_block();
        self.b.ins().brif(status, self.fail, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    fn call_status(&mut self, id: FuncId, args: &[Clif]) -> Clif {
        let rt = self.rt();
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(rt);
        all.extend_from_slice(args);
        let fref = self.module.declare_func_in_func(id, &mut self.b.func);
        let inst = self.b.ins().call(fref, &all);
        let status = self.b.inst_results(inst)[0];
        self.check(status);
        status
    }

    fn str_ptr(&mut self, text: &str) -> (Clif, Clif) {
        let (id, len) = self.strings[text];
        let gv = self.module.declare_data_in_func(id, &mut self.b.func);
        let addr = self.b.ins().global_value(types::I64, gv);
        let n = self.b.ins().iconst(types::I64, len);
        (addr, n)
    }

    fn emit(&mut self, instr: &Instr) {
        match instr {
            Instr::ConstInt(n) => {
                let v = self.b.ins().iconst(types::I64, *n);
                self.call_status(self.imports.push_int, &[v]);
            }
            Instr::ConstFloat(n) => {
                let bits = self.b.ins().iconst(types::I64, n.to_bits() as i64);
                self.call_status(self.imports.push_float, &[bits]);
            }
            Instr::ConstStr(s) => {
                let (p, n) = self.str_ptr(s);
                self.call_status(self.imports.push_str, &[p, n]);
            }
            Instr::ConstBlock(id) => {
                let idv = self.b.ins().iconst(types::I32, *id as i64);
                let nodes = self.b.ins().iconst(types::I32, self.lens[*id] as i64);
                self.call_status(self.imports.push_func, &[idv, nodes]);
            }
            Instr::ConstCap(s) => {
                let (p, n) = self.str_ptr(s);
                self.call_status(self.imports.push_cap, &[p, n]);
            }
            Instr::Bind(s) => {
                let (p, n) = self.str_ptr(s);
                self.call_status(self.imports.bind, &[p, n]);
            }
            Instr::Load(s) if matches!(s.as_str(), "va" | "vs" | "vm") => self.emit_simd(s),
            Instr::Load(s) => {
                let (p, n) = self.str_ptr(s);
                self.call_status(self.imports.load, &[p, n]);
            }
            Instr::Move(s) => {
                let (p, n) = self.str_ptr(s);
                self.call_status(self.imports.mov, &[p, n]);
            }
            Instr::Op(s) => match s.as_str() {
                "+" | "-" | "*" => self.emit_int_binop(s),
                "_" => self.emit_int_neg(),
                "==" | "!=" | "<" | ">" | "<=" | ">=" => self.emit_int_cmp(s),
                _ => {
                    let (p, n) = self.str_ptr(s);
                    self.call_status(self.imports.op, &[p, n]);
                }
            },
            Instr::Select => {
                self.call_status(self.imports.select, &[]);
            }
            Instr::Invoke => self.emit_invoke(0),
            Instr::While => self.emit_while(),
            Instr::Match => {
                self.call_status(self.imports.mat, &[]);
            }
            Instr::Attempt => {
                self.call_status(self.imports.attempt, &[]);
            }
        }
    }

    /// `va`/`vs`/`vm` on two matching vectors. A shadowed name, or any other
    /// stack shape, falls through to `rt_load` so the VM reports the error.
    fn emit_simd(&mut self, word: &str) {
        let a_addr = self.simd_addr(0);
        let b_addr = self.simd_addr(16);
        let (np, nl) = self.str_ptr(word);
        let kind = self.call_ret(self.imports.take_simd, &[np, nl, a_addr, b_addr]);
        let kind_v = self.b.declare_var(types::I32);
        self.b.def_var(kind_v, kind);
        let zero = self.b.ins().iconst(types::I32, 0);
        let miss = self.b.ins().icmp(IntCC::Equal, kind, zero);
        let slow = self.b.create_block();
        let fast = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().brif(miss, slow, &[], fast, &[]);

        self.b.switch_to_block(slow);
        let (np, nl) = self.str_ptr(word);
        self.call_status(self.imports.load, &[np, nl]);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(fast);
        let rest = self.b.create_block();
        self.emit_simd_kind(word, kind_v, 1, types::I32X4, false, done);
        if word != "vm" {
            self.emit_simd_kind(word, kind_v, 2, types::I8X16, false, done);
        }
        self.emit_simd_kind(word, kind_v, 3, types::F32X4, true, done);
        self.emit_simd_kind(word, kind_v, 4, types::F64X2, true, done);
        self.b.ins().jump(rest, &[]);
        self.b.switch_to_block(rest);
        self.b.ins().jump(self.fail, &[]);
        self.b.switch_to_block(done);
    }

    fn emit_simd_kind(
        &mut self,
        word: &str,
        kind_v: cranelift_frontend::Variable,
        want: i64,
        ty: Type,
        float: bool,
        done: cranelift_codegen::ir::Block,
    ) {
        let kind = self.b.use_var(kind_v);
        let imm = self.b.ins().iconst(types::I32, want);
        let eq = self.b.ins().icmp(IntCC::Equal, kind, imm);
        let hit = self.b.create_block();
        let next = self.b.create_block();
        self.b.ins().brif(eq, hit, &[], next, &[]);

        self.b.switch_to_block(hit);
        let a_addr = self.simd_addr(0);
        let b_addr = self.simd_addr(16);
        let a = self.b.ins().load(ty, MemFlags::trusted(), a_addr, 0);
        let b = self.b.ins().load(ty, MemFlags::trusted(), b_addr, 0);
        let result = if float {
            match word {
                "va" => self.b.ins().fadd(a, b),
                "vs" => self.b.ins().fsub(a, b),
                _ => self.b.ins().fmul(a, b),
            }
        } else {
            match word {
                "va" => self.b.ins().iadd(a, b),
                "vs" => self.b.ins().isub(a, b),
                _ => self.b.ins().imul(a, b),
            }
        };
        let store_at = self.simd_addr(0);
        self.b.ins().store(MemFlags::trusted(), result, store_at, 0);
        let k = self.b.ins().iconst(types::I32, want);
        self.call_status(self.imports.push_simd, &[k, store_at]);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(next);
    }

    fn simd_addr(&mut self, offset: i32) -> Clif {
        self.b.ins().stack_addr(types::I64, self.simd, offset)
    }

    /// `+ - *` on two ints. Anything else (float, big, a type error, underflow)
    /// falls through to `rt_op`, which is the VM.
    fn emit_int_binop(&mut self, op: &str) {
        let a_addr = self.pair_addr(0);
        let b_addr = self.pair_addr(8);
        let status = self.call_ret(self.imports.take_ints, &[a_addr, b_addr]);
        let zero = self.b.ins().iconst(types::I32, 0);
        let is_int = self.b.ins().icmp(IntCC::Equal, status, zero);
        let fast = self.b.create_block();
        let slow = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().brif(is_int, fast, &[], slow, &[]);

        self.b.switch_to_block(slow);
        self.emit_vm_op(op);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(fast);
        let a_at = self.pair_addr(0);
        let b_at = self.pair_addr(8);
        let a = self.b.ins().load(types::I64, MemFlags::trusted(), a_at, 0);
        let b = self.b.ins().load(types::I64, MemFlags::trusted(), b_at, 0);
        let (result, of) = match op {
            "+" => self.b.ins().sadd_overflow(a, b),
            "-" => self.b.ins().ssub_overflow(a, b),
            _ => self.b.ins().smul_overflow(a, b),
        };
        self.finish_int(op, result, of, done);
    }

    /// `_` on an int. `i64::MIN` negated does not fit.
    fn emit_int_neg(&mut self) {
        let addr = self.pair_addr(0);
        let status = self.call_ret(self.imports.take_int, &[addr]);
        let zero = self.b.ins().iconst(types::I32, 0);
        let is_int = self.b.ins().icmp(IntCC::Equal, status, zero);
        let fast = self.b.create_block();
        let slow = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().brif(is_int, fast, &[], slow, &[]);

        self.b.switch_to_block(slow);
        self.emit_vm_op("_");
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(fast);
        let a_at = self.pair_addr(0);
        let a = self.b.ins().load(types::I64, MemFlags::trusted(), a_at, 0);
        let z = self.b.ins().iconst(types::I64, 0);
        let (result, of) = self.b.ins().ssub_overflow(z, a);
        self.finish_int("_", result, of, done);
    }

    /// Comparisons of two ints push `1` or `0`. Other pairs use the VM, which
    /// also compares strings, floats, and bigs.
    fn emit_int_cmp(&mut self, op: &str) {
        let a_addr = self.pair_addr(0);
        let b_addr = self.pair_addr(8);
        let status = self.call_ret(self.imports.take_ints, &[a_addr, b_addr]);
        let zero = self.b.ins().iconst(types::I32, 0);
        let is_int = self.b.ins().icmp(IntCC::Equal, status, zero);
        let fast = self.b.create_block();
        let slow = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().brif(is_int, fast, &[], slow, &[]);

        self.b.switch_to_block(slow);
        self.emit_vm_op(op);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(fast);
        let a_at = self.pair_addr(0);
        let b_at = self.pair_addr(8);
        let a = self.b.ins().load(types::I64, MemFlags::trusted(), a_at, 0);
        let b = self.b.ins().load(types::I64, MemFlags::trusted(), b_at, 0);
        let cc = match op {
            "==" => IntCC::Equal,
            "!=" => IntCC::NotEqual,
            "<" => IntCC::SignedLessThan,
            ">" => IntCC::SignedGreaterThan,
            "<=" => IntCC::SignedLessThanOrEqual,
            _ => IntCC::SignedGreaterThanOrEqual,
        };
        let bit = self.b.ins().icmp(cc, a, b);
        let wide = self.b.ins().uextend(types::I64, bit);
        self.call_status(self.imports.push_int, &[wide]);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
    }

    fn pair_addr(&mut self, offset: i32) -> Clif {
        self.b.ins().stack_addr(types::I64, self.pair, offset)
    }

    fn emit_vm_op(&mut self, op: &str) {
        let (p, n) = self.str_ptr(op);
        self.call_status(self.imports.op, &[p, n]);
    }

    /// Push `result` when `of` is clear. Otherwise record `OVERFLOW` and stop.
    fn finish_int(&mut self, op: &str, result: Clif, of: Clif, done: cranelift_codegen::ir::Block) {
        let saved = self.b.declare_var(types::I64);
        self.b.def_var(saved, result);
        let ovf = self.b.create_block();
        let ok = self.b.create_block();
        self.b.ins().brif(of, ovf, &[], ok, &[]);

        self.b.switch_to_block(ovf);
        let (p, n) = self.str_ptr(op);
        self.call_status(self.imports.overflow, &[p, n]);
        self.b.ins().jump(self.fail, &[]);

        self.b.switch_to_block(ok);
        let result = self.b.use_var(saved);
        self.call_status(self.imports.push_int, &[result]);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
    }

    fn emit_invoke(&mut self, which: i64) {
        let which_v = self.b.ins().iconst(types::I32, which);
        let id = self.call_i64(self.imports.pop_func, &[which_v]);
        let neg = self.b.ins().iconst(types::I64, -1);
        let bad = self.b.ins().icmp(IntCC::Equal, id, neg);
        let ok = self.b.create_block();
        self.b.ins().brif(bad, self.fail, &[], ok, &[]);
        self.b.switch_to_block(ok);
        let id32 = self.b.ins().ireduce(types::I32, id);
        self.call_status(self.imports.call, &[id32, which_v]);
    }

    fn emit_while(&mut self) {
        let one = self.b.ins().iconst(types::I32, 1);
        let body = self.call_i64(self.imports.pop_func, &[one]);
        self.bail_if_neg(body);
        let cond = self.call_i64(self.imports.pop_func, &[one]);
        self.bail_if_neg(cond);
        let body_v = self.b.declare_var(types::I64);
        let cond_v = self.b.declare_var(types::I64);
        self.b.def_var(body_v, body);
        self.b.def_var(cond_v, cond);

        let header = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(header);

        let cond_id = self.b.use_var(cond_v);
        let cond32 = self.b.ins().ireduce(types::I32, cond_id);
        let which = self.b.ins().iconst(types::I32, 1);
        self.call_status(self.imports.call, &[cond32, which]);
        let flag = self.call_i32(self.imports.pop_truthy, &[]);
        let two = self.b.ins().iconst(types::I32, 2);
        let err = self.b.ins().icmp(IntCC::Equal, flag, two);
        let after_flag = self.b.create_block();
        self.b.ins().brif(err, self.fail, &[], after_flag, &[]);
        self.b.switch_to_block(after_flag);

        let cont = self.b.create_block();
        let do_body = self.b.create_block();
        self.b.ins().brif(flag, do_body, &[], cont, &[]);

        self.b.switch_to_block(do_body);
        let body_id = self.b.use_var(body_v);
        let body32 = self.b.ins().ireduce(types::I32, body_id);
        let which = self.b.ins().iconst(types::I32, 1);
        self.call_status(self.imports.call, &[body32, which]);
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(cont);
    }

    fn bail_if_neg(&mut self, id: Clif) {
        let neg = self.b.ins().iconst(types::I64, -1);
        let bad = self.b.ins().icmp(IntCC::Equal, id, neg);
        let ok = self.b.create_block();
        self.b.ins().brif(bad, self.fail, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    fn call_i64(&mut self, id: FuncId, args: &[Clif]) -> Clif {
        self.call_ret(id, args)
    }

    fn call_i32(&mut self, id: FuncId, args: &[Clif]) -> Clif {
        self.call_ret(id, args)
    }

    fn call_ret(&mut self, id: FuncId, args: &[Clif]) -> Clif {
        let rt = self.rt();
        let mut all = Vec::with_capacity(args.len() + 1);
        all.push(rt);
        all.extend_from_slice(args);
        let fref = self.module.declare_func_in_func(id, &mut self.b.func);
        let inst = self.b.ins().call(fref, &all);
        self.b.inst_results(inst)[0]
    }
}

fn compile_object(ir: &IrProgram, fast: bool) -> Result<Vec<u8>, String> {
    let isa = host_isa(true, fast)?;
    let builder = ObjectBuilder::new(isa, "zown", default_libcall_names())
        .map_err(|e| e.to_string())?;
    let module = ObjectModule::new(builder);
    let (module, _ids) = compile_program(module, ir)?;
    module.finish().emit().map_err(|e| e.to_string())
}

/// Link a Cranelift object into an executable whose `main` calls block 0 after
/// registering every block with the runtime. `rustc` supplies the standard
/// library so the runtime's allocator and stdout are initialized.
fn link_exe(object: &[u8], out: &Path, ir: &IrProgram, file: &str) -> Result<(), String> {
    // One linker at a time. The directory name is remapped to a constant so
    // two builds of the same source do not embed a pid or a temp path.
    static LINK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LINK_LOCK.lock().map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join(format!("zown-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let obj = dir.join("prog.o");
    let driver = dir.join("driver.rs");
    fs::write(&obj, object).map_err(|e| e.to_string())?;
    fs::write(&driver, driver_source(ir, file)).map_err(|e| e.to_string())?;
    let deps = deps_dir()?;
    let externs = zown_externs()?;
    let rustc = find_rustc()?;
    let out_abs = if out.is_absolute() {
        out.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(out)
    };
    if let Some(parent) = out_abs.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut cmd = rustc_cmd(&rustc);
    cmd.env("SOURCE_DATE_EPOCH", "0")
        .arg(&driver)
        .arg("-o")
        .arg(&out_abs)
        .arg("--edition")
        .arg("2021")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("-C")
        .arg(format!("link-arg={}", obj.display()))
        .arg("-C")
        .arg("debuginfo=0")
        .arg("-C")
        .arg("strip=symbols")
        .arg("-C")
        .arg("codegen-units=1")
        .arg("-C")
        .arg("link-arg=-Wl,--build-id=none")
        .arg("--remap-path-prefix")
        .arg(format!("{dir}=zown-link", dir = dir.display()));
    for ext in &externs {
        cmd.arg("--extern").arg(ext);
    }
    let status = cmd
        .output()
        .map_err(|e| format!("cannot run rustc ({}): {e}", rustc.display()))?;
    let stderr = String::from_utf8_lossy(&status.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);
    if status.status.success() {
        Ok(())
    } else {
        Err(format!("rustc failed to link {}\n{stderr}", out_abs.display()))
    }
}

fn driver_source(ir: &IrProgram, file: &str) -> String {
    let mut externs = String::new();
    let mut sets = String::new();
    for i in 0..ir.blocks.len() {
        externs.push_str(&format!("    fn zown_b{i}(rt: *mut zown_rt::Rt);\n"));
        let ops = nats(&ir.blocks[i].0)
            .iter()
            .map(rust_nat)
            .collect::<Vec<_>>()
            .join(", ");
        sets.push_str(&format!("        (*rt).set_fn({i}, zown_b{i});\n"));
        sets.push_str(&format!("        (*rt).set_desc({i}, vec![{ops}]);\n"));
    }
    let main_id = ir.main;
    format!(
        "fn main() {{\n    let rt = Box::into_raw(Box::new(zown_rt::Rt::new({file:?})));\n    unsafe {{\n{sets}        zown_b{main_id}(rt);\n        let code = zown_rt::rt_finish(rt);\n        std::process::exit(code);\n    }}\n}}\nunsafe extern \"C\" {{\n{externs}}}\n"
    )
}

fn nats(instrs: &[Instr]) -> Vec<NatOp> {
    instrs
        .iter()
        .map(|ins| match ins {
            Instr::ConstInt(n) => NatOp::Int(*n),
            Instr::ConstFloat(n) => NatOp::Float(*n),
            Instr::ConstStr(s) => NatOp::Str(s.clone()),
            Instr::ConstBlock(id) => NatOp::Block(*id as u32),
            Instr::Bind(s) => NatOp::Bind(s.clone()),
            Instr::Move(s) => NatOp::Move(s.clone()),
            Instr::ConstCap(s) => NatOp::Cap(s.clone()),
            Instr::Load(s) => NatOp::Name(s.clone()),
            Instr::Op(s) => NatOp::Op(s.clone()),
            Instr::Invoke => NatOp::Op("@".into()),
            Instr::Select => NatOp::Op("?".into()),
            Instr::While => NatOp::Op(";".into()),
            Instr::Match => NatOp::Op("??".into()),
            Instr::Attempt => NatOp::Name("ty".into()),
        })
        .collect()
}

fn rust_nat(op: &NatOp) -> String {
    let text = |s: &str| format!("{s:?}.to_string()");
    match op {
        NatOp::Int(n) => format!("zown_rt::NatOp::Int({n})"),
        NatOp::Float(n) => format!("zown_rt::NatOp::Float(f64::from_bits({}))", n.to_bits()),
        NatOp::Str(s) => format!("zown_rt::NatOp::Str({})", text(s)),
        NatOp::Name(s) => format!("zown_rt::NatOp::Name({})", text(s)),
        NatOp::Op(s) => format!("zown_rt::NatOp::Op({})", text(s)),
        NatOp::Block(id) => format!("zown_rt::NatOp::Block({id})"),
        NatOp::Cap(s) => format!("zown_rt::NatOp::Cap({})", text(s)),
        NatOp::Bind(s) => format!("zown_rt::NatOp::Bind({})", text(s)),
        NatOp::Move(s) => format!("zown_rt::NatOp::Move({})", text(s)),
    }
}

fn deps_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("zownc has no parent directory")?;
    if parent.ends_with("deps") {
        Ok(parent.to_path_buf())
    } else {
        Ok(parent.join("deps"))
    }
}

fn zown_externs() -> Result<Vec<String>, String> {
    let dir = deps_dir()?;
    let mut best: HashMap<String, (std::time::SystemTime, PathBuf)> = HashMap::new();
    for ent in fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
        let path = ent.map_err(|e| e.to_string())?.path();
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        const RUNTIME: &[&str] = &["zown_rt", "zown_vm", "zown_parser", "zown_ast", "zown_lexer"];
        if !fname.starts_with("libzown_") || !fname.ends_with(".rlib") {
            continue;
        }
        let rest = &fname["lib".len()..fname.len() - ".rlib".len()];
        let name = match rest.rsplit_once('-') {
            Some((name, _)) if RUNTIME.contains(&name) => name.to_string(),
            _ => continue,
        };
        let modified = path
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        match best.get(&name) {
            Some((prev, _)) if *prev > modified => {}
            _ => {
                best.insert(name, (modified, path));
            }
        }
    }
    if !best.contains_key("zown_rt") {
        return Err(format!(
            "libzown_rt-*.rlib not found in {} (build zownc first)",
            dir.display()
        ));
    }
    Ok(best
        .into_iter()
        .map(|(name, (_, path))| format!("{name}={}", path.display()))
        .collect())
}

fn rustc_cmd(rustc: &Path) -> Command {
    let mut cmd = Command::new(rustc);
    // This Windows host has no MSVC linker. Prefer the gnu toolchain when the
    // caller has not picked one. `rust-toolchain.toml` stays host-default.
    if std::env::var_os("RUSTUP_TOOLCHAIN").is_none() {
        if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            let gnu = PathBuf::from(home)
                .join(".rustup")
                .join("toolchains")
                .join("stable-x86_64-pc-windows-gnu");
            if gnu.is_dir() {
                cmd.env("RUSTUP_TOOLCHAIN", "stable-x86_64-pc-windows-gnu");
            }
        }
    }
    cmd
}

fn find_rustc() -> Result<PathBuf, String> {
    if let Ok(path) = which_on_path("rustc") {
        return Ok(path);
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let candidate = PathBuf::from(home).join(".cargo").join("bin").join(rustc_name());
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("rustc not found; it is required to link a native executable".into())
}

fn rustc_name() -> &'static str {
    if cfg!(windows) { "rustc.exe" } else { "rustc" }
}

fn which_on_path(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Ok(exe);
            }
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> ExecResult {
        execute(src, "<test>").unwrap()
    }

    #[test]
    fn adds_and_prints() {
        let r = run("10 20 * .");
        assert_eq!(r.exit, 0);
        assert_eq!(r.stdout, "200\n");
    }

    #[test]
    fn hello() {
        let r = run("[$Hello, World!$.]:h h@");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "Hello, World!\n");
    }

    #[test]
    fn counts() {
        let r = run("0:c [ c 3 < ] [ c . c 1 + :c ] ;");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "0\n1\n2\n");
    }

    #[test]
    fn attempt_div_zero() {
        let r = run("1 :x 0 :y [ x y / ] ty .");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "err(DIV_ZERO)\n");
    }

    #[test]
    fn match_seven() {
        let r = run("7 [ [7] [ . ] [any] [ , $no$ . ] ] ??");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "7\n");
    }

    #[test]
    fn vector_sum() {
        let r = run("[ 1 2 3 4 ] vi vh .");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "10\n");
    }

    #[test]
    fn div_zero() {
        let r = run("1 0 /");
        assert_eq!(r.exit, 1);
        assert!(r.stderr.contains("DIV_ZERO"), "{}", r.stderr);
    }

    #[test]
    fn inlined_int_overflow_is_a_trap() {
        let r = run("9223372036854775807 1 +");
        assert_eq!(r.exit, 1);
        assert!(r.stderr.contains("OVERFLOW"), "{}", r.stderr);
        let r = run("9223372036854775807 _ 1 - _");
        assert_eq!(r.exit, 1, "{}", r.stderr);
        assert!(r.stderr.contains("OVERFLOW"), "{}", r.stderr);
    }

    #[test]
    fn inlined_int_compare() {
        let r = run("5 3 < .  3 5 < .");
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "0\n1\n");
    }

    #[test]
    fn llvm_exe_prints_hello_and_arith() {
        let dir = std::env::temp_dir().join(format!("zown-llvm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let hello = dir.join("hello");
        write_llvm_exe("[$Hello, World!$.]:h h@", "hello.zn", &hello).unwrap();
        let out = std::process::Command::new(&hello).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, b"Hello, World!\n");
        let arith = dir.join("arith");
        write_llvm_exe("0:c [ c 3 < ] [ c . c 1 + :c ] ;", "count.zn", &arith).unwrap();
        let out = std::process::Command::new(&arith).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, b"0\n1\n2\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn optimized_native_matches_the_vm() {
        let r = execute_tuned("10 20 * .", "a.zn", true).unwrap();
        assert_eq!(r.exit, 0, "{}", r.stderr);
        assert_eq!(r.stdout, "200\n");
    }

    #[test]
    fn two_exes_of_the_same_source_match() {
        let dir = std::env::temp_dir().join(format!("zown-det-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.exe");
        let b = dir.join("b.exe");
        let src = "10 20 * .";
        write_exe(src, "arith.zn", &a).unwrap();
        write_exe(src, "arith.zn", &b).unwrap();
        let ba = std::fs::read(&a).unwrap();
        let bb = std::fs::read(&b).unwrap();
        assert_eq!(ba, bb, "native exe is not byte-identical across two builds");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_match_exe() {
        let dir = std::env::temp_dir().join(format!("zown-exe-match-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("match.exe");
        write_exe("7 [ [7] [ . ] [any] [ , $no$ . ] ] ??", "match.zn", &exe).unwrap();
        let out = std::process::Command::new(&exe).output().unwrap();
        assert!(
            out.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "7\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_hello_exe() {
        let dir = std::env::temp_dir().join(format!("zown-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("hello.exe");
        write_exe("[$Hello, World!$.]:h h@", "hello.zn", &exe).unwrap();
        let out = std::process::Command::new(&exe).output().unwrap();
        assert!(
            out.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "Hello, World!\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
