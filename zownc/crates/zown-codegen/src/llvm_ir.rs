//! LLVM IR backend. Each block is a function that calls the same C ABI as the
//! Cranelift backend (`rt_*`). `clang -O2` is the optimizer. Bare metal is a
//! later target (M17); this path is a hosted executable.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zown_ir::{Instr, IrProgram, lower};
use zown_parser::parse;

pub fn emit_llvm(ir: &IrProgram, file: &str) -> String {
    let mut g = Gen::new(ir, file);
    g.emit()
}

struct Gen<'a> {
    ir: &'a IrProgram,
    file: &'a str,
    strings: HashMap<String, (String, usize)>,
    nstr: usize,
}

impl<'a> Gen<'a> {
    fn new(ir: &'a IrProgram, file: &'a str) -> Self {
        Gen {
            ir,
            file,
            strings: HashMap::new(),
            nstr: 0,
        }
    }

    fn intern(&mut self, text: &str) -> (String, usize) {
        if let Some(found) = self.strings.get(text) {
            return found.clone();
        }
        let id = format!("@.s{}", self.nstr);
        self.nstr += 1;
        let len = text.len();
        self.strings.insert(text.to_string(), (id.clone(), len));
        (id, len)
    }

    fn emit(&mut self) -> String {
        let (file_id, file_len) = self.intern(self.file);
        let mut bodies = String::new();
        for (i, block) in self.ir.blocks.iter().enumerate() {
            bodies.push_str(&self.block_fn(i, &block.0));
        }
        let main = self.main_fn(&file_id, file_len);
        let mut globals = String::new();
        let keys: Vec<String> = self.strings.keys().cloned().collect();
        for key in keys {
            let (id, len) = self.strings[&key].clone();
            if key.is_empty() {
                globals.push_str(&format!(
                    "{id} = private unnamed_addr constant [1 x i8] zeroinitializer\n"
                ));
            } else {
                let bytes = llvm_bytes(&key);
                globals.push_str(&format!(
                    "{id} = private unnamed_addr constant [{len} x i8] c\"{bytes}\"\n"
                ));
            }
        }
        let decls = String::from(
            "declare i32 @rt_failed(ptr)\n\
             declare i32 @rt_push_int(ptr, i64)\n\
             declare i32 @rt_push_float(ptr, i64)\n\
             declare i32 @rt_push_str(ptr, ptr, i64)\n\
             declare i32 @rt_push_func(ptr, i32, i32)\n\
             declare i32 @rt_push_cap(ptr, ptr, i64)\n\
             declare i32 @rt_bind(ptr, ptr, i64)\n\
             declare i32 @rt_load(ptr, ptr, i64)\n\
             declare i32 @rt_move(ptr, ptr, i64)\n\
             declare i32 @rt_op(ptr, ptr, i64)\n\
             declare i32 @rt_select(ptr)\n\
             declare i64 @rt_pop_func(ptr, i32)\n\
             declare i32 @rt_pop_truthy(ptr)\n\
             declare i32 @rt_call(ptr, i32, i32)\n\
             declare i32 @rt_attempt(ptr)\n\
             declare i32 @rt_match(ptr)\n\
             declare ptr @rt_box_new(ptr, i64)\n\
             declare void @rt_set_fn(ptr, i64, ptr)\n\
             declare void @rt_desc_begin(ptr, i64)\n\
             declare void @rt_desc_push(ptr, i64, i32, ptr, i64, i64, i64)\n\
             declare i32 @rt_finish(ptr)\n\n",
        );
        format!(
            "; ModuleID = 'zown'\n\
             target triple = \"x86_64-pc-linux-gnu\"\n\n\
             {globals}\n{decls}{bodies}{main}"
        )
    }

    fn main_fn(&mut self, file_id: &str, file_len: usize) -> String {
        let mut s = String::from("define i32 @main() {\n");
        s.push_str(&format!(
            "  %rt = call ptr @rt_box_new(ptr {file_id}, i64 {file_len})\n"
        ));
        for i in 0..self.ir.blocks.len() {
            s.push_str(&format!(
                "  call void @rt_set_fn(ptr %rt, i64 {i}, ptr @zown_b{i})\n"
            ));
            s.push_str(&format!("  call void @rt_desc_begin(ptr %rt, i64 {i})\n"));
            for instr in &self.ir.blocks[i].0 {
                s.push_str(&self.desc_push(i, instr));
            }
        }
        let entry = self.ir.main;
        s.push_str(&format!("  call void @zown_b{entry}(ptr %rt)\n"));
        s.push_str("  %code = call i32 @rt_finish(ptr %rt)\n");
        s.push_str("  ret i32 %code\n}\n");
        s
    }

    fn desc_push(&mut self, id: usize, instr: &Instr) -> String {
        let (kind, text, n, bits) = match instr {
            Instr::ConstInt(v) => (0, String::new(), *v, 0u64),
            Instr::ConstFloat(v) => (1, String::new(), 0, v.to_bits()),
            Instr::ConstStr(t) => (2, t.clone(), 0, 0),
            Instr::Load(t) => (3, t.clone(), 0, 0),
            Instr::Op(t) => (4, t.clone(), 0, 0),
            Instr::ConstBlock(b) => (5, String::new(), *b as i64, 0),
            Instr::ConstCap(t) => (6, t.clone(), 0, 0),
            Instr::Bind(t) => (7, t.clone(), 0, 0),
            Instr::Move(t) => (8, t.clone(), 0, 0),
            Instr::Invoke => (4, "@".into(), 0, 0),
            Instr::Select => (4, "?".into(), 0, 0),
            Instr::While => (4, ";".into(), 0, 0),
            Instr::Match => (4, "??".into(), 0, 0),
            Instr::Attempt => (3, "ty".into(), 0, 0),
        };
        let (sid, len) = self.intern(&text);
        let bits_i = bits as i64;
        format!(
            "  call void @rt_desc_push(ptr %rt, i64 {id}, i32 {kind}, ptr {sid}, i64 {len}, i64 {n}, i64 {bits_i})\n"
        )
    }

    fn block_fn(&mut self, index: usize, instrs: &[Instr]) -> String {
        let mut s = format!("define void @zown_b{index}(ptr %rt) {{\nentry:\n");
        s.push_str("  %f = call i32 @rt_failed(ptr %rt)\n");
        s.push_str("  %fbad = icmp ne i32 %f, 0\n");
        s.push_str("  br i1 %fbad, label %fail, label %s0\n");
        let mut lab = 0usize;
        for instr in instrs {
            s.push_str(&format!("s{lab}:\n"));
            let next = lab + 1;
            s.push_str(&self.instr(instr, next));
            lab = next;
        }
        s.push_str(&format!("s{lab}:\n  ret void\nfail:\n  ret void\n}}\n\n"));
        s
    }

    fn instr(&mut self, instr: &Instr, next: usize) -> String {
        match instr {
            Instr::ConstInt(n) => self.checked(&format!("call i32 @rt_push_int(ptr %rt, i64 {n})"), next),
            Instr::ConstFloat(n) => {
                let bits = n.to_bits() as i64;
                self.checked(&format!("call i32 @rt_push_float(ptr %rt, i64 {bits})"), next)
            }
            Instr::ConstStr(t) => {
                let (id, len) = self.intern(t);
                self.checked(
                    &format!("call i32 @rt_push_str(ptr %rt, ptr {id}, i64 {len})"),
                    next,
                )
            }
            Instr::ConstBlock(id) => {
                let nodes = self.ir.blocks[*id].0.len() as i32;
                self.checked(
                    &format!("call i32 @rt_push_func(ptr %rt, i32 {id}, i32 {nodes})"),
                    next,
                )
            }
            Instr::ConstCap(t) => {
                let (id, len) = self.intern(t);
                self.checked(
                    &format!("call i32 @rt_push_cap(ptr %rt, ptr {id}, i64 {len})"),
                    next,
                )
            }
            Instr::Bind(t) => self.text_call("rt_bind", t, next),
            Instr::Load(t) => self.text_call("rt_load", t, next),
            Instr::Move(t) => self.text_call("rt_move", t, next),
            Instr::Op(t) => self.text_call("rt_op", t, next),
            Instr::Select => self.checked("call i32 @rt_select(ptr %rt)", next),
            Instr::Match => self.checked("call i32 @rt_match(ptr %rt)", next),
            Instr::Attempt => self.checked("call i32 @rt_attempt(ptr %rt)", next),
            Instr::Invoke => self.invoke(0, next),
            Instr::While => self.while_loop(next),
        }
    }

    fn text_call(&mut self, func: &str, text: &str, next: usize) -> String {
        let (id, len) = self.intern(text);
        self.checked(
            &format!("call i32 @{func}(ptr %rt, ptr {id}, i64 {len})"),
            next,
        )
    }

    fn checked(&self, call: &str, next: usize) -> String {
        format!(
            "  %c{next} = {call}\n  %b{next} = icmp ne i32 %c{next}, 0\n  br i1 %b{next}, label %fail, label %s{next}\n"
        )
    }

    fn invoke(&self, which: i32, next: usize) -> String {
        format!(
            "  %id{next} = call i64 @rt_pop_func(ptr %rt, i32 {which})\n\
             \x20 %neg{next} = icmp eq i64 %id{next}, -1\n\
             \x20 br i1 %neg{next}, label %fail, label %go{next}\n\
             go{next}:\n\
             \x20 %id32{next} = trunc i64 %id{next} to i32\n\
             \x20 %c{next} = call i32 @rt_call(ptr %rt, i32 %id32{next}, i32 {which})\n\
             \x20 %b{next} = icmp ne i32 %c{next}, 0\n\
             \x20 br i1 %b{next}, label %fail, label %s{next}\n"
        )
    }

    fn while_loop(&self, next: usize) -> String {
        format!(
            "  %body{next} = call i64 @rt_pop_func(ptr %rt, i32 1)\n\
             \x20 %bodybad{next} = icmp eq i64 %body{next}, -1\n\
             \x20 br i1 %bodybad{next}, label %fail, label %wcond{next}\n\
             wcond{next}:\n\
             \x20 %cond{next} = call i64 @rt_pop_func(ptr %rt, i32 1)\n\
             \x20 %condbad{next} = icmp eq i64 %cond{next}, -1\n\
             \x20 br i1 %condbad{next}, label %fail, label %whead{next}\n\
             whead{next}:\n\
             \x20 %cond32{next} = trunc i64 %cond{next} to i32\n\
             \x20 %cs{next} = call i32 @rt_call(ptr %rt, i32 %cond32{next}, i32 1)\n\
             \x20 %csbad{next} = icmp ne i32 %cs{next}, 0\n\
             \x20 br i1 %csbad{next}, label %fail, label %wflag{next}\n\
             wflag{next}:\n\
             \x20 %flag{next} = call i32 @rt_pop_truthy(ptr %rt)\n\
             \x20 %ferr{next} = icmp eq i32 %flag{next}, 2\n\
             \x20 br i1 %ferr{next}, label %fail, label %wtest{next}\n\
             wtest{next}:\n\
             \x20 %go{next} = icmp ne i32 %flag{next}, 0\n\
             \x20 br i1 %go{next}, label %wbody{next}, label %s{next}\n\
             wbody{next}:\n\
             \x20 %body32{next} = trunc i64 %body{next} to i32\n\
             \x20 %bs{next} = call i32 @rt_call(ptr %rt, i32 %body32{next}, i32 1)\n\
             \x20 %bsbad{next} = icmp ne i32 %bs{next}, 0\n\
             \x20 br i1 %bsbad{next}, label %fail, label %whead{next}\n"
        )
    }
}

fn llvm_bytes(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            32..=126 if *b != b'"' => out.push(*b as char),
            _ => out.push_str(&format!("\\{b:02X}")),
        }
    }
    out
}

pub fn write_llvm_exe(src: &str, file: &str, out: &Path) -> Result<(), String> {
    let nodes = parse(src).map_err(|e| e.msg)?;
    let ir = lower(&nodes);
    let ll = emit_llvm(&ir, file);
    let dir = std::env::temp_dir().join(format!("zown-llvm-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ll_path = dir.join("prog.ll");
    fs::write(&ll_path, ll).map_err(|e| e.to_string())?;
    let lib = rt_archive()?;
    let lib_dir = lib
        .parent()
        .ok_or("staticlib has no parent")?
        .to_path_buf();
    let mut cmd = Command::new("clang");
    cmd.arg("-O2")
        .arg("-o")
        .arg(out)
        .arg(&ll_path)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-Wl,--whole-archive")
        .arg("-lzown_rt")
        .arg("-Wl,--no-whole-archive")
        .arg("-lpthread")
        .arg("-ldl")
        .arg("-lm")
        .arg("-lgcc_s");
    let status = cmd.output().map_err(|e| format!("clang: {e}"))?;
    if !status.status.success() {
        return Err(format!(
            "clang failed\n{}",
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    Ok(())
}

fn rt_archive() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut dir = exe.parent().ok_or("no exe dir")?.to_path_buf();
    if dir.ends_with("deps") {
        dir = dir.parent().ok_or("no parent")?.to_path_buf();
    }
    let path = dir.join("libzown_rt.a");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "missing {} — build it with: cargo rustc -p zown-rt --crate-type staticlib",
            path.display()
        ))
    }
}
