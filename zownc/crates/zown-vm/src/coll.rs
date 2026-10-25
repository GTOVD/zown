//! Collections on a fat pointer. Same behavior as `zown/coll.py`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::buf::BUF_MAX;
use super::error::{BOUNDS, TYPE_MISMATCH};
use super::value::Value;
use super::{RunError, Vm};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Elem {
    Int(i64),
    Str(String),
}

impl Elem {
    fn show(&self) -> String {
        match self {
            Elem::Int(n) => n.to_string(),
            Elem::Str(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BNode {
    leaf: bool,
    keys: Vec<(String, Elem)>,
    kids: Vec<BNode>,
}

impl BNode {
    fn new(leaf: bool) -> Self {
        BNode {
            leaf,
            keys: Vec::new(),
            kids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TNode {
    kids: HashMap<char, TNode>,
    end: bool,
}

impl TNode {
    fn new() -> Self {
        TNode {
            kids: HashMap::new(),
            end: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CollStore {
    pub kind: &'static str,
    pub freed: bool,
    items: Vec<Elem>,
    order: Vec<String>,
    pairs: HashMap<String, Elem>,
    set: HashSet<Elem>,
    root: BNode,
    trie: TNode,
    trie_n: usize,
    cap: usize,
    bits: Vec<u8>,
    nbits: usize,
    seq: i64,
    heap: Vec<(i64, i64)>,
}

impl CollStore {
    fn fresh(kind: &'static str) -> Self {
        CollStore {
            kind,
            freed: false,
            items: Vec::new(),
            order: Vec::new(),
            pairs: HashMap::new(),
            set: HashSet::new(),
            root: BNode::new(true),
            trie: TNode::new(),
            trie_n: 0,
            cap: 0,
            bits: Vec::new(),
            nbits: 0,
            seq: 0,
            heap: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        match self.kind {
            "ls" | "qu" | "rg" => self.items.len(),
            "sc" => self.set.len(),
            "mp" => self.order.len(),
            "te" => tree_count(&self.root),
            "ti" => self.trie_n,
            "pq" => self.heap.len(),
            _ => self.nbits,
        }
    }
}

pub fn format_coll(store: &RefCell<CollStore>, exclusive: bool) -> String {
    let Ok(g) = store.try_borrow() else {
        return "coll(?)".into();
    };
    let body = if g.freed {
        "-".to_string()
    } else {
        g.count().to_string()
    };
    let mark = if exclusive { "" } else { "&" };
    format!("{}{mark}({body})", g.kind)
}

fn push_coll(vm: &mut Vm, store: CollStore) {
    vm.push(Value::Coll {
        store: Rc::new(RefCell::new(store)),
        exclusive: true,
    });
}

const PUSH: &[&str] = &["ls", "qu", "rg", "pq", "sc", "bm"];
const POP: &[&str] = &["ls", "qu", "rg", "pq"];

impl Vm {
    fn type_msg(&self, op: &str, msg: String) -> RunError {
        self.err(TYPE_MISMATCH, msg, Some(op.to_string()), "")
    }

    fn top_coll(&self, op: &str) -> Result<Rc<RefCell<CollStore>>, RunError> {
        match self.stack.last() {
            Some(Value::Coll { store, .. }) => Ok(store.clone()),
            Some(other) => {
                let tn = other.type_name();
                Err(self.type_msg(op, format!("`{op}` expected a collection, got {tn}")))
            }
            None => Err(self.err(
                BOUNDS,
                format!("`{op}` needs a collection under the operand"),
                Some(op.to_string()),
                "",
            )),
        }
    }

    fn excl<'a>(
        &self,
        op: &str,
        store: &'a RefCell<CollStore>,
        exclusive: bool,
    ) -> Result<std::cell::RefMut<'a, CollStore>, RunError> {
        if !exclusive {
            return Err(self.type_msg(
                op,
                format!("`{op}` needs the exclusive collection, got a borrow"),
            ));
        }
        let g = store.borrow_mut();
        if g.freed {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` collection was freed"),
                Some(op.to_string()),
                "the exclusive owner was dropped with `,`",
            ));
        }
        Ok(g)
    }

    fn coll_live<'a>(
        &self,
        op: &str,
        store: &'a RefCell<CollStore>,
    ) -> Result<std::cell::Ref<'a, CollStore>, RunError> {
        let g = store.borrow();
        if g.freed {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` collection was freed"),
                Some(op.to_string()),
                "the exclusive owner was dropped with `,`",
            ));
        }
        Ok(g)
    }

    fn elem(&self, op: &str, v: Value) -> Result<Elem, RunError> {
        match v {
            Value::Int(n) => Ok(Elem::Int(n)),
            Value::Str(s) if !s.contains('\n') => Ok(Elem::Str(s)),
            Value::Str(_) => Err(self.type_msg(op, format!("`{op}` expected one line of text"))),
            other => {
                let tn = other.type_name();
                Err(self.type_msg(op, format!("`{op}` expected an int or a string, got {tn}")))
            }
        }
    }

    fn key_str(&self, op: &str, v: Value) -> Result<String, RunError> {
        match v {
            Value::Str(s) if !s.is_empty() && !s.chars().any(|c| c.is_whitespace()) => Ok(s),
            Value::Str(_) => Err(self.type_msg(op, format!("`{op}` expected one word, got a string with whitespace"))),
            other => {
                let tn = other.type_name();
                Err(self.type_msg(op, format!("`{op}` expected one word, got {tn}")))
            }
        }
    }

    fn plain(&self, op: &str, v: Value) -> Result<i64, RunError> {
        match v {
            Value::Int(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.type_msg(op, format!("`{op}` expected an int, got {tn}")))
            }
        }
    }

    fn room(&self, op: &str, n: usize) -> Result<(), RunError> {
        if n as i64 > BUF_MAX {
            Err(self.err(
                BOUNDS,
                format!("`{op}` collection is above the limit {BUF_MAX}"),
                Some(op.to_string()),
                "",
            ))
        } else {
            Ok(())
        }
    }

    fn coll_flag(&self) -> Result<bool, RunError> {
        match self.stack.last() {
            Some(Value::Coll { exclusive, .. }) => Ok(*exclusive),
            _ => Ok(false),
        }
    }

    pub(super) fn b_ls(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("ls"));
        Ok(())
    }
    pub(super) fn b_qu(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("qu"));
        Ok(())
    }
    pub(super) fn b_mp(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("mp"));
        Ok(())
    }
    pub(super) fn b_sc(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("sc"));
        Ok(())
    }
    pub(super) fn b_te(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("te"));
        Ok(())
    }
    pub(super) fn b_ti(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("ti"));
        Ok(())
    }
    pub(super) fn b_pq(&mut self) -> Result<(), RunError> {
        push_coll(self, CollStore::fresh("pq"));
        Ok(())
    }

    pub(super) fn b_rg(&mut self) -> Result<(), RunError> {
        let n = self.pop("rg")?;
        let n = self.plain("rg", n)?;
        if n < 1 {
            return Err(self.err(
                BOUNDS,
                "`rg` capacity must be >= 1".into(),
                Some("rg".into()),
                "",
            ));
        }
        self.room("rg", n as usize)?;
        let mut store = CollStore::fresh("rg");
        store.cap = n as usize;
        push_coll(self, store);
        Ok(())
    }

    pub(super) fn b_bm(&mut self) -> Result<(), RunError> {
        let n = self.pop("bm")?;
        let n = self.plain("bm", n)?;
        if n < 1 {
            return Err(self.err(
                BOUNDS,
                "`bm` bit count must be >= 1".into(),
                Some("bm".into()),
                "",
            ));
        }
        self.room("bm", n as usize)?;
        let mut store = CollStore::fresh("bm");
        store.nbits = n as usize;
        store.bits = vec![0; (n as usize + 7) / 8];
        push_coll(self, store);
        Ok(())
    }

    pub(super) fn b_bv(&mut self) -> Result<(), RunError> {
        let n = self.pop("bv")?;
        let n = self.plain("bv", n)?;
        if n < 1 {
            return Err(self.err(
                BOUNDS,
                "`bv` bit count must be >= 1".into(),
                Some("bv".into()),
                "",
            ));
        }
        self.room("bv", n as usize)?;
        let mut store = CollStore::fresh("bv");
        store.nbits = n as usize;
        store.bits = vec![0; (n as usize + 7) / 8];
        push_coll(self, store);
        Ok(())
    }

    pub(super) fn b_ps(&mut self) -> Result<(), RunError> {
        let v = self.pop("ps")?;
        let store = self.top_coll("ps")?;
        let kind = store.borrow().kind;
        if !PUSH.contains(&kind) {
            return Err(self.type_msg("ps", format!("`ps` cannot push onto a {kind}")));
        }
        let exclusive = self.coll_flag()?;
        let mut g = self.excl("ps", &store, exclusive)?;
        match g.kind {
            "bm" => {
                let key = match v {
                    Value::Str(s) => s,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("ps", format!("`ps` expected one word, got {tn}")));
                    }
                };
                if key.is_empty() || key.chars().any(|c| c.is_whitespace()) {
                    return Err(self.type_msg("ps", "`ps` expected one word, got a string with whitespace".into()));
                }
                let nbits = g.nbits;
                let bit = fnv(&key) % nbits;
                g.bits[bit / 8] |= 1 << (bit % 8);
            }
            "pq" => {
                let n = match v {
                    Value::Int(n) => n,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("ps", format!("`ps` expected an int, got {tn}")));
                    }
                };
                if (g.heap.len() as i64 + 1) > BUF_MAX {
                    return Err(self.err(
                        BOUNDS,
                        format!("`ps` collection is above the limit {BUF_MAX}"),
                        Some("ps".into()),
                        "",
                    ));
                }
                heap_push(&mut g, n);
            }
            "sc" => {
                let el = self.elem("ps", v)?;
                if g.set.insert(el.clone()) {
                    if g.set.len() as i64 > BUF_MAX {
                        g.set.remove(&el);
                        return Err(self.err(
                            BOUNDS,
                            format!("`ps` collection is above the limit {BUF_MAX}"),
                            Some("ps".into()),
                            "",
                        ));
                    }
                }
            }
            "rg" => {
                let el = self.elem("ps", v)?;
                g.items.push(el);
                if g.items.len() > g.cap {
                    g.items.remove(0);
                }
            }
            _ => {
                let el = self.elem("ps", v)?;
                if g.items.len() as i64 + 1 > BUF_MAX {
                    return Err(self.err(
                        BOUNDS,
                        format!("`ps` collection is above the limit {BUF_MAX}"),
                        Some("ps".into()),
                        "",
                    ));
                }
                g.items.push(el);
            }
        }
        Ok(())
    }

    pub(super) fn b_po(&mut self) -> Result<(), RunError> {
        let store = self.top_coll("po")?;
        let kind = store.borrow().kind;
        if !POP.contains(&kind) {
            return Err(self.type_msg("po", format!("`po` cannot pop a {kind}")));
        }
        let exclusive = self.coll_flag()?;
        let mut g = self.excl("po", &store, exclusive)?;
        if g.kind == "pq" {
            if g.heap.is_empty() {
                return Err(self.err(
                    BOUNDS,
                    "`po` priority queue is empty".into(),
                    Some("po".into()),
                    "",
                ));
            }
            let n = heap_pop(&mut g.heap);
            drop(g);
            self.push(Value::Int(n));
            return Ok(());
        }
        if g.items.is_empty() {
            let kind = g.kind;
            return Err(self.err(
                BOUNDS,
                format!("`po` {kind} is empty"),
                Some("po".into()),
                "",
            ));
        }
        let el = if g.kind == "ls" {
            g.items.pop().unwrap()
        } else {
            g.items.remove(0)
        };
        drop(g);
        self.push(elem_value(el));
        Ok(())
    }

    pub(super) fn b_gt(&mut self) -> Result<(), RunError> {
        let key = self.pop("gt")?;
        let store = self.top_coll("gt")?;
        let g = self.coll_live("gt", &store)?;
        match g.kind {
            "ls" => {
                let idx = match key {
                    Value::Int(n) => n,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("gt", format!("`gt` expected an int, got {tn}")));
                    }
                };
                if idx < 0 || idx as usize >= g.items.len() {
                    let span = if g.items.is_empty() {
                        "empty list".to_string()
                    } else {
                        format!("list [0..{}]", g.items.len() - 1)
                    };
                    return Err(self.err(
                        BOUNDS,
                        format!("`gt` index {idx} is outside {span}"),
                        Some("gt".into()),
                        "",
                    ));
                }
                let el = g.items[idx as usize].clone();
                drop(g);
                self.push(elem_value(el));
            }
            "bv" => {
                let idx = match key {
                    Value::Int(n) => n,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("gt", format!("`gt` expected an int, got {tn}")));
                    }
                };
                if idx < 0 || idx as usize >= g.nbits {
                    return Err(self.err(
                        BOUNDS,
                        format!("`gt` index {idx} is outside bits [0..{}]", g.nbits - 1),
                        Some("gt".into()),
                        "",
                    ));
                }
                let bit = (g.bits[idx as usize / 8] >> (idx as usize % 8)) & 1;
                drop(g);
                self.push(Value::Int(bit as i64));
            }
            "mp" => {
                let name = self.key_str("gt", key)?;
                let found = g.pairs.get(&name).cloned();
                drop(g);
                match found {
                    Some(el) => self.push(elem_value(el)),
                    None => {
                        return Err(self.err(
                            BOUNDS,
                            format!("`gt` key {name} is absent"),
                            Some("gt".into()),
                            "",
                        ))
                    }
                }
            }
            "te" => {
                let name = self.key_str("gt", key)?;
                let found = tree_get(&g.root, &name);
                drop(g);
                match found {
                    Some(el) => self.push(elem_value(el)),
                    None => {
                        return Err(self.err(
                            BOUNDS,
                            format!("`gt` key {name} is absent"),
                            Some("gt".into()),
                            "",
                        ))
                    }
                }
            }
            kind => return Err(self.type_msg("gt", format!("`gt` cannot index a {kind}"))),
        }
        Ok(())
    }

    pub(super) fn b_pt(&mut self) -> Result<(), RunError> {
        let top = self.pop("pt")?;
        let is_trie = matches!(
            self.stack.last(),
            Some(Value::Coll { store, .. }) if store.borrow().kind == "ti"
        );
        if is_trie {
            let store = self.top_coll("pt")?;
            let exclusive = self.coll_flag()?;
            let mut g = self.excl("pt", &store, exclusive)?;
            let key = self.key_str("pt", top)?;
            if !trie_has(&g.trie, &key) {
                if g.trie_n as i64 + 1 > BUF_MAX {
                    return Err(self.err(
                        BOUNDS,
                        format!("`pt` collection is above the limit {BUF_MAX}"),
                        Some("pt".into()),
                        "",
                    ));
                }
                trie_put(&mut g.trie, &key);
                g.trie_n += 1;
            }
            return Ok(());
        }
        let key = self.pop("pt")?;
        let store = self.top_coll("pt")?;
        let exclusive = self.coll_flag()?;
        let mut g = self.excl("pt", &store, exclusive)?;
        match g.kind {
            "bv" => {
                let idx = match key {
                    Value::Int(n) => n,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("pt", format!("`pt` expected an int, got {tn}")));
                    }
                };
                let bit = match top {
                    Value::Int(n) => n,
                    other => {
                        let tn = other.type_name();
                        return Err(self.type_msg("pt", format!("`pt` expected an int, got {tn}")));
                    }
                };
                if bit != 0 && bit != 1 {
                    return Err(self.err(
                        BOUNDS,
                        format!("`pt` bit {bit} is not 0 or 1"),
                        Some("pt".into()),
                        "",
                    ));
                }
                if idx < 0 || idx as usize >= g.nbits {
                    return Err(self.err(
                        BOUNDS,
                        format!("`pt` index {idx} is outside bits [0..{}]", g.nbits - 1),
                        Some("pt".into()),
                        "",
                    ));
                }
                let mask = 1u8 << (idx as usize % 8);
                if bit == 1 {
                    g.bits[idx as usize / 8] |= mask;
                } else {
                    g.bits[idx as usize / 8] &= !mask;
                }
            }
            "mp" => {
                let name = self.key_str("pt", key)?;
                let val = self.elem("pt", top)?;
                if !g.pairs.contains_key(&name) {
                    if g.order.len() as i64 + 1 > BUF_MAX {
                        return Err(self.err(
                            BOUNDS,
                            format!("`pt` collection is above the limit {BUF_MAX}"),
                            Some("pt".into()),
                            "",
                        ));
                    }
                    g.order.push(name.clone());
                }
                g.pairs.insert(name, val);
            }
            "te" => {
                let name = self.key_str("pt", key)?;
                let val = self.elem("pt", top)?;
                let fresh = tree_get(&g.root, &name).is_none();
                if fresh && tree_count(&g.root) as i64 + 1 > BUF_MAX {
                    return Err(self.err(
                        BOUNDS,
                        format!("`pt` collection is above the limit {BUF_MAX}"),
                        Some("pt".into()),
                        "",
                    ));
                }
                let root = std::mem::replace(&mut g.root, BNode::new(true));
                g.root = tree_put(root, name, val);
            }
            kind => return Err(self.type_msg("pt", format!("`pt` cannot update a {kind}"))),
        }
        Ok(())
    }

    pub(super) fn b_hs(&mut self) -> Result<(), RunError> {
        let v = self.pop("hs")?;
        let store = self.top_coll("hs")?;
        let g = self.coll_live("hs", &store)?;
        let bit = match g.kind {
            "sc" => {
                let el = self.elem("hs", v)?;
                if g.set.contains(&el) { 1 } else { 0 }
            }
            "ti" => {
                let key = self.key_str("hs", v)?;
                if trie_has(&g.trie, &key) { 1 } else { 0 }
            }
            "bm" => {
                let key = self.key_str("hs", v)?;
                let bit = fnv(&key) % g.nbits;
                ((g.bits[bit / 8] >> (bit % 8)) & 1) as i64
            }
            kind => return Err(self.type_msg("hs", format!("`hs` cannot test a {kind}"))),
        };
        drop(g);
        self.push(Value::Int(bit));
        Ok(())
    }

    pub(super) fn b_sk(&mut self) -> Result<(), RunError> {
        let store = self.top_coll("sk")?;
        let g = self.coll_live("sk", &store)?;
        let text = match g.kind {
            "ls" | "qu" | "rg" => join_elems(&g.items),
            "sc" => {
                let mut ints: Vec<i64> = g.set.iter().filter_map(|e| match e {
                    Elem::Int(n) => Some(*n),
                    _ => None,
                }).collect();
                let mut strs: Vec<String> = g.set.iter().filter_map(|e| match e {
                    Elem::Str(s) => Some(s.clone()),
                    _ => None,
                }).collect();
                ints.sort();
                strs.sort();
                let mut lines: Vec<String> = ints.iter().map(|n| n.to_string()).collect();
                lines.extend(strs);
                lines.join("\n")
            }
            "mp" => g
                .order
                .iter()
                .map(|k| format!("{k} {}", g.pairs[k].show()))
                .collect::<Vec<_>>()
                .join("\n"),
            "te" => {
                let mut rows = Vec::new();
                tree_walk(&g.root, &mut rows);
                rows.iter()
                    .map(|(k, v)| format!("{k} {}", v.show()))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "ti" => {
                let mut rows = Vec::new();
                trie_walk(&g.trie, String::new(), &mut rows);
                rows.join("\n")
            }
            "pq" => {
                let mut heap = g.heap.clone();
                let mut out = Vec::new();
                while !heap.is_empty() {
                    out.push(heap_pop(&mut heap).to_string());
                }
                out.join("\n")
            }
            "bv" => {
                let mut chars = String::new();
                for i in 0..g.nbits {
                    chars.push(if (g.bits[i / 8] >> (i % 8)) & 1 == 1 { '1' } else { '0' });
                }
                chars
            }
            kind => return Err(self.type_msg("sk", format!("`sk` cannot scan a {kind}"))),
        };
        drop(g);
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_px(&mut self) -> Result<(), RunError> {
        let prefix = self.pop("px")?;
        let store = self.top_coll("px")?;
        let g = self.coll_live("px", &store)?;
        if g.kind != "ti" {
            let kind = g.kind;
            return Err(self.type_msg("px", format!("`px` expected a trie, got {kind}")));
        }
        let prefix = match prefix {
            Value::Str(s) if !s.chars().any(|c| c.is_whitespace()) => s,
            Value::Str(_) => {
                return Err(self.type_msg("px", "`px` expected one word, got a string with whitespace".into()))
            }
            other => {
                let tn = other.type_name();
                return Err(self.type_msg("px", format!("`px` expected one word, got {tn}")));
            }
        };
        let mut node = &g.trie;
        for ch in prefix.chars() {
            match node.kids.get(&ch) {
                Some(n) => node = n,
                None => {
                    drop(g);
                    self.push(Value::Str(String::new()));
                    return Ok(());
                }
            }
        }
        let mut rows = Vec::new();
        trie_walk(node, prefix, &mut rows);
        drop(g);
        self.push(Value::Str(rows.join("\n")));
        Ok(())
    }

    pub(super) fn coll_len(&self, store: &RefCell<CollStore>) -> Result<i64, RunError> {
        let g = store.borrow();
        if g.freed {
            return Err(self.err(
                BOUNDS,
                "`ln` collection was freed".into(),
                Some("ln".into()),
                "the exclusive owner was dropped with `,`",
            ));
        }
        Ok(g.count() as i64)
    }
}

fn elem_value(el: Elem) -> Value {
    match el {
        Elem::Int(n) => Value::Int(n),
        Elem::Str(s) => Value::Str(s),
    }
}

fn join_elems(items: &[Elem]) -> String {
    items.iter().map(Elem::show).collect::<Vec<_>>().join("\n")
}

fn fnv(text: &str) -> usize {
    let mut h: u32 = 2166136261;
    for byte in text.as_bytes() {
        h ^= *byte as u32;
        h = h.wrapping_mul(16777619);
    }
    h as usize
}

fn heap_push(store: &mut CollStore, n: i64) {
    store.seq += 1;
    store.heap.push((n, store.seq));
    let mut i = store.heap.len() - 1;
    while i > 0 {
        let p = (i - 1) / 2;
        if store.heap[p] <= store.heap[i] {
            break;
        }
        store.heap.swap(p, i);
        i = p;
    }
}

fn heap_pop(heap: &mut Vec<(i64, i64)>) -> i64 {
    let top = heap[0].0;
    let last = heap.pop().unwrap();
    if !heap.is_empty() {
        heap[0] = last;
        let mut i = 0;
        let n = heap.len();
        loop {
            let left = i * 2 + 1;
            let right = left + 1;
            let mut small = i;
            if left < n && heap[left] < heap[small] {
                small = left;
            }
            if right < n && heap[right] < heap[small] {
                small = right;
            }
            if small == i {
                break;
            }
            heap.swap(i, small);
            i = small;
        }
    }
    top
}

fn trie_put(node: &mut TNode, key: &str) {
    let mut node = node;
    for ch in key.chars() {
        node = node.kids.entry(ch).or_insert_with(TNode::new);
    }
    node.end = true;
}

fn trie_has(node: &TNode, key: &str) -> bool {
    let mut node = node;
    for ch in key.chars() {
        match node.kids.get(&ch) {
            Some(n) => node = n,
            None => return false,
        }
    }
    node.end
}

fn trie_walk(node: &TNode, prefix: String, out: &mut Vec<String>) {
    if node.end {
        out.push(prefix.clone());
    }
    let mut keys: Vec<char> = node.kids.keys().copied().collect();
    keys.sort();
    for ch in keys {
        let mut next = prefix.clone();
        next.push(ch);
        trie_walk(&node.kids[&ch], next, out);
    }
}

fn tree_count(node: &BNode) -> usize {
    node.keys.len() + node.kids.iter().map(tree_count).sum::<usize>()
}

fn tree_get(node: &BNode, key: &str) -> Option<Elem> {
    let mut i = 0;
    while i < node.keys.len() && key > node.keys[i].0.as_str() {
        i += 1;
    }
    if i < node.keys.len() && node.keys[i].0 == key {
        return Some(node.keys[i].1.clone());
    }
    if node.leaf {
        None
    } else {
        tree_get(&node.kids[i], key)
    }
}

fn tree_walk(node: &BNode, out: &mut Vec<(String, Elem)>) {
    if node.leaf {
        out.extend(node.keys.iter().cloned());
        return;
    }
    for (i, kv) in node.keys.iter().enumerate() {
        tree_walk(&node.kids[i], out);
        out.push(kv.clone());
    }
    tree_walk(node.kids.last().unwrap(), out);
}

fn split_child(parent: &mut BNode, i: usize) {
    let (median, z) = {
        let y = &mut parent.kids[i];
        let mut z = BNode::new(y.leaf);
        let median = y.keys[1].clone();
        z.keys = y.keys.split_off(2);
        y.keys.truncate(1);
        if !y.leaf {
            z.kids = y.kids.split_off(2);
            y.kids.truncate(2);
        }
        (median, z)
    };
    parent.kids.insert(i + 1, z);
    parent.keys.insert(i, median);
}

fn insert_nonfull(node: &mut BNode, key: String, val: Elem) {
    if node.leaf {
        if let Some(slot) = node.keys.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = val;
            return;
        }
        node.keys.push((key, val));
        node.keys.sort_by(|a, b| a.0.cmp(&b.0));
        return;
    }
    let mut i = node.keys.len() as isize - 1;
    while i >= 0 && key < node.keys[i as usize].0 {
        i -= 1;
    }
    if i >= 0 && node.keys[i as usize].0 == key {
        node.keys[i as usize].1 = val;
        return;
    }
    let mut i = (i + 1) as usize;
    if node.kids[i].keys.len() == 3 {
        split_child(node, i);
        if key == node.keys[i].0 {
            node.keys[i].1 = val;
            return;
        }
        if key > node.keys[i].0 {
            i += 1;
        }
    }
    insert_nonfull(&mut node.kids[i], key, val);
}

fn tree_put(mut root: BNode, key: String, val: Elem) -> BNode {
    if root.keys.len() == 3 {
        let mut fresh = BNode::new(false);
        fresh.kids.push(root);
        split_child(&mut fresh, 0);
        insert_nonfull(&mut fresh, key, val);
        fresh
    } else {
        insert_nonfull(&mut root, key, val);
        root
    }
}
