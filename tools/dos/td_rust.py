"""Rust code from Borland Turbo Debugger symbols: record types, the data segment's variables,
and the Pascal reader/writer they use. Used by tdinfo.py (commands rust, rust-globals,
rust-reader); see its help.

  rust_for(td, type_names)   one Rust module: structs with read/write for record types
  rust_globals(td, out_dir)  a module directory: one file per Pascal unit with that unit's
                             variables at their data-segment offsets, types/ (one file per
                             record type), mod.rs joining them into Globals with load/store
  rust_locals(td, proc_name) one Rust module: a struct of a procedure's local variables (and
                             parameters) at their BP offsets, with load/store from an image of the
                             stack segment and the procedure's BP
  reader()                   templates/pascal.rs: PascalReader, PascalWriter, PStr, Real48
"""
import os
import re
import struct

from tdinfo import element_record

HERE = os.path.dirname(os.path.abspath(__file__))


def reader():
    with open(os.path.join(HERE, 'templates', 'pascal.rs')) as f:
        return f.read()


RUST_KEYWORDS = {'as', 'break', 'const', 'continue', 'crate', 'else', 'enum', 'extern', 'false', 'fn',
                 'for', 'if', 'impl', 'in', 'let', 'loop', 'match', 'mod', 'move', 'mut', 'pub', 'ref',
                 'return', 'self', 'static', 'struct', 'super', 'trait', 'true', 'type', 'unsafe', 'use',
                 'where', 'while', 'async', 'await', 'dyn', 'box', 'yield', 'macro', 'try', 'final'}
RUST_SCALARS = {0x04: ('i8', 'i8'), 0x05: ('i16', 'i16'), 0x06: ('i32', 'i32'), 0x08: ('u8', 'u8'),
                0x09: ('u16', 'u16'), 0x0A: ('u32', 'u32'), 0x0C: ('u8', 'u8'), 0x28: ('bool', 'boolean'),
                0x0E: ('Real48', 'real48'), 0x0D: ('f32', 'f32'), 0x0F: ('f64', 'f64')}



def rust_ident(name):
    n = name.lower()
    return n + '_' if n in RUST_KEYWORDS else n


RUST_PRELUDE = {'Option', 'Result', 'Box', 'String', 'Vec', 'Some', 'None', 'Ok', 'Err', 'Self',
                'Copy', 'Clone', 'Default', 'Drop', 'Fn', 'Iterator', 'Send', 'Sized', 'Sync', 'Debug'}


def rust_struct_name(name):
    base = name[:-4] if name.lower().endswith('type') and len(name) > 4 else name
    if '_' in base or base.isupper():   # SMALL_PACKET -> SmallPacket
        base = ''.join(p[:1].upper() + p[1:].lower() for p in base.split('_') if p)
    base = base[:1].upper() + base[1:]
    return base + 'Rec' if base in RUST_PRELUDE else base   # Optiontype -> OptionRec


def rust_type(td, t, out_structs, hint, exact_bool=False):
    """(rust type, read expression, write function) for type t. The write function takes the
    expression of a value and returns a statement that writes it with `w`. Nested anonymous
    records become structs. exact_bool keeps a Boolean as u8 (any byte value survives)."""
    ty = td.type(t)
    tid, name, size, b, w = ty
    if tid == 0x28 and exact_bool:
        return 'u8', 'r.u8()?', lambda v: f'w.u8({v});'
    if tid in RUST_SCALARS:
        rt, method = RUST_SCALARS[tid]
        return rt, f'r.{method}()?', lambda v, m=method: f'w.{m}({v});'
    if tid == 0x03:
        return f'PStr<{size - 1}>', 'r.pstr()?', lambda v: f'w.pstr(&({v}));'
    if tid == 0x1C:
        count = size // max(td.type_size(w), 1)
        et, er, ew = rust_type(td, w, out_structs, hint, exact_bool)
        if et == 'u8':
            return f'[u8; {count}]', 'r.array()?', lambda v: f'w.bytes(&({v}));'
        return (f'[{et}; {count}]', f'r.arr(|r| Ok({er}))?',
                lambda v, ew=ew: f'for x in ({v}).iter() {{ {ew("*x")} }}')
    if tid == 0x1E:
        sname = rust_struct_name(td.name(name)) if name else hint
        if sname not in out_structs:
            out_structs[sname] = None   # reserve, then fill (recursion)
            out_structs[sname] = rust_record(td, t, sname, out_structs, exact_bool)
        return sname, f'{sname}::read(r)?', lambda v: f'({v}).write(w);'
    if tid in (0x15, 0x16) and size == 4:
        return 'u32', 'r.u32()?', lambda v: f'w.u32({v});'      # far pointer: offset, segment
    return f'[u8; {size}]', 'r.array()?', lambda v: f'w.bytes(&({v}));'   # unknown kinds stay raw


def rust_record(td, t, sname, out_structs, exact_bool=False):
    fields, reads, writes = [], [], []
    for off, fname, ftype, fsize in td.record_fields(t):
        rt, rexpr, wfn = rust_type(td, _member_type(td, t, fname), out_structs, sname + fname[:1].upper() + fname[1:], exact_bool)
        ident = rust_ident(fname)
        fields.append(f'    /// +{off:#06x} {fname}: {ftype}\n    pub {ident}: {rt},')
        reads.append(f'            {ident}: {rexpr},')
        writes.append(f'        {wfn("self." + ident)}')
    size = td.type_size(t)
    return (f'#[derive(Debug, Clone, Copy, PartialEq)]\npub struct {sname} {{\n' + '\n'.join(fields) + '\n}\n\n'
            f'impl {sname} {{\n    /// Bytes in the original\'s memory and files.\n    pub const SIZE: usize = {size};\n\n'
            f'    pub fn read(r: &mut PascalReader) -> Result<Self, ShortRead> {{\n'
            f'        let start = r.pos();\n        let v = {sname} {{\n' + '\n'.join(reads) + '\n        };\n'
            f'        debug_assert_eq!(r.pos() - start, Self::SIZE);\n        Ok(v)\n    }}\n\n'
            f'    pub fn write(&self, w: &mut PascalWriter) {{\n' + '\n'.join(writes) + '\n    }\n}\n')


def _member_type(td, t, fname):
    first = td.type(t)[4]
    i = first - 1
    while 0 <= i < len(td.members):
        flags, name, mt = td.members[i]
        if td.name(name) == fname and not flags & 0x40:
            return mt
        if flags & 0x80:
            break
        i += 1
    raise KeyError(fname)


def rust_for(td, type_names):
    named = {n['name'].lower(): n for n in td.named_types()}
    out_structs, tables = {}, []
    for tn in type_names:
        nt = named.get(tn.lower())
        if nt is None:
            raise SystemExit(f'{tn}: no such named type')
        t = nt['type']
        ty = td.type(t)
        if ty[0] == 0x1C:     # array of records: a table type plus its element struct
            elem = element_record(td, t)
            ename = rust_struct_name(nt['name'])
            rust_type(td, elem, out_structs, ename) if td.type(elem)[1] else out_structs.setdefault(ename, rust_record(td, elem, ename, out_structs))
            index, = struct.unpack_from('<H', td.type_slot(t + 1))
            lo, hi = struct.unpack('<ii', td.type_slot(index + 1))
            ename = rust_struct_name(td.name(td.type(elem)[1])) if td.type(elem)[1] else ename
            tables.append(f'/// Pascal `{nt["name"]} = array[{lo}..{hi}] of {ename}` ({nt["size"]} bytes).\n'
                          f'pub const {nt["name"].upper()}_LOW: i32 = {lo};\npub const {nt["name"].upper()}_COUNT: usize = {hi - lo + 1};\n')
        elif ty[0] == 0x1E:
            rust_type(td, t, out_structs, rust_struct_name(nt['name']))
        else:
            raise SystemExit(f'{tn}: not a record or an array of records')
    head = ('// Generated by tools/dos/tdinfo.py rust (GameFrame) from the original program\'s\n'
            '// debug symbols. Field order, types and offsets are the original\'s; do not edit by hand.\n'
            '#![allow(clippy::all)]\n#[allow(unused_imports)]\nuse crate::pascal::{PascalReader, PascalWriter, PStr, Real48, ShortRead};\n\n')
    return head + '\n'.join(tables) + '\n' + '\n'.join(v for v in out_structs.values() if v)


GENERATED = ('// Generated by tools/dos/tdinfo.py rust-globals (GameFrame) from the original\n'
             '// program\'s debug symbols. Do not edit by hand: regenerate.\n')
IMPORTS = ('#[allow(unused_imports)]\nuse crate::pascal::{PascalReader, PascalWriter, PStr, Real48, ShortRead};\n')


def unit_module(unit):
    m = (unit or 'other').lower()
    return m + '_' if m in RUST_KEYWORDS else m


def snake(name):
    """GREXDat -> grex_dat, SingleEnemy -> single_enemy (a run of capitals is one word)."""
    s = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', name)
    return re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', s).lower()


def write_types(out_dir, out_structs):
    """types/: one file per record type, and mod.rs re-exporting them all."""
    tdir = os.path.join(out_dir, 'types')
    os.makedirs(tdir, exist_ok=True)
    old = os.path.join(out_dir, 'types.rs')
    if os.path.exists(old):
        os.remove(old)
    names = []
    for sname, code in out_structs.items():
        if not code:
            continue
        mod = snake(sname)
        names.append((mod, sname))
        with open(os.path.join(tdir, f'{mod}.rs'), 'w') as f:
            f.write(f'//! Record type `{sname}`.\n' + GENERATED + '#![allow(clippy::all)]\n' + IMPORTS +
                    '#[allow(unused_imports)]\nuse super::*;\n\n' + code)
    with open(os.path.join(tdir, 'mod.rs'), 'w') as f:
        f.write("//! Record types of the data segment's variables, one file each.\n" + GENERATED +
                ''.join(f'mod {m};\n' for m, _ in sorted(names)) + '\n' +
                ''.join(f'pub use {m}::{n};\n' for m, n in sorted(names)))


def rust_globals(td, out_dir):
    """Write the data segment's variables as a module directory under out_dir: one file per
    Pascal unit (struct with load/store at the original offsets; Booleans kept as u8 so every
    byte survives), types/ (one file per record type), mod.rs (Globals: one field per unit)."""
    auto = td.exe.autodata if td.exe.kind == 'NE' else None
    rows = []
    for i, (name, t, off, seg, bits) in enumerate(td.symbols, 1):
        ty = td.type(t)
        if bits & 7 != 0 or not ty or ty[0] in (0x23, 0x24) or (auto and seg != auto):
            continue
        rows.append((off, td.type_size(t), td.name(name), t, unit_module(td.module_of_symbol(i))))
    rows.sort()
    out_structs, units, end = {}, {}, 0
    for off, size, name, t, unit in rows:
        alias = off < end           # overlaps the variable before it: loaded, not stored
        end = max(end, off + size)
        units.setdefault(unit, []).append((off, size, name, t, alias))
    os.makedirs(out_dir, exist_ok=True)
    for unit, vars_ in units.items():
        fields, loads, stores, seen = [], [], [], set()
        for off, size, name, t, alias in vars_:
            ident = rust_ident(name)
            while ident in seen:
                ident += f'_{off:04x}'
            seen.add(ident)
            rt, rexpr, wfn = rust_type(td, t, out_structs, rust_struct_name('G' + name), exact_bool=True)
            note = ' (overlaps the variable before it: loaded, not stored)' if alias else ''
            fields.append(f'    /// {off:#06x} {name}: {td.type_name(t, 1)} ({size} bytes){note}\n    pub {ident}: {rt},')
            loads.append(f'            {ident}: {{ r.seek({off:#x}); {rexpr} }},')
            if not alias:
                stores.append(f'        w.seek({off:#x}); {wfn("self." + ident)}')
        sname = rust_struct_name(unit.rstrip('_'))
        body = (f'//! Unit `{unit.rstrip("_")}`: its variables in the data segment.\n' + GENERATED +
                '#![allow(clippy::all)]\n' + IMPORTS + '#[allow(unused_imports)]\nuse super::types::*;\n\n'
                f'#[derive(Debug, Clone, PartialEq)]\npub struct {sname} {{\n' + '\n'.join(fields) + '\n}\n\n'
                f'impl {sname} {{\n    pub fn load(r: &mut PascalReader) -> Result<Self, ShortRead> {{\n'
                f'        Ok({sname} {{\n' + '\n'.join(loads) + '\n        })\n    }\n\n'
                '    pub fn store(&self, w: &mut PascalWriter) {\n' + '\n'.join(stores) + '\n    }\n}\n')
        with open(os.path.join(out_dir, f'{unit}.rs'), 'w') as f:
            f.write(body)
    write_types(out_dir, out_structs)
    mods = sorted(units)
    with open(os.path.join(out_dir, 'mod.rs'), 'w') as f:
        f.write('//! Every variable of the original\'s data segment, by Pascal unit, at its original\n'
                '//! offset. `Globals::load` reads them from a 64 KB image of the segment and `store`\n'
                '//! writes them back (bytes between variables are left alone), so ported code can be\n'
                '//! checked against the original\'s memory after an emulated run.\n' + GENERATED +
                IMPORTS + '\npub mod types;\n' + ''.join(f'pub mod {m};\n' for m in mods) +
                '\n#[derive(Debug, Clone, PartialEq)]\npub struct Globals {\n' +
                ''.join(f'    pub {m}: {m}::{rust_struct_name(m.rstrip("_"))},\n' for m in mods) + '}\n\n'
                'impl Globals {\n    pub fn load(ds: &[u8]) -> Result<Box<Globals>, ShortRead> {\n'
                '        let r = &mut PascalReader::new(ds);\n        Ok(Box::new(Globals {\n' +
                ''.join(f'            {m}: {m}::{rust_struct_name(m.rstrip("_"))}::load(r)?,\n' for m in mods) +
                '        }))\n    }\n\n    pub fn store(&self, ds: &mut [u8]) {\n        let w = &mut PascalWriter::new(ds);\n' +
                ''.join(f'        self.{m}.store(w);\n' for m in mods) + '    }\n}\n')
    return sorted(units)


def rust_locals(td, proc_name):
    """A struct of a procedure's locals and parameters at their BP offsets (Booleans as u8),
    with load(ss, bp) and store(ss, bp) on an image of the stack segment. Offsets wrap at 64 KB,
    as BP-relative addresses do."""
    procs = [p for p in td.procedures() if p['name'].lower() == proc_name.lower()]
    if not procs:
        raise SystemExit(f'{proc_name}: no such procedure')
    proc = procs[0]
    first = next(i for i, (n, t, o, seg, b) in enumerate(td.symbols, 1) if i == proc['index'])
    scope = next(((f, c) for f, c, _, _, off, length in td.scopes if off == proc['offset'] and length), None)
    if scope is None:
        raise SystemExit(f'{proc_name}: no scope (no locals) in the symbols')
    out_structs, fields, loads, stores, seen = {}, [], [], [], set()
    sname = rust_struct_name(proc['name']) + 'Locals'
    rows = []
    for i in range(scope[0], scope[0] + scope[1]):
        name, t, o, seg, bits = td.symbols[i - 1]
        if bits & 7 not in (2, 3) or not td.type(t):
            continue
        rel = o - 0x10000 if o >= 0x8000 else o
        rows.append((rel, td.name(name), t, bits & 7 == 3))
    for rel, name, t, by_ref in sorted(rows):
        ident = rust_ident(name)
        while ident in seen:
            ident += '_'
        seen.add(ident)
        if by_ref:                      # a var parameter: a far pointer on the stack
            rt, rexpr, wfn = 'u32', 'r.u32()?', (lambda v: f'w.u32({v});')
            what = f'var {td.type_name(t, 1)} (a far pointer)'
        else:
            rt, rexpr, wfn = rust_type(td, t, out_structs, rust_struct_name('L' + name), exact_bool=True)
            what = td.type_name(t, 1)
        fields.append(f'    /// [bp{rel:+d}] {name}: {what}\n    pub {ident}: {rt},')
        loads.append(f'            {ident}: {{ r.seek(bp.wrapping_add({rel & 0xFFFF:#x}) as usize); {rexpr} }},')
        stores.append(f'        w.seek(bp.wrapping_add({rel & 0xFFFF:#x}) as usize); {wfn("self." + ident)}')
    head = (f'//! Local variables of `{proc["module"]}.{proc["name"]}` ({proc["address"]}), on its stack frame.\n'
            + GENERATED.replace('rust-globals', f'rust-locals {proc["name"]}') + '#![allow(clippy::all)]\n' + IMPORTS + '\n')
    body = (f'#[derive(Debug, Clone, PartialEq)]\npub struct {sname} {{\n' + '\n'.join(fields) + '\n}\n\n'
            f'impl {sname} {{\n    /// Read them from an image of the stack segment, with the procedure\'s BP.\n'
            f'    pub fn load(ss: &[u8], bp: u16) -> Result<Box<Self>, ShortRead> {{\n'
            f'        let r = &mut PascalReader::new(ss);\n        Ok(Box::new({sname} {{\n' + '\n'.join(loads) + '\n        }))\n    }\n\n'
            f'    pub fn store(&self, ss: &mut [u8], bp: u16) {{\n        let w = &mut PascalWriter::new(ss);\n' + '\n'.join(stores) + '\n    }\n}\n')
    return head + body + '\n' + '\n'.join(v for v in out_structs.values() if v)
