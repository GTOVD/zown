//! Layout tree. Same geometry as `zown/ui.py`.

use super::error::{BOUNDS, TYPE_MISMATCH};
use super::value::{Rgb, UiNode};
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
        let (w, h) = if kind == "bt" {
            (n * size + size * 2, size * 2)
        } else {
            (n * size, size)
        };
        self.push(Value::Ui(UiNode {
            kind,
            text,
            color,
            x: 0,
            y: 0,
            w,
            h,
            children: Vec::new(),
        }));
        Ok(())
    }

    pub(super) fn b_tx(&mut self) -> Result<(), RunError> {
        self.text_node("tx")
    }

    pub(super) fn b_bt(&mut self) -> Result<(), RunError> {
        self.text_node("bt")
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
        let (w, h) = place(kind, &mut nodes, pad, gap);
        self.push(Value::Ui(UiNode {
            kind,
            text: String::new(),
            color,
            x: 0,
            y: 0,
            w,
            h,
            children: nodes,
        }));
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
        self.push(Value::Str(dump(&node, 0, 0, None, 0)));
        Ok(())
    }
}

fn place(kind: &str, children: &mut [UiNode], pad: i64, gap: i64) -> (i64, i64) {
    if kind == "rw" {
        let mut x = pad;
        let mut max_h = 0;
        for ch in children.iter_mut() {
            ch.x = x;
            ch.y = pad;
            x += ch.w + gap;
            if ch.h > max_h {
                max_h = ch.h;
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
    for ch in children.iter_mut() {
        ch.x = pad;
        ch.y = y;
        y += ch.h + gap;
        if ch.w > max_w {
            max_w = ch.w;
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
            lines.push(dump(ch, ox + ch.x, oy + ch.y, Some(&node.color), depth + 1));
        }
        return lines.join("\n");
    }
    let mark = if contrast_ok(&node.color, bg) { "ok" } else { "low" };
    format!(
        "{indent}{} {ox} {oy} {} {} {mark} {}",
        node.kind, node.w, node.h, node.text
    )
}
