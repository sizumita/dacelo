// macho.rs -- minimal Mach-O 64-bit object file writer (arm64)

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
pub enum RelocKind {
    /// 8-byte absolute pointer (data sections)
    Unsigned,
    /// adrp Xn, sym@GOTPAGE
    GotLoadPage21,
    /// ldr Xt, [Xn, sym@GOTPAGEOFF]
    GotLoadPageOff12,
}

pub struct Reloc {
    /// byte offset of the relocated field within its section
    pub offset: u64,
    pub sym_name: String,
    pub kind: RelocKind,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Section {
    Text = 1,
    Data = 2,
}

pub struct Symbol {
    pub name: String,
    pub section: Option<Section>, // None => undefined extern
    pub value: u64,
    /// exported to other objects (N_EXT)
    pub global: bool,
}

/// Builder collects everything needed to emit a relocatable object.
///
/// All relocations are ARM64_RELOC_UNSIGNED on 8-byte fields
/// (literal-pool quads / data pointers).
pub struct ObjectBuilder {
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    pub syms: Vec<Symbol>,
    pub relocs_text: Vec<Reloc>,
    pub relocs_data: Vec<Reloc>,
}

fn put(buf: &mut [u8], pos: usize, bytes: &[u8]) {
    buf[pos..pos + bytes.len()].copy_from_slice(bytes);
}

impl ObjectBuilder {
    pub fn new() -> ObjectBuilder {
        ObjectBuilder {
            text: Vec::new(),
            data: Vec::new(),
            syms: Vec::new(),
            relocs_text: Vec::new(),
            relocs_data: Vec::new(),
        }
    }

    pub fn define(&mut self, name: &str, section: Section, offset: u64, global: bool) {
        self.syms.push(Symbol {
            name: name.to_string(),
            section: Some(section),
            value: offset,
            global,
        });
    }

    pub fn add_extern(&mut self, name: &str) {
        self.syms.push(Symbol {
            name: name.to_string(),
            section: None,
            value: 0,
            global: false,
        });
    }

}

/// Emit a section_64 header (80 bytes).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn emit_section_header_full(
    out: &mut Vec<u8>,
    sectname: &[u8],
    segname: &[u8],
    addr: u64,
    size: u64,
    fileoff: u32,
    align_log2: u32,
    reloff: u32,
    nreloc: u32,
) {
    let mut s = [0u8; 80];
    let mut sn = [0u8; 16];
    sn[..sectname.len()].copy_from_slice(sectname);
    put(&mut s, 0, &sn);
    let mut sg = [0u8; 16];
    sg[..segname.len()].copy_from_slice(segname);
    put(&mut s, 16, &sg);
    put(&mut s, 32, &addr.to_le_bytes());
    put(&mut s, 40, &size.to_le_bytes());
    put(&mut s, 48, &fileoff.to_le_bytes());
    put(&mut s, 52, &align_log2.to_le_bytes());
    put(&mut s, 56, &reloff.to_le_bytes());
    put(&mut s, 60, &nreloc.to_le_bytes());
    // flags @64 = S_REGULAR (0); reserved1..3 stay zero
    out.extend_from_slice(&s);
}

fn emit_segment_header(
    out: &mut Vec<u8>,
    segname: &[u8],
    vmsize: u64,
    fileoff: u64,
    nsects: u32,
) {
    const LC_SEGMENT_64: u32 = 0x19;
    let mut sg = [0u8; 16];
    sg[..segname.len()].copy_from_slice(segname);
    out.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
    out.extend_from_slice(&(72u32 + 80 * nsects).to_le_bytes()); // cmdsize
    out.extend_from_slice(&sg);
    out.extend_from_slice(&0u64.to_le_bytes()); // vmaddr
    out.extend_from_slice(&vmsize.to_le_bytes());
    out.extend_from_slice(&fileoff.to_le_bytes());
    out.extend_from_slice(&vmsize.to_le_bytes()); // filesize == vmsize
    out.extend_from_slice(&7u32.to_le_bytes());   // maxprot
    out.extend_from_slice(&7u32.to_le_bytes());   // initprot
    out.extend_from_slice(&nsects.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());   // flags
}

impl ObjectBuilder {
    /// Serialize the object file.
    pub fn finish(self) -> Result<Vec<u8>, String> {
        const MH_MAGIC_64: u32 = 0xFEEDFACF;
        const CPU_TYPE_ARM64: u32 = 0x0100_000C;
        const MH_OBJECT: u32 = 1;
        const LC_SYMTAB: u32 = 0x02;
        const LC_BUILD_VERSION: u32 = 0x32;
        const PLATFORM_MACOS: u32 = 1;

        // ---- symbol table ----
        // ld64 expects locals first (by section), then globals, then undefineds
        let mut strtab: Vec<u8> = vec![0];
        let mut sym_index: HashMap<String, usize> = HashMap::new();
        struct NList {
            stroff: u32,
            ty: u8,
            sect: u8,
            desc: u16,
            value: u64,
        }
        let mut nlists: Vec<NList> = Vec::new();

        let mut push_str = |strtab: &mut Vec<u8>, s: &str| -> u32 {
            let off = strtab.len() as u32;
            strtab.extend_from_slice(s.as_bytes());
            strtab.push(0);
            off
        };

        let align_up = |v: usize, a: usize| (v + a - 1) / a * a;
        let data_addr = align_up(self.text.len(), 8) as u64;

        let locals: Vec<&Symbol> = self
            .syms
            .iter()
            .filter(|s| s.section.is_some() && !s.global)
            .collect();
        let globals: Vec<&Symbol> = self
            .syms
            .iter()
            .filter(|s| s.section.is_some() && s.global)
            .collect();
        let undefs: Vec<&Symbol> = self.syms.iter().filter(|s| s.section.is_none()).collect();

        for s in locals.iter().chain(globals.iter()) {
            sym_index.insert(s.name.clone(), nlists.len());
            let (sect_no, ty, base) = match s.section.unwrap() {
                Section::Text => (1u8, if s.global { 0x0Fu8 } else { 0x0Eu8 }, 0u64),
                Section::Data => (2u8, if s.global { 0x0Fu8 } else { 0x0Eu8 }, data_addr),
            };
            nlists.push(NList {
                stroff: push_str(&mut strtab, &s.name),
                ty,
                sect: sect_no,
                desc: 0,
                value: base + s.value,
            });
        }
        for s in &undefs {
            sym_index.insert(s.name.clone(), nlists.len());
            nlists.push(NList {
                stroff: push_str(&mut strtab, &s.name),
                ty: 0x01, // N_EXT | N_UNDF
                sect: 0,
                desc: 0,
                value: 0,
            });
        }

        let encode_relocs = |relocs: &[Reloc]| -> Result<Vec<u8>, String> {
            const ARM64_RELOC_UNSIGNED: u32 = 0;
            const ARM64_RELOC_GOT_LOAD_PAGE21: u32 = 5;
            const ARM64_RELOC_GOT_LOAD_PAGEOFF12: u32 = 6;
            let mut buf = Vec::new();
            for r in relocs {
                let symidx = *sym_index.get(&r.sym_name).ok_or_else(|| {
                    format!("relocation against unknown symbol `{}`", r.sym_name)
                })? as u32;
                let (pcrel, length, rtype) = match r.kind {
                    RelocKind::Unsigned => (0u32, 3u32, ARM64_RELOC_UNSIGNED),
                    RelocKind::GotLoadPage21 => (1, 2, ARM64_RELOC_GOT_LOAD_PAGE21),
                    RelocKind::GotLoadPageOff12 => (0, 2, ARM64_RELOC_GOT_LOAD_PAGEOFF12),
                };
                buf.extend_from_slice(&(r.offset as u32).to_le_bytes());
                let word: u32 = symidx              // r_symbolnum:24
                    | pcrel << 24
                    | length << 25
                    | 1 << 27                        // r_extern=1
                    | rtype << 28;
                buf.extend_from_slice(&word.to_le_bytes());
            }
            Ok(buf)
        };
        let text_rel_bytes = encode_relocs(&self.relocs_text)?;
        let data_rel_bytes = encode_relocs(&self.relocs_data)?;

        // ---- file layout ----
        const HEADER_SIZE: usize = 32;
        const BUILD_VER_SIZE: usize = 24;
        const SYMTAB_CMD_SIZE: usize = 24;

        let ncmds = 3u32; // segment + build_version + symtab
        let sizeof_cmds = 72 + 80 * 2 + BUILD_VER_SIZE + SYMTAB_CMD_SIZE;

        let align_up = |v: usize, a: usize| (v + a - 1) / a * a;
        let text_fileoff = HEADER_SIZE + sizeof_cmds;
        let content_start = text_fileoff as u64;
        let data_fileoff = align_up(text_fileoff + self.text.len(), 16);
        let rel_text_off = data_fileoff + self.data.len();
        let rel_data_off = rel_text_off + text_rel_bytes.len();
        let sym_off = align_up(rel_data_off + data_rel_bytes.len(), 8);
        let str_off = sym_off + nlists.len() * 16;

        let mut out = Vec::with_capacity(str_off + strtab.len());

        // header
        out.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        out.extend_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        out.extend_from_slice(&MH_OBJECT.to_le_bytes());
        out.extend_from_slice(&ncmds.to_le_bytes());
        out.extend_from_slice(&(sizeof_cmds as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // flags
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved

        // single unnamed segment containing both sections (matches clang -c output)
        let text_pad = align_up(self.text.len(), 8);
        let total_content = text_pad + self.data.len();
        emit_segment_header(&mut out, b"", total_content as u64, content_start as u64, 2);

        emit_section_header_full(
            &mut out,
            b"__text",
            b"__TEXT",
            /*addr*/ 0,
            self.text.len() as u64,
            text_fileoff as u32,
            2, // 4-byte alignment
            if text_rel_bytes.is_empty() { 0 } else { rel_text_off as u32 },
            self.relocs_text.len() as u32,
        );
        emit_section_header_full(
            &mut out,
            b"__data",
            b"__DATA",
            text_pad as u64, // addr within segment
            self.data.len() as u64,
            data_fileoff as u32,
            3, // 8-byte alignment
            if data_rel_bytes.is_empty() { 0 } else { rel_data_off as u32 },
            self.relocs_data.len() as u32,
        );

        // LC_BUILD_VERSION
        out.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        out.extend_from_slice(&(BUILD_VER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&PLATFORM_MACOS.to_le_bytes());
        out.extend_from_slice(&0x000C_0000u32.to_le_bytes()); // min macOS 12.0.0
        out.extend_from_slice(&0x000C_0000u32.to_le_bytes()); // sdk
        out.extend_from_slice(&0u32.to_le_bytes()); // ntools

        // LC_SYMTAB
        out.extend_from_slice(&LC_SYMTAB.to_le_bytes());
        out.extend_from_slice(&(SYMTAB_CMD_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&(sym_off as u32).to_le_bytes());
        out.extend_from_slice(&(nlists.len() as u32).to_le_bytes());
        out.extend_from_slice(&(str_off as u32).to_le_bytes());
        out.extend_from_slice(&(strtab.len() as u32).to_le_bytes());

        assert_eq!(out.len(), text_fileoff);

        // contents
        out.extend_from_slice(&self.text);
        while out.len() < data_fileoff {
            out.push(0);
        }
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&text_rel_bytes);
        out.extend_from_slice(&data_rel_bytes);
        while out.len() < sym_off {
            out.push(0);
        }
        for e in &nlists {
            out.extend_from_slice(&e.stroff.to_le_bytes());
            out.push(e.ty);
            out.push(e.sect);
            out.extend_from_slice(&e.desc.to_le_bytes());
            out.extend_from_slice(&e.value.to_le_bytes());
        }
        out.extend_from_slice(&strtab);

        Ok(out)
    }
}
