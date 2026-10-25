//! Boot images for a Zown program that only prints.
//!
//! `image` is a 512-byte real-mode sector. It writes the text into the VGA
//! buffer at `0xB8000` and then runs `int 0x20`. `kern_image` is the long-mode
//! kernel: paging, one timer IRQ, and a reported page fault. The sector is
//! not that kernel.

mod kern;

pub use kern::image as kern_image;

use std::collections::HashMap;

use zown_ast::NodeKind;
use zown_parser::parse;

const ORIGIN: usize = 0x7C00;
const SECTOR: usize = 512;
/// Rows of 80 cells, two bytes each. A longer print does not fit this sector.
const ROW_BYTES: u16 = 160;

pub fn screen(src: &str) -> Result<String, String> {
    let nodes = parse(src).map_err(|e| e.msg)?;
    let mut out = String::new();
    let mut i = 0;
    while i < nodes.len() {
        let text = match &nodes[i].kind {
            NodeKind::Str(s) => s,
            _ => return Err(only_prints()),
        };
        let newline = match nodes.get(i + 1).map(|n| &n.kind) {
            Some(NodeKind::Name(name)) if name == "pr" => false,
            Some(NodeKind::Op(op)) if op == "." => true,
            _ => return Err(only_prints()),
        };
        if text.bytes().any(|b| b == 0) {
            return Err("a bare image cannot print a NUL".into());
        }
        out.push_str(text);
        if newline {
            out.push('\n');
        }
        i += 2;
    }
    Ok(out)
}

/// Boot sector. 512 bytes, signature `0xAA55`.
pub fn image(src: &str) -> Result<Vec<u8>, String> {
    let text = screen(src)?;
    let (mut code, msg_at) = stub();
    if code.len() != msg_at {
        return Err("boot stub did not end at the message".into());
    }
    if code.len() + text.len() + 1 > SECTOR - 2 {
        return Err("the text does not fit in a boot sector".into());
    }
    code.extend(text.as_bytes());
    code.push(0);
    code.resize(SECTOR - 2, 0);
    code.push(0x55);
    code.push(0xAA);
    Ok(code)
}

fn only_prints() -> String {
    "a bare image only prints string literals (`$text$ pr` or `$text$ .`)".into()
}

fn stub() -> (Vec<u8>, usize) {
    let mut a = Asm::default();
    // jmp far 0000:start — normalizes CS, whatever the BIOS used.
    a.byte(0xEA);
    a.abs16("start");
    a.bytes(&[0x00, 0x00]);

    a.label("start");
    a.bytes(&[
        0xFA, // cli
        0x31, 0xC0, // xor ax, ax
        0x8E, 0xD8, // mov ds, ax
        0x8E, 0xD0, // mov ss, ax
        0xBC, 0x00, 0x7C, // mov sp, 0x7C00
        0xFC, // cld
        // IVT[0x20] = handler:0000
        0xC7, 0x06, 0x80, 0x00,
    ]);
    a.abs16("handler");
    a.bytes(&[
        0xC7, 0x06, 0x82, 0x00, 0x00, 0x00, // mov word [0x0082], 0
        0xC6, 0x06, 0x00, 0x80, 0x00, // mov byte [0x8000], 0
        0xBE, // mov si, msg
    ]);
    a.abs16("msg");
    a.bytes(&[
        0xB8, 0x00, 0xB8, // mov ax, 0xB800
        0x8E, 0xC0, // mov es, ax
        0x31, 0xFF, // xor di, di
    ]);

    a.label("loop");
    a.bytes(&[0xAC, 0x84, 0xC0]); // lodsb; test al, al
    a.rel8(0x74, "after");
    a.bytes(&[0x3C, 0x0A]); // cmp al, 10
    a.rel8(0x74, "nl");
    a.bytes(&[
        0xB4, 0x07, // mov ah, 7
        0xAB, // stosw
        0xE6, 0xE9, // out 0xe9, al
    ]);
    a.rel8(0xEB, "loop");

    a.label("nl");
    a.bytes(&[
        0xE6,
        0xE9, // out 0xe9, al
        0x89,
        0xF8, // mov ax, di
        0x31,
        0xD2, // xor dx, dx
        0xBB,
        (ROW_BYTES & 0xFF) as u8,
        (ROW_BYTES >> 8) as u8, // mov bx, 160
        0xF7,
        0xF3, // div bx
        0x40, // inc ax
        0xF7,
        0xE3, // mul bx
        0x89,
        0xC7, // mov di, ax
    ]);
    a.rel8(0xEB, "loop");

    a.label("after");
    a.bytes(&[0xCD, 0x20, 0xFA]); // int 0x20; cli
    a.label("halt");
    a.byte(0xF4); // hlt
    a.rel8(0xEB, "halt");

    a.label("handler");
    a.bytes(&[
        0xC6, 0x06, 0x00, 0x80, 0xB1, // mov byte [0x8000], 0xB1
        0xCF, // iret
    ]);
    a.label("msg");
    a.resolve();
    let msg = a.labels["msg"];
    (a.code, msg)
}

#[derive(Default)]
struct Asm {
    code: Vec<u8>,
    labels: HashMap<&'static str, usize>,
    abs: Vec<(usize, &'static str)>,
    rel: Vec<(usize, &'static str)>,
}

impl Asm {
    fn label(&mut self, name: &'static str) {
        self.labels.insert(name, self.code.len());
    }

    fn byte(&mut self, b: u8) {
        self.code.push(b);
    }

    fn bytes(&mut self, b: &[u8]) {
        self.code.extend_from_slice(b);
    }

    fn abs16(&mut self, label: &'static str) {
        self.abs.push((self.code.len(), label));
        self.code.extend_from_slice(&[0, 0]);
    }

    fn rel8(&mut self, opcode: u8, label: &'static str) {
        self.code.push(opcode);
        self.rel.push((self.code.len(), label));
        self.code.push(0);
    }

    fn resolve(&mut self) {
        for (at, name) in &self.abs {
            let ip = ORIGIN + self.labels[name];
            self.code[*at] = (ip & 0xFF) as u8;
            self.code[*at + 1] = (ip >> 8) as u8;
        }
        for (at, name) in &self.rel {
            let next = at + 1;
            let dest = self.labels[name] as isize;
            let rel = dest - next as isize;
            assert!((-128..128).contains(&rel), "jump to {name} is out of range");
            self.code[*at] = rel as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zown_is_a_boot_sector() {
        let img = image("$Zown$ .\n").unwrap();
        assert_eq!(img.len(), 512);
        assert_eq!(&img[510..], &[0x55, 0xAA]);
        assert!(img.windows(4).any(|w| w == b"Zown"));
        assert_eq!(screen("$A$ pr $B$ .\n").unwrap(), "AB\n");
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(image("1 .\n").is_err());
        assert!(image("$a\\0b$ pr\n").is_err());
    }
}
