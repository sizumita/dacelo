// ARM64 (AArch64) instruction encoder for dacelo Gen 2

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
pub enum Cond {
    Eq = 0,
    Ne = 1,
    Hs = 2,
    Lo = 3,
    Mi = 4,
    Pl = 5,
    Ge = 10,
    Lt = 11,
    Gt = 12,
    Le = 13,
}

impl Cond {
    fn invert(self) -> u32 {
        (self as u32) ^ 1
    }
}

pub const R0: u32 = 0;
pub const R8: u32 = 8;
pub const R9: u32 = 9;
pub const R10: u32 = 10;
pub const R11: u32 = 11;
pub const R29_FP: u32 = 29;
pub const R30_LR: u32 = 30;
pub const RSP: u32 = 31;
pub const XZR: u32 = 31;

#[derive(Clone)]
pub enum LitTarget {
    /// external (undefined) symbol, e.g. runtime function
    Extern(String),
    /// locally defined text label
    Label(u32),
    /// GOT slot index (address of slot in __DATA,__got)
    Got(usize),
}

#[derive(Clone, Copy)]
enum FixKind {
    B,
    Bl,
    BCond(u32),
    Cbz(u32),
    Cbnz(u32),
}

struct Fixup {
    pos: usize,
    label: u32,
    kind: FixKind,
}

/// ADRP instruction awaiting page21 patch
struct AdrpRef {
    pos: usize,
    reg: u32,
    slot: usize,
}

/// literal-pool entry (kept for compatibility; no longer emitted)
#[allow(dead_code)]


pub struct CodeBuf {
    pub code: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
    got_slots: Vec<String>,          // ordered unique keys
    got_map: HashMap<String, usize>, // key -> slot index
    adrp_refs: Vec<AdrpRef>,
    pool_base: usize,
}

impl CodeBuf {
    pub fn new() -> CodeBuf {
        CodeBuf {
            code: Vec::new(),
            labels: vec![None; 8],
            fixups: Vec::new(),
            got_slots: Vec::new(),
            got_map: HashMap::new(),
            adrp_refs: Vec::new(),
            pool_base: 0,
        }
    }

    pub fn new_label(&mut self) -> u32 {
        self.labels.push(None);
        (self.labels.len() - 1) as u32
    }

    pub fn bind_label(&mut self, l: u32) {
        assert!(self.labels[l as usize].is_none(), "label rebound");
        self.labels[l as usize] = Some(self.code.len());
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    pub fn push4(&mut self, insn: u32) {
        self.code.extend_from_slice(&insn.to_le_bytes());
    }

    fn patch32(&mut self, pos: usize, insn: u32) {
        self.code[pos..pos + 4].copy_from_slice(&insn.to_le_bytes());
    }

    // ---- got slots ----

    fn intern_got_slot(&mut self, key: String) -> usize {
        if !self.got_map.contains_key(&key) {
            let idx = self.got_slots.len();
            self.got_map.insert(key.clone(), idx);
            self.got_slots.push(key);
            return idx;
        }
        self.got_map[&key]
    }

    fn emit_adrp_ldr_got(&mut self, xt: u32, key: String) {
        let slot = self.intern_got_slot(key);
        let pos = self.here();
        self.adrp_refs.push(AdrpRef { pos, reg: xt, slot });
        // adrp xt, sym@GOTPAGE  (imm21 patched by linker)
        self.push4(0x90000000 | xt);
        // ldr xt, [xt, sym@GOTPAGEOFF]  (imm12 patched by linker)
        self.push4(0xF9400000 | xt << 5 | xt);
    }

    /// load address of external symbol through the GOT: adrp+ldr
    pub fn ldr_lit_extern(&mut self, xt: u32, sym: &str) {
        self.emit_adrp_ldr_got(xt, format!("E:{sym}"));
    }

    /// load address of a local text label through the GOT
    pub fn ldr_lit_label(&mut self, xt: u32, label: u32) {
        // labels get their text symbol name at finish time via label_syms;
        // here we key by id and resolve the extern name later
        self.emit_adrp_ldr_got(xt, format!("L:{label}"));
    }

    // ---- branches ----

    pub fn b(&mut self, label: u32) {
        let pos = self.here();
        self.fixups.push(Fixup { pos, label, kind: FixKind::B });
        self.push4(0x14000000);
    }

    pub fn bl(&mut self, label: u32) {
        let pos = self.here();
        self.fixups.push(Fixup { pos, label, kind: FixKind::Bl });
        self.push4(0x94000000);
    }

    pub fn b_cond(&mut self, cond: Cond, label: u32) {
        let pos = self.here();
        self.fixups.push(Fixup { pos, label, kind: FixKind::BCond(cond as u32) });
        self.push4(0x54000000);
    }

    pub fn cbz(&mut self, rt: u32, label: u32) {
        let pos = self.here();
        self.fixups.push(Fixup { pos, label, kind: FixKind::Cbz(rt) });
        self.push4(0xB4000000);
    }

    pub fn cbnz(&mut self, rt: u32, label: u32) {
        let pos = self.here();
        self.fixups.push(Fixup { pos, label, kind: FixKind::Cbnz(rt) });
        self.push4(0xB5000000);
    }

    // ---- immediates ----

    pub fn load_const_u64(&mut self, rd: u32, v: u64) {
        match v {
            _ if v <= 0xFFFF => self.movz(rd, v as u16, 0),
            _ => {
                self.movz(rd, (v & 0xFFFF) as u16, 0);
                if (v >> 16) & 0xFFFF != 0 { self.movk(rd, ((v >> 16) & 0xFFFF) as u16, 1); }
                if (v >> 32) & 0xFFFF != 0 { self.movk(rd, ((v >> 32) & 0xFFFF) as u16, 2); }
                if (v >> 48) != 0 { self.movk(rd, ((v >> 48) & 0xFFFF) as u16, 3); }
            }
        }
    }

    pub fn load_const_i64(&mut self, rd: u32, v: i64) {
        self.load_const_u64(rd, v as u64);
    }

    pub fn movz(&mut self, rd: u32, imm16: u16, hw: u32) {
        self.push4(0xD2800000 | hw << 21 | (imm16 as u32) << 5 | rd);
    }

    pub fn movk(&mut self, rd: u32, imm16: u16, hw: u32) {
        self.push4(0xF2800000 | hw << 21 | (imm16 as u32) << 5 | rd);
    }

    // ---- register ops ----

    /// mov rd, rn
    pub fn mov(&mut self, rd: u32, rn: u32) {
        self.push4(0xAA000000 | rn << 16 | XZR << 5 | rd);
    }

    /// add rd, rn, rm {lsl #shift_amt}
    pub fn add_reg(&mut self, rd: u32, rn: u32, rm: u32, shift_amt: u32) {
        // shift type LSL (00) at bits 23..22, imm6 at bits 15..10
        self.push4(0x8B000000 | rm << 16 | (shift_amt & 63) << 10 | rn << 5 | rd);
    }

    pub fn add_imm(&mut self, rd: u32, rn: u32, imm12: u32) {
        assert!(imm12 < 4096);
        self.push4(0x91000000 | imm12 << 10 | rn << 5 | rd);
    }

    pub fn sub_imm(&mut self, rd: u32, rn: u32, imm12: u32) {
        assert!(imm12 < 4096);
        self.push4(0xD1000000 | imm12 << 10 | rn << 5 | rd);
    }

    pub fn cmp_imm(&mut self, rn: u32, imm12: u32) {
        assert!(imm12 < 4096);
        self.push4(0xF1000000 | imm12 << 10 | rn << 5 | XZR);
    }

    pub fn cmp_reg(&mut self, rn: u32, rm: u32) {
        self.push4(0xEB000000 | rm << 16 | rn << 5 | XZR);
    }

    pub fn sub_reg(&mut self, rd: u32, rn: u32, rm: u32) {
        self.push4(0xCB000000 | rm << 16 | rn << 5 | rd);
    }

    pub fn mul(&mut self, rd: u32, rn: u32, rm: u32) {
        self.push4(0x9B007C00 | rm << 16 | rn << 5 | rd);
    }

    pub fn msub(&mut self, rd: u32, rn: u32, rm: u32, ra: u32) {
        self.push4(0x9B008000 | rm << 16 | ra << 10 | rn << 5 | rd);
    }

    pub fn sdiv(&mut self, rd: u32, rn: u32, rm: u32) {
        self.push4(0x9AC00C00 | rm << 16 | rn << 5 | rd);
    }

    /// arithmetic shift right by immediate
    pub fn asr_imm(&mut self, rd: u32, rn: u32, s: u32) {
        let s = s & 63;
        self.push4(0x93400000 | s << 16 | 63 << 10 | rn << 5 | rd);
    }

    pub fn cset(&mut self, rd: u32, cond: Cond) {
        self.push4(0x9A800400 | XZR << 16 | cond.invert() << 12 | XZR << 5 | rd);
    }

    pub fn orr_reg(&mut self, rd: u32, rn: u32, rm: u32) {
        self.push4(0xAA000000 | rm << 16 | rn << 5 | rd);
    }

    // ---- memory ----

    pub fn ldr_off(&mut self, xt: u32, xn: u32, byte_off: i64) {
        assert!(byte_off >= 0 && byte_off % 8 == 0 && byte_off < 32760);
        self.push4(0xF9400000 | ((byte_off as u32 / 8) << 10) | xn << 5 | xt);
    }

    /// str xzr, [xn], #8  (post-index zero store, used by frame zero-init)
    pub fn str_xzr_post8(&mut self, xn: u32) {
        // STR (imm, post-index) 64-bit: 0xF8000000 | imm9<<12 | 01<<10 | Rn<<5 | Rt(xzr=31)
        self.push4(0xF800_0000u32 | (8u32 << 12) | (0b01 << 10) | (xn << 5) | 31);
    }

    pub fn str_off(&mut self, xt: u32, xn: u32, byte_off: i64) {
        assert!(byte_off >= 0 && byte_off % 8 == 0 && byte_off < 32760);
        self.push4(0xF9000000 | ((byte_off as u32 / 8) << 10) | xn << 5 | xt);
    }

    /// str into frame slot k at [fp, #-16-8k]
    /// store into local frame slot k at [sp, #8k]
    /// (frame reserved in prologue; sp stays fixed inside the body)
    pub fn str_slot(&mut self, xt: u32, slot: u32) {
        let off = 8 * slot as i64;
        assert!(off >= 0 && off < 32760, "frame too deep");
        self.push4(0xF9000000 | ((off as u32 / 8) << 10) | RSP << 5 | xt);
    }

    pub fn ldr_slot(&mut self, xt: u32, slot: u32) {
        let off = 8 * slot as i64;
        assert!(off >= 0 && off < 32760, "frame too deep");
        self.push4(0xF9400000 | ((off as u32 / 8) << 10) | RSP << 5 | xt);
    }

    /// sub sp, sp, x9   (x9 = byte count; used for dynamic frame sizes)
    pub fn sub_sp_reg(&mut self) {
        // SUB (extended register) form so Rd=31 means SP: 0xcb2963ff
        self.push4(0xCB2963FF);
    }

    /// add sp, sp, x9
    pub fn add_sp_reg(&mut self) {
        self.push4(0x8B2963FF);
    }

    /// rewrite the movz instruction at byte offset `pos` to carry imm16
    pub fn patch_movz_imm16(&mut self, pos: usize, imm16: u16) {
        let insn = 0xD2800000u32 | (imm16 as u32) << 5 | R9;
        self.patch32(pos, insn);
    }

    pub fn stp_fp_lr(&mut self) {
        // stp x29, x30, [sp, #-16]!
        self.push4(0xA9800000 | ((-2i64 as u32) & 0x7F) << 15 | R30_LR << 10 | RSP << 5 | R29_FP);
    }

    /// ldp x29, x30, [sp], #16 ; ret
    pub fn ldp_fp_lr_ret(&mut self) {
        self.push4(0xA8C00000 | 2 << 15 | R30_LR << 10 | RSP << 5 | R29_FP);
        self.push4(0xD65F03C0);
    }

    pub fn sub_sp_frame(&mut self, bytes: u32) {
        assert!(bytes % 16 == 0 && bytes < 4096);
        self.sub_imm(RSP, RSP, bytes);
    }

    pub fn add_sp_frame(&mut self, bytes: u32) {
        assert!(bytes % 16 == 0 && bytes < 4096);
        self.add_imm(RSP, RSP, bytes);
    }

    pub fn mov_sp_fp(&mut self) {
        // mov x29, sp -- must use ADD (SP not accessible to ORR)
        self.add_imm(R29_FP, RSP, 0);
    }

    pub fn blr(&mut self, xn: u32) {
        self.push4(0xD63F0000 | xn << 5);
    }

    pub fn br(&mut self, xn: u32) {
        self.push4(0xD61F0000 | xn << 5);
    }

    pub fn ret(&mut self) {
        self.push4(0xD65F03C0);
    }

    fn align_to(&mut self, n: usize) {
        while self.code.len() % n != 0 {
            self.code.push(0);
        }
    }

    /// Call after ALL code has been emitted. Resolves ADRP+LDR GOT pairs and
    /// internal branches. GOT slot keys of form "E:name"/"L:id" are returned
    /// so the object writer can emit __got entries with proper relocations.
    pub fn finish(mut self, label_name_of: &dyn Fn(u32) -> String) -> FinishedCode {

        // patch branches
        for f in std::mem::take(&mut self.fixups) {
            let target = self.labels[f.label as usize]
                .expect("unbound label referenced");
            let delta = target as i64 - f.pos as i64;
            assert!(delta % 4 == 0);
            let off26 = ((delta >> 2) as u32) & 0x03FF_FFFF;
            match f.kind {
                FixKind::B => self.patch32(f.pos, 0x14000000 | off26),
                FixKind::Bl => self.patch32(f.pos, 0x94000000 | off26),
                FixKind::BCond(cond) => {
                    assert!(delta.abs() <= (1 << 20));
                    self.patch32(f.pos, 0x54000000 | (((delta >> 2) as u32 & 0x7FFFF) << 5) | cond)
                }
                FixKind::Cbz(rt) => self.patch32(
                    f.pos,
                    0xB4000000 | (((delta >> 2) as u32 & 0x7FFFF) << 5) | rt,
                ),
                FixKind::Cbnz(rt) => self.patch32(
                    f.pos,
                    0xB5000000 | (((delta >> 2) as u32 & 0x7FFFF) << 5) | rt,
                ),
            }
        }
        let label_offsets = self
            .labels
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.map(|o| (i as u32, o)))
            .collect();
        let got: Vec<(String, String)> = self
            .got_slots
            .iter()
            .map(|k| match k.split_once(':').unwrap() {
                ("E", name) => (k.clone(), name.to_string()),
                ("L", id) => {
                    let lid: u32 = id.parse().unwrap();
                    (k.clone(), label_name_of(lid))
                }
                _ => unreachable!(),
            })
            .collect();
        let adrp_refs = std::mem::take(&mut self.adrp_refs)
            .into_iter()
            .map(|a| (a.pos, a.reg, a.slot))
            .collect();
        FinishedCode {
            code: self.code,
            label_offsets,
            got_slots: got,
            adrp_refs,
        }
    }
}

/// Result of assembling the whole program text section.
pub struct FinishedCode {
    pub code: Vec<u8>,
    /// resolved label positions: (label id, byte offset)
    pub label_offsets: Vec<(u32, usize)>,
    /// (internal key, target symbol name) per GOT slot, in order
    pub got_slots: Vec<(String, String)>,
    /// ADRP instruction positions + got slot index, in emission order
    pub adrp_refs: Vec<(usize /*pos*/, u32 /*reg*/, usize /*slot*/)>,
}
