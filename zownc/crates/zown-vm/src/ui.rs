//! Layout tree. Same geometry as `zown/ui.py`.

use std::cell::RefCell;
use std::rc::Rc;

use super::buf::BUF_MAX;
use super::error::{BOUNDS, OVERFLOW, TYPE_MISMATCH};
use super::value::{BufAlloc, Rgb, UiHandler, UiNode};
use super::{RunError, Value, Vm};

fn lum(r: u8, g: u8, b: u8) -> i64 {
    (2126 * r as i64 + 7152 * g as i64 + 722 * b as i64) / 1000
}

fn contrast_ok(fg: &Rgb, bg: &Rgb) -> bool {
    let a = lum(fg.r, fg.g, fg.b);
    let b = lum(bg.r, bg.g, bg.b);
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 128) * 10 >= (lo + 128) * 45
}

fn ui_val(
    kind: &'static str,
    text: String,
    color: Rgb,
    w: i64,
    h: i64,
    children: Vec<Rc<RefCell<UiNode>>>,
) -> Value {
    Value::Ui(Rc::new(RefCell::new(UiNode {
        kind,
        text,
        color,
        x: 0,
        y: 0,
        w,
        h,
        children,
        handler: None,
    })))
}

impl Vm {
    fn pop_plain(&mut self, op: &str) -> Result<i64, RunError> {
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

    fn pop_rgb(&mut self, op: &str) -> Result<Rgb, RunError> {
        match self.pop(op)? {
            Value::Color(c) => Ok(c),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a color, got {tn}"),
                    Some(op.to_string()),
                    "build one with r g b co",
                ))
            }
        }
    }

    fn channel(&mut self, op: &str) -> Result<u8, RunError> {
        let n = self.pop_plain(op)?;
        if !(0..=255).contains(&n) {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` channel {n} is outside 0..255"),
                Some(op.to_string()),
                "",
            ));
        }
        Ok(n as u8)
    }

    pub(super) fn b_co(&mut self) -> Result<(), RunError> {
        let b = self.channel("co")?;
        let g = self.channel("co")?;
        let r = self.channel("co")?;
        self.push(Value::Color(Rgb { r, g, b }));
        Ok(())
    }

    fn text_node(&mut self, kind: &'static str) -> Result<(), RunError> {
        let color = self.pop_rgb(kind)?;
        let size = self.pop_plain(kind)?;
        if size < 1 {
            return Err(self.err(
                BOUNDS,
                format!("`{kind}` size must be >= 1"),
                Some(kind.to_string()),
                "",
            ));
        }
        let text = match self.pop(kind)? {
            Value::Str(s) if !s.contains('\n') => s,
            Value::Str(_) => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{kind}` expected one line of text, got a string with a newline"),
                    Some(kind.to_string()),
                    "",
                ))
            }
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{kind}` expected one line of text, got {tn}"),
                    Some(kind.to_string()),
                    "",
                ));
            }
        };
        let n = text.chars().count() as i64;
        let button = kind == "bt" || kind == "bh";
        let (w, h, stored) = if button {
            (n * size + size * 2, size * 2, "bt")
        } else {
            (n * size, size, "tx")
        };
        self.push(ui_val(stored, text, color, w, h, Vec::new()));
        Ok(())
    }

    pub(super) fn b_tx(&mut self) -> Result<(), RunError> {
        self.text_node("tx")
    }

    pub(super) fn b_bt(&mut self) -> Result<(), RunError> {
        self.text_node("bt")
    }

    pub(super) fn b_bh(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("bh")?;
        self.seal_button(UiHandler::Ast(body))
    }

    /// `text_node("bh")` then attach `handler`. The block is already popped.
    pub fn seal_button(&mut self, handler: UiHandler) -> Result<(), RunError> {
        self.text_node("bh")?;
        if let Some(Value::Ui(n)) = self.stack.last() {
            n.borrow_mut().handler = Some(handler);
        }
        Ok(())
    }

    pub fn finish_box(
        &mut self,
        op: &str,
        kind: &'static str,
        color: Rgb,
        gap: i64,
        pad: i64,
        kids: Vec<Value>,
    ) -> Result<(), RunError> {
        if pad < 0 || gap < 0 {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` pad and gap must be >= 0"),
                Some(op.to_string()),
                "",
            ));
        }
        let mut nodes = Vec::with_capacity(kids.len());
        for kid in kids {
            match kid {
                Value::Ui(n) => nodes.push(n),
                other => {
                    let tn = other.type_name();
                    return Err(self.err(
                        TYPE_MISMATCH,
                        format!("`{op}` child is a {tn}, not a node"),
                        Some(op.to_string()),
                        "the block must leave text, buttons, or boxes",
                    ));
                }
            }
        }
        let (w, h) = place(kind, &nodes, pad, gap);
        self.push(ui_val(kind, String::new(), color, w, h, nodes));
        Ok(())
    }

    /// After the block id is popped: pad, then gap, then the color.
    pub fn take_box_frame(&mut self, op: &str) -> Result<(Rgb, i64, i64), RunError> {
        let pad = self.pop_plain(op)?;
        let gap = self.pop_plain(op)?;
        let color = self.pop_rgb(op)?;
        Ok((color, gap, pad))
    }

    pub(super) fn b_box(&mut self, row: bool) -> Result<(), RunError> {
        let op = if row { "rw" } else { "cm" };
        let body = self.pop_block(op)?;
        let pad = self.pop_plain(op)?;
        let gap = self.pop_plain(op)?;
        let color = self.pop_rgb(op)?;
        let saved = std::mem::take(&mut self.stack);
        let ran = self.invoke(body, op);
        let kids = std::mem::replace(&mut self.stack, saved);
        ran?;
        self.finish_box(op, op, color, gap, pad, kids)
    }

    pub(super) fn b_ly(&mut self) -> Result<(), RunError> {
        let node = match self.pop("ly")? {
            Value::Ui(n) => n,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`ly` expected a node, got {tn}"),
                    Some("ly".into()),
                    "build a tree with tx, bt, rw, or cm",
                ));
            }
        };
        let text = dump(&node.borrow(), 0, 0, None, 0);
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_pk(&mut self) -> Result<(), RunError> {
        if let Some(hit) = self.pointer_handler()? {
            self.fire(hit, "pk")?;
        }
        Ok(())
    }

    pub(super) fn b_ky(&mut self) -> Result<(), RunError> {
        if let Some(hit) = self.key_handler()? {
            self.fire(hit, "ky")?;
        }
        Ok(())
    }

    /// Pop `node x y` and return the handler of the deepest button that contains
    /// the point. A miss or a button with no handler is `Ok(None)`.
    pub fn pointer_handler(&mut self) -> Result<Option<UiHandler>, RunError> {
        let y = self.pop_plain("pk")?;
        let x = self.pop_plain("pk")?;
        let node = self.pop_node("pk")?;
        Ok(search(&node, 0, 0, &|n, ox, oy| {
            n.kind == "bt" && ox <= x && x < ox + n.w && oy <= y && y < oy + n.h
        })
        .flatten())
    }

    /// Pop `node key` and return the handler of the first button with that label.
    pub fn key_handler(&mut self) -> Result<Option<UiHandler>, RunError> {
        let key = match self.pop("ky")? {
            Value::Str(s) if !s.contains('\n') => s,
            Value::Str(_) => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`ky` expected one line of text, got a string with a newline".into(),
                    Some("ky".into()),
                    "",
                ))
            }
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`ky` expected one line of text, got {tn}"),
                    Some("ky".into()),
                    "",
                ));
            }
        };
        let node = self.pop_node("ky")?;
        Ok(search(&node, 0, 0, &|n, _, _| n.kind == "bt" && n.text == key).flatten())
    }

    pub(super) fn b_an(&mut self) -> Result<(), RunError> {
        let n = self.pop_plain("an")?;
        if n < 0 {
            return Err(self.err(
                BOUNDS,
                "`an` tick count must be >= 0".into(),
                Some("an".into()),
                "",
            ));
        }
        let node = self.pop_node("an")?;
        let too_wide = node.borrow().w > i64::MAX - n;
        if too_wide {
            return Err(self.err(
                OVERFLOW,
                "`an` width does not fit int".into(),
                Some("an".into()),
                "",
            ));
        }
        node.borrow_mut().w += n;
        self.push(Value::Ui(node));
        Ok(())
    }

    pub(super) fn b_gp(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("gp")?;
        let (w, h) = self.gp_dims()?;
        let saved = std::mem::take(&mut self.stack);
        let ran = self.invoke(body, "gp");
        let left = std::mem::replace(&mut self.stack, saved);
        ran?;
        self.finish_gpu(w, h, left)
    }

    /// Pop `h` then `w` and refuse a picture the buffer budget will not hold.
    /// The shader block is already off the stack.
    pub fn gp_dims(&mut self) -> Result<(i64, i64), RunError> {
        let h = self.pop_plain("gp")?;
        let w = self.pop_plain("gp")?;
        if w < 1 || h < 1 {
            return Err(self.err(
                BOUNDS,
                "`gp` width and height must be >= 1".into(),
                Some("gp".into()),
                "",
            ));
        }
        match w.checked_mul(h) {
            Some(n) if n <= BUF_MAX / 3 => Ok((w, h)),
            _ => Err(self.err(
                BOUNDS,
                "`gp` picture is larger than the buffer budget".into(),
                Some("gp".into()),
                "",
            )),
        }
    }

    /// Fill an exclusive RGB buffer from the one color `left` holds.
    pub fn finish_gpu(&mut self, w: i64, h: i64, left: Vec<Value>) -> Result<(), RunError> {
        let color = match left.as_slice() {
            [Value::Color(c)] => *c,
            _ => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`gp` shader must leave one color".into(),
                    Some("gp".into()),
                    "the block runs on a fresh stack and leaves r g b co",
                ))
            }
        };
        let pixels = (w as usize).saturating_mul(h as usize);
        let nbytes = pixels.saturating_mul(3);
        let mut data = vec![0u8; nbytes];
        for px in data.chunks_exact_mut(3) {
            px[0] = color.r;
            px[1] = color.g;
            px[2] = color.b;
        }
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(data),
                n: nbytes,
            })),
            exclusive: true,
        });
        Ok(())
    }

    fn pop_node(&mut self, op: &str) -> Result<Rc<RefCell<UiNode>>, RunError> {
        match self.pop(op)? {
            Value::Ui(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a node, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn fire(&mut self, hit: UiHandler, op: &str) -> Result<(), RunError> {
        match hit {
            UiHandler::Ast(body) => self.invoke(body, op),
            UiHandler::Native(_) => Err(self.err(
                super::error::UNSUPPORTED,
                format!("`{op}` native handler is dispatched by the runtime"),
                Some(op.to_string()),
                "",
            )),
        }
    }
}

fn place(kind: &str, children: &[Rc<RefCell<UiNode>>], pad: i64, gap: i64) -> (i64, i64) {
    if kind == "rw" {
        let mut x = pad;
        let mut max_h = 0;
        for ch in children {
            let mut c = ch.borrow_mut();
            c.x = x;
            c.y = pad;
            x += c.w + gap;
            if c.h > max_h {
                max_h = c.h;
            }
        }
        if !children.is_empty() {
            x -= gap;
        } else {
            x = pad;
        }
        let h = if children.is_empty() { 0 } else { max_h } + pad * 2;
        return (x + pad, h);
    }
    let mut y = pad;
    let mut max_w = 0;
    for ch in children {
        let mut c = ch.borrow_mut();
        c.x = pad;
        c.y = y;
        y += c.h + gap;
        if c.w > max_w {
            max_w = c.w;
        }
    }
    if !children.is_empty() {
        y -= gap;
    } else {
        y = pad;
    }
    let w = if children.is_empty() { 0 } else { max_w } + pad * 2;
    (w, y + pad)
}

fn dump(node: &UiNode, ox: i64, oy: i64, parent: Option<&Rgb>, depth: usize) -> String {
    let black = Rgb { r: 0, g: 0, b: 0 };
    let bg = parent.unwrap_or(&black);
    let indent = "  ".repeat(depth);
    if node.kind == "rw" || node.kind == "cm" {
        let mut lines = vec![format!("{indent}{} {ox} {oy} {} {}", node.kind, node.w, node.h)];
        for ch in &node.children {
            let c = ch.borrow();
            lines.push(dump(&c, ox + c.x, oy + c.y, Some(&node.color), depth + 1));
        }
        return lines.join("\n");
    }
    let mark = if contrast_ok(&node.color, bg) { "ok" } else { "low" };
    format!(
        "{indent}{} {ox} {oy} {} {} {mark} {}",
        node.kind, node.w, node.h, node.text
    )
}

/// `None` is a miss. `Some(None)` is a button with no handler, which stops the
/// search. `Some(Some(h))` runs `h`.
fn search(
    node: &Rc<RefCell<UiNode>>,
    ox: i64,
    oy: i64,
    pred: &impl Fn(&UiNode, i64, i64) -> bool,
) -> Option<Option<UiHandler>> {
    let children: Vec<_> = node.borrow().children.clone();
    for ch in &children {
        let (cx, cy) = {
            let c = ch.borrow();
            (c.x, c.y)
        };
        if let Some(found) = search(ch, ox + cx, oy + cy, pred) {
            return Some(found);
        }
    }
    let n = node.borrow();
    if pred(&n, ox, oy) {
        Some(n.handler.clone())
    } else {
        None
    }
}
