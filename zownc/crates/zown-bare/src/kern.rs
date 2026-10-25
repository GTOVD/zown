//! Long-mode kernel image for a Zown program that only prints.
//!
//! The real-mode loader may use `int 0x13` to read the rest of the floppy.
//! The 64-bit kernel does not execute `int`. It identity-maps the low 2 MiB,
//! prints to the VGA text buffer, counts four PIT ticks, then switches to a
//! page table that leaves physical page `0x5000` unmapped. The load of that
//! page is `#PF`. The handler stores `0xF1` at `0x7E00` and halts. It does
//! not reset the machine.

use std::collections::HashMap;

use super::screen;

const KERNEL: usize = 0x8000;
const SECTOR: usize = 512;
/// Row 24, column 0. The printed line stays above this row.
const STATUS: usize = 24 * 160;
const TICKS: u64 = 4;
/// ~1000 Hz. Four ticks is a few milliseconds, not a wall-clock wait.
const PIT_DIVISOR: u16 = 1193;

pub fn image(src: &str) -> Result<Vec<u8>, String> {
    let text = screen(src)?;
    fits(&text)?;
    let mut kernel = kernel(&text)?;
    if kernel.len() > 16 * SECTOR {
        return Err("the long-mode image does not fit one floppy track".into());
    }
    let sectors = (kernel.len() + SECTOR - 1) / SECTOR;
    kernel.resize(sectors * SECTOR, 0);
    let mut img = boot_sector(sectors as u8);
    img.extend(kernel);
    Ok(img)
}

fn fits(text: &str) -> Result<(), String> {
    let mut di = 0usize;
    for &b in text.as_bytes() {
        if b == b'\n' {
            di = (di / 160 + 1) * 160;
        } else {
            di += 2;
        }
        if di > STATUS {
            return Err("the text reaches the status row".into());
        }
    }
    Ok(())
}

fn boot_sector(sectors: u8) -> Vec<u8> {
    let mut a = Asm::new(0x7C00);
    a.bytes(&[
        0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x00, 0x7C, 0xFC,
    ]);
    // mov [drive], dl
    a.bytes(&[0x88, 0x16]);
    a.abs16("drive");
    a.bytes(&[0xFB, 0xBE, 0x05, 0x00]);
    a.label("retry");
    a.bytes(&[
        0xB8, 0x00, 0x08, // mov ax, 0x0800
        0x8E, 0xC0, // mov es, ax
        0x31, 0xDB, // xor bx, bx
        0xB4, 0x02, // mov ah, 0x02
        0xB0, sectors, // mov al, count
        0xB5, 0x00, // mov ch, 0
        0xB1, 0x02, // mov cl, 2
        0xB6, 0x00, // mov dh, 0
        0x8A, 0x16, // mov dl, [drive]
    ]);
    a.abs16("drive");
    a.bytes(&[0xCD, 0x13]);
    a.rel8(0x73, "ok");
    a.bytes(&[0x30, 0xE4, 0x8A, 0x16]);
    a.abs16("drive");
    a.bytes(&[0xCD, 0x13, 0x4E]);
    a.rel8(0x75, "retry");
    a.bytes(&[0xB0, b'D', 0xBA, 0xE9, 0x00, 0xEE]);
    a.label("dead");
    a.bytes(&[0xF4]);
    a.rel8(0xEB, "dead");
    a.label("ok");
    a.bytes(&[0xFA, 0xEA, 0x00, 0x00, 0x00, 0x08]);
    a.label("drive");
    a.byte(0);
    a.resolve().expect("boot sector labels");
    let mut code = a.code;
    code.resize(SECTOR - 2, 0);
    code.push(0x55);
    code.push(0xAA);
    code
}

fn kernel(text: &str) -> Result<Vec<u8>, String> {
    let mut a = Asm::new(KERNEL);

    // 16-bit entry. CS = 0x0800, so offsets match this image.
    a.bytes(&[
        0xFA, 0xB8, 0x00, 0x08, 0x8E, 0xD8, 0x8E, 0xC0, 0xFC, 0x0F, 0x01, 0x16,
    ]);
    a.off16("gdtr");
    a.bytes(&[
        0x66, 0x0F, 0x20, 0xC0, 0x66, 0x83, 0xC8, 0x01, 0x66, 0x0F, 0x22, 0xC0,
    ]);
    a.bytes(&[0x66, 0xEA]);
    a.lin32("pm");
    a.bytes(&[0x08, 0x00]);

    a.label("pm");
    a.bytes(&[
        0x66, 0xB8, 0x10, 0x00, // mov ax, 0x10
        0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, // ds, es, ss
        0xBC, 0x00, 0x00, 0x07, 0x00, // mov esp, 0x70000
        0xFC, // cld
        0xBF, 0x00, 0x10, 0x00, 0x00, // mov edi, 0x1000
        0xB9, 0x00, 0x0C, 0x00, 0x00, // mov ecx, 3072
        0x31, 0xC0, // xor eax, eax
        0xF3, 0xAB, // rep stosd
        0xC7, 0x05, 0x00, 0x10, 0x00, 0x00, 0x03, 0x20, 0x00, 0x00, // PML4[0] = 0x2003
        0xC7, 0x05, 0x00, 0x20, 0x00, 0x00, 0x03, 0x30, 0x00, 0x00, // PDPT[0] = 0x3003
        0xC7, 0x05, 0x00, 0x30, 0x00, 0x00, 0x83, 0x00, 0x00, 0x00, // PD[0] = 2 MiB page
        0xB8, 0x00, 0x10, 0x00, 0x00, // mov eax, 0x1000
        0x0F, 0x22, 0xD8, // mov cr3, eax
        0x0F, 0x20, 0xE0, // mov eax, cr4
        0x83, 0xC8, 0x20, // or eax, PAE
        0x0F, 0x22, 0xE0, // mov cr4, eax
        0xB9, 0x80, 0x00, 0x00, 0xC0, // mov ecx, EFER
        0x0F, 0x32, // rdmsr
        0x0D, 0x00, 0x01, 0x00, 0x00, // or eax, LME
        0x0F, 0x30, // wrmsr
        0x0F, 0x20, 0xC0, // mov eax, cr0
        0x0D, 0x01, 0x00, 0x00, 0x80, // or eax, PG | PE
        0x0F, 0x22, 0xC0, // mov cr0, eax
        0xEA,
    ]);
    a.lin32("long");
    a.bytes(&[0x18, 0x00]);

    a.label("long");
    a.bytes(&[
        0xFA, 0x66, 0xB8, 0x20, 0x00, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x48, 0xBC, 0x00, 0x00,
        0x07, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rsp, 0x70000
        0xFC,
    ]);
    // Clear the report bytes, then mark that this instruction is 64-bit.
    a.bytes(&[0xC6, 0x04, 0x25, 0x00, 0x7E, 0x00, 0x00, 0x00]);
    a.bytes(&[0xC6, 0x04, 0x25, 0x01, 0x7E, 0x00, 0x00, 0x64]);
    a.bytes(&[
        0x48, 0xC7, 0x04, 0x25, 0x08, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    a.bytes(&[0x31, 0xFF, 0x48, 0xBE]);
    a.lin64("msg");

    a.label("print");
    a.bytes(&[0xAC, 0x84, 0xC0]);
    a.rel8(0x74, "printed");
    a.bytes(&[0x3C, 0x0A]);
    a.rel8(0x74, "nl");
    a.bytes(&[
        0xB4, 0x07, 0x66, 0x89, 0x87, 0x00, 0x80, 0x0B, 0x00, // mov [rdi+0xB8000], ax
        0x66, 0xBA, 0xE9, 0x00, 0xEE, // out 0xE9, al
        0x83, 0xC7, 0x02,
    ]);
    a.rel8(0xEB, "print");

    a.label("nl");
    a.bytes(&[
        0x66, 0xBA, 0xE9, 0x00, 0xEE, 0x89, 0xF8, 0x31, 0xD2, 0xB9, 0xA0, 0x00, 0x00, 0x00, 0xF7,
        0xF1, 0xFF, 0xC0, 0x69, 0xC0, 0xA0, 0x00, 0x00, 0x00, 0x89, 0xC7,
    ]);
    a.rel8(0xEB, "print");

    a.label("printed");
    // IDT at 0xF000. Every vector halts with 0xEE until 14 and 32 are replaced.
    a.bytes(&[0x48, 0xB8]);
    a.lin64("def");
    a.bytes(&[
        0x31, 0xC9, // xor ecx, ecx
        0x48, 0xBF, 0x00, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rdi, 0xF000
    ]);
    a.label("fill");
    store_gate(&mut a);
    a.bytes(&[
        0x48, 0x83, 0xC7, 0x10, // add rdi, 16
        0xFF, 0xC1, // inc ecx
        0x81, 0xF9, 0x00, 0x01, 0x00, 0x00, // cmp ecx, 256
    ]);
    a.rel8(0x72, "fill");

    a.bytes(&[0x48, 0xB8]);
    a.lin64("pf");
    a.bytes(&[0x48, 0xBF, 0xE0, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    store_gate(&mut a);
    a.bytes(&[0x48, 0xB8]);
    a.lin64("timer");
    a.bytes(&[0x48, 0xBF, 0x00, 0xF2, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    store_gate(&mut a);

    a.bytes(&[0x0F, 0x01, 0x1C, 0x25]);
    a.lin32("idtr");

    // PIC: master vector 0x20, IRQ0 only.
    pic(&mut a, 0x20, 0x11);
    pic(&mut a, 0xA0, 0x11);
    pic(&mut a, 0x21, 0x20);
    pic(&mut a, 0xA1, 0x28);
    pic(&mut a, 0x21, 0x04);
    pic(&mut a, 0xA1, 0x02);
    pic(&mut a, 0x21, 0x01);
    pic(&mut a, 0xA1, 0x01);
    pic(&mut a, 0x21, 0xFE);
    pic(&mut a, 0xA1, 0xFF);
    // PIT channel 0, rate generator, divisor for about 1 kHz.
    pic(&mut a, 0x43, 0x34);
    pic(&mut a, 0x40, (PIT_DIVISOR & 0xFF) as u8);
    pic(&mut a, 0x40, (PIT_DIVISOR >> 8) as u8);

    a.bytes(&[0xFB]);
    a.label("wait");
    a.bytes(&[
        0xF4,
        0x48,
        0x8B,
        0x04,
        0x25,
        0x08,
        0x7E,
        0x00,
        0x00,
        0x48,
        0x83,
        0xF8,
        TICKS as u8,
    ]);
    a.rel8(0x72, "wait");
    a.bytes(&[0xFA]);

    // Four pages at 0x20000: PML4, PDPT, PD, PT. Page 5 (0x5000) stays absent.
    a.bytes(&[
        0x48, 0xBF, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rdi, 0x20000
        0x48, 0xC7, 0xC1, 0x00, 0x08, 0x00, 0x00, // mov rcx, 2048
        0x31, 0xC0, 0xF3, 0x48, 0xAB, // rep stosq
        0x48, 0xC7, 0x04, 0x25, 0x00, 0x00, 0x02, 0x00, 0x03, 0x10, 0x02, 0x00, // -> 0x21003
        0x48, 0xC7, 0x04, 0x25, 0x00, 0x10, 0x02, 0x00, 0x03, 0x20, 0x02, 0x00, // -> 0x22003
        0x48, 0xC7, 0x04, 0x25, 0x00, 0x20, 0x02, 0x00, 0x03, 0x30, 0x02, 0x00, // -> 0x23003
        0x31, 0xC0, 0x48, 0xBF, 0x00, 0x30, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    a.label("pt");
    a.bytes(&[0x83, 0xF8, 0x05]);
    a.rel8(0x74, "skip");
    a.bytes(&[
        0x89, 0xC3, 0xC1, 0xE3, 0x0C, 0x83, 0xCB, 0x03, 0x48, 0x89, 0x1F,
    ]);
    a.label("skip");
    a.bytes(&[
        0x48, 0x83, 0xC7, 0x08, 0xFF, 0xC0, 0x3D, 0x00, 0x02, 0x00, 0x00,
    ]);
    a.rel8(0x72, "pt");
    a.bytes(&[
        0x48, 0xC7, 0xC0, 0x00, 0x00, 0x02, 0x00, // mov rax, 0x20000
        0x48, 0x0F, 0x22, 0xD8, // mov cr3, rax
        0xB8, 0x00, 0x50, 0x00, 0x00, // mov eax, 0x5000
        0x8A, 0x00, // mov al, [rax]
        0xC6, 0x04, 0x25, 0x00, 0x7E, 0x00, 0x00, 0xE0, // a present page would land here
        0xFA,
    ]);
    a.label("hang");
    a.bytes(&[0xF4]);
    a.rel8(0xEB, "hang");

    a.label("timer");
    a.bytes(&[
        0x48,
        0x8B,
        0x04,
        0x25,
        0x08,
        0x7E,
        0x00,
        0x00, // mov rax, [0x7E08]
        0x48,
        0xFF,
        0xC0, // inc rax
        0x48,
        0x89,
        0x04,
        0x25,
        0x08,
        0x7E,
        0x00,
        0x00,
        0x48,
        0x83,
        0xF8,
        TICKS as u8,
    ]);
    a.rel8(0x75, "eoi");
    // '4' at row 24, column 0.
    a.bytes(&[0x66, 0xC7, 0x04, 0x25, 0x00, 0x8F, 0x0B, 0x00, b'4', 0x07]);
    a.label("eoi");
    a.bytes(&[0xB0, 0x20, 0x66, 0xBA, 0x20, 0x00, 0xEE, 0x48, 0xCF]);

    a.label("pf");
    a.bytes(&[
        0x48, 0x0F, 0x20, 0xD0, // mov rax, cr2
        0x89, 0x04, 0x25, 0x04, 0x7E, 0x00, 0x00, // mov [0x7E04], eax
        0xC6, 0x04, 0x25, 0x00, 0x7E, 0x00, 0x00, 0xF1, 0x66, 0xC7, 0x04, 0x25, 0x02, 0x8F, 0x0B,
        0x00, b'F', 0x07, 0xFA,
    ]);
    a.label("pfh");
    a.bytes(&[0xF4]);
    a.rel8(0xEB, "pfh");

    a.label("def");
    a.bytes(&[0xC6, 0x04, 0x25, 0x00, 0x7E, 0x00, 0x00, 0xEE, 0xFA]);
    a.label("defh");
    a.bytes(&[0xF4]);
    a.rel8(0xEB, "defh");

    a.label("gdt");
    a.bytes(&[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // null
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xCF, 0x00, // code32
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00, // data32
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xAF, 0x00, // code64
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00, // data64
    ]);
    a.label("gdtr");
    a.bytes(&[39, 0]);
    a.lin32("gdt");

    a.label("idtr");
    a.bytes(&[0xFF, 0x0F, 0x00, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    a.label("msg");
    a.bytes(text.as_bytes());
    a.byte(0);

    a.resolve()?;
    Ok(a.code)
}

fn pic(a: &mut Asm, port: u16, value: u8) {
    a.bytes(&[
        0x66,
        0xBA,
        (port & 0xFF) as u8,
        (port >> 8) as u8,
        0xB0,
        value,
        0xEE,
    ]);
}

/// Write one 16-byte interrupt gate. `rax` is the handler. `rdi` is the slot.
fn store_gate(a: &mut Asm) {
    a.bytes(&[
        0x66, 0x89, 0x07, // mov [rdi], ax
        0x66, 0xC7, 0x47, 0x02, 0x18, 0x00, // selector 0x18
        0x66, 0xC7, 0x47, 0x04, 0x00, 0x8E, // interrupt gate
        0x89, 0xC2, // mov edx, eax
        0xC1, 0xEA, 0x10, // shr edx, 16
        0x66, 0x89, 0x57, 0x06, // mov [rdi+6], dx
        0x48, 0x89, 0xC2, // mov rdx, rax
        0x48, 0xC1, 0xEA, 0x20, // shr rdx, 32
        0x89, 0x57, 0x08, // mov [rdi+8], edx
        0xC7, 0x47, 0x0C, 0x00, 0x00, 0x00, 0x00,
    ]);
}

struct Asm {
    base: usize,
    code: Vec<u8>,
    labels: HashMap<&'static str, usize>,
    abs16: Vec<(usize, &'static str)>,
    off16: Vec<(usize, &'static str)>,
    lin32: Vec<(usize, &'static str)>,
    lin64: Vec<(usize, &'static str)>,
    rel8: Vec<(usize, &'static str)>,
}

impl Asm {
    fn new(base: usize) -> Self {
        Self {
            base,
            code: Vec::new(),
            labels: HashMap::new(),
            abs16: Vec::new(),
            off16: Vec::new(),
            lin32: Vec::new(),
            lin64: Vec::new(),
            rel8: Vec::new(),
        }
    }

    fn label(&mut self, name: &'static str) {
        self.labels.insert(name, self.code.len());
    }

    fn byte(&mut self, b: u8) {
        self.code.push(b);
    }

    fn bytes(&mut self, b: &[u8]) {
        self.code.extend_from_slice(b);
    }

    fn abs16(&mut self, name: &'static str) {
        self.abs16.push((self.code.len(), name));
        self.code.extend_from_slice(&[0, 0]);
    }

    fn off16(&mut self, name: &'static str) {
        self.off16.push((self.code.len(), name));
        self.code.extend_from_slice(&[0, 0]);
    }

    fn lin32(&mut self, name: &'static str) {
        self.lin32.push((self.code.len(), name));
        self.code.extend_from_slice(&[0, 0, 0, 0]);
    }

    fn lin64(&mut self, name: &'static str) {
        self.lin64.push((self.code.len(), name));
        self.code.extend_from_slice(&[0; 8]);
    }

    fn rel8(&mut self, opcode: u8, name: &'static str) {
        self.code.push(opcode);
        self.rel8.push((self.code.len(), name));
        self.code.push(0);
    }

    fn resolve(&mut self) -> Result<(), String> {
        for (at, name) in &self.abs16 {
            let ip = self.base + self.pos(name)?;
            self.code[*at] = (ip & 0xFF) as u8;
            self.code[*at + 1] = (ip >> 8) as u8;
        }
        for (at, name) in &self.off16 {
            let off = self.pos(name)?;
            self.code[*at] = (off & 0xFF) as u8;
            self.code[*at + 1] = (off >> 8) as u8;
        }
        for (at, name) in &self.lin32 {
            let ip = (self.base + self.pos(name)?) as u32;
            self.code[*at..*at + 4].copy_from_slice(&ip.to_le_bytes());
        }
        for (at, name) in &self.lin64 {
            let ip = (self.base + self.pos(name)?) as u64;
            self.code[*at..*at + 8].copy_from_slice(&ip.to_le_bytes());
        }
        for (at, name) in &self.rel8 {
            let next = at + 1;
            let dest = self.pos(name)? as isize;
            let rel = dest - next as isize;
            if !(-128..128).contains(&rel) {
                return Err(format!("jump to {name} does not fit in a byte"));
            }
            self.code[*at] = rel as u8;
        }
        Ok(())
    }

    fn pos(&self, name: &str) -> Result<usize, String> {
        self.labels
            .get(name)
            .copied()
            .ok_or_else(|| format!("missing label {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zown_is_a_long_mode_floppy() {
        let img = image("$Zown$ .\n").unwrap();
        assert!(img.len() > 512 && img.len() % 512 == 0);
        assert_eq!(&img[510..512], &[0x55, 0xAA]);
        assert!(img.windows(4).any(|w| w == b"Zown"));
        assert!(img.len() / 512 - 1 <= 16);
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(image("1 .\n").is_err());
        assert!(image("$a\\0b$ pr\n").is_err());
    }

    #[test]
    fn a_long_line_does_not_cover_the_status_row() {
        let line = "x".repeat(24 * 80 + 1);
        let src = format!("${line}$ pr\n");
        assert!(image(&src).is_err());
    }
}
