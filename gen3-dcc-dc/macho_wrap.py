#!/usr/bin/env python3
"""Mach-O object wrapper for dacelo Gen 3 self-hosting.
Reads raw section binaries + metadata from JSON manifest,
emits a valid Mach-O arm64 object file."""
import struct, json, sys

def main():
    manifest = json.load(open(sys.argv[1]))
    out_path = sys.argv[2]
    
    text = bytes(manifest["text"])
    data = bytes(manifest["data"])
    # symbols: list of {name, sect(0=undef,1=text,2=data), value, global}
    syms = manifest.get("syms", [])
    # relocs: list of {sect(1=text,2=data), offset, symidx_or_name, kind, pcrel, length, extern}
    trelocs = manifest.get("trelocs", [])
    drelocs = manifest.get("drelocs", [])

    # build strtab & symbol table
    strtab = b"\x00"
    nlists = []
    sym_index = {}
    defined = [s for s in syms if s["sect"] != 0]
    undefs = [s for s in syms if s["sect"] == 0]

    def add_str(s):
        nonlocal strtab
        off = len(strtab)
        strtab += s.encode() + b"\x00"
        return off

    for s in defined:
        sym_index[s["name"]] = len(nlists)
        ty = 0x0F if s.get("global") else 0x0E
        nlists.append((add_str(s["name"]), ty, s["sect"], 0, s["value"]))
    for s in undefs:
        sym_index[s["name"]] = len(nlists)
        nlists.append((add_str(s["name"]), 0x01, 0, 0, 0))

    def resolve(name):
        if name.startswith("L:"):
            name = name[2:]
        elif name.startswith("E:"):
            name = name[2:]
        idx = sym_index.get(name)
        if idx is None:
            print(f"WARNING: unresolved symbol {name}", file=sys.stderr)
            return None
        return idx

    def encode_reloc(offset, symname, typ, pcrel, length=2):
        idx = resolve(symname)
        if idx is None: idx = 0
        extern = 1
        word = idx | (pcrel << 24) | (length << 25) | (extern << 27) | (typ << 28)
        return struct.pack("<Ii", offset, word)

    treloc_bytes = b"".join(
        encode_reloc(r["offset"], r["sym"], {"gotpage":5,"gotoff":6}.get(r["kind"],0),
                     1 if r["kind"]=="gotpage" else 0)
        for r in trelocs)
    dreloc_bytes = b"".join(
        encode_reloc(r["offset"], r["sym"], 0, 0, 3)
        for r in drelocs)

    # layout
    HEADER_SIZE = 32
    SEG_CMD = 72 + 80 * 2  # one segment with two sections
    BUILD_VER = 24
    SYMTAB_CMD = 24
    sizeof_cmds = SEG_CMD + BUILD_VER + SYMTAB_CMD
    text_off = HEADER_SIZE + sizeof_cmds
    align_up = lambda v,a: (v+a-1)//a*a
    data_off = align_up(text_off + len(text), 16)
    trel_off = data_off + len(data)
    drel_off = trel_off + len(treloc_bytes)
    sym_off = align_up(drel_off + len(dreloc_bytes), 8)
    str_off = sym_off + len(nlists)*16

    out = bytearray()
    out += struct.pack("<IiiIIII", 0xFEEDFACF, 0x0100000C, 0, 1, 3,
                       SEG_CMD+BUILD_VER+SYMTAB_CMD, 0)
    out += struct.pack("<I", 0)  # reserved

    # LC_SEGMENT_64 (single unnamed segment, both sections)
    segname = b"\x00"*16
    out += struct.pack("<II", 0x19, SEG_CMD)
    out += segname
    out += struct.pack("<QQQQ", 0, total_content, text_off, total_content)
    out += struct.pack("<III", 7, 7, 2)
    out += struct.pack("<I", 0)

    for sectname, segname2, addr, size, off, algn, roff, nrel in [
        (b"__text", b"__TEXT", 0, len(text), text_off, 2,
         trel_off if treloc_bytes else 0, len(trelocs)),
        (b"__data", b"__DATA", align_up(len(text),8), len(data), data_off, 3,
         drel_off if dreloc_bytes else 0, len(drelocs)),
    ]:
        sn = sectname.ljust(16, b"\x00")
        sg = segname2.ljust(16, b"\x00")
        out += sn + sg
        out += struct.pack("<QQIIIIIIII", addr, size, off, algn, roff, nrel, 0, 0, 0, 0)

    # LC_BUILD_VERSION
    out += struct.pack("<IIIII", 0x32, 24, 1, 0x000C0000, 0x000C0000)
    # (platform=macos, minos 12.0, sdk 12.0, ntools=0) -- but ntools missing; pad
    out += struct.pack("<I", 0)

    # LC_SYMTAB
    out += struct.pack("<IIIIII", 0x02, 24, sym_off, len(nlists), str_off, len(strtab))

    assert len(out) == text_off, f"layout mismatch {len(out)} vs {text_off}"

    out += text
    while len(out) < data_off: out.append(0)
    out += data
    out += treloc_bytes + dreloc_bytes
    while len(out) < sym_off: out.append(0)
    for stroff, ty, sect, desc, val in nlists:
        out += struct.pack("<IBBH", stroff, ty, sect, desc)
        out += struct.pack("<Q", val)
    out += strtab

    open(out_path, "wb").write(bytes(out))
    print(f"macho_wrap: wrote {out_path} ({len(out)} bytes)")

if __name__ == "__main__":
    main()
