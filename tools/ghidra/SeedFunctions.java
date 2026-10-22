// Seed the functions Ghidra's auto-analysis cannot reach in FRONTIER.EXE and the EL2*.OVL overlays,
// then let analysis follow the direct calls from them. See docs/OVERLAYS.md for the structures.
//
// FRONTIER.EXE: the far-pointer vector table at DS:0000-01cf (every `lcall [disp16]` goes through it),
//   the lazy module thunks, near jump tables, and call overrides so `lcall [vec]` decompiles as a
//   direct call. Overlay-filled slots (DS:01d0-0357) and module calls get comments naming the overlay
//   routine, read from the overlay files next to the imported EXE.
// EL2*.OVL: the TSR stub at 0000, the export block at 005e defined as data (Ghidra otherwise runs the
//   stub on into it), and every exported far pointer seeded as a function.
// Coverage (docs/COVERAGE.md), when game-data/derived/coverage/seeds/<NAME>.cov and .entries exist
//   (tools/coverage/report.py): every instruction the original executed on the runtime is
//   disassembled, and every address it entered by a call, far jump or interrupt becomes a function.
//
// Usage: -postScript SeedFunctions.java   (before ExportAll.java)
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.data.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public class SeedFunctions extends GhidraScript {
  // Main data segment, as stored in the file (mov ax,1615h / mov ds,ax at 1000:071e).
  static final int MAIN_DS_RAW = 0x1615;
  static final int LOAD_SEG = 0x1000; // Ghidra's load segment for every image
  static final int MAIN_VEC_END = 0x1d0; // DS:0000-01cf: main-program routines
  static final int MODULE_TABLE = 0x396, MODULE_STUB = 0x35c;

  // Overlay export blocks: name tag, first DS slot it fills (from the block's dest word).
  static final String[] DRIVERS = { "EL2MCGA3.OVL", "EL2MATH3.OVL", "EL2ADLIB.OVL", "EL2ROL.OVL",
    "EL2SPK.OVL", "EL2SETUP.OVL" };

  SegmentedAddressSpace sp;
  Listing listing;
  FunctionManager fm;
  int seeded = 0;

  Address at(int seg, int off) { return sp.getAddress(seg & 0xffff, off & 0xffff); }
  int u16(Address a) throws Exception { return getShort(a) & 0xffff; }

  /** Raw MZ image (header stripped) of a sibling file of the imported program. */
  static class Mz {
    byte[] img;
    Mz(File f) throws IOException {
      byte[] d = Files.readAllBytes(f.toPath());
      int hdr = ((d[8] & 0xff) | (d[9] & 0xff) << 8) * 16;
      img = Arrays.copyOfRange(d, hdr, d.length);
    }
    int u16(int o) { return (img[o] & 0xff) | (img[o + 1] & 0xff) << 8; }
  }

  @Override
  public void run() throws Exception {
    sp = (SegmentedAddressSpace) currentProgram.getAddressFactory().getDefaultAddressSpace();
    listing = currentProgram.getListing();
    fm = currentProgram.getFunctionManager();
    int before = fm.getFunctionCount();
    File dir = new File(currentProgram.getExecutablePath()).getParentFile();
    String name = currentProgram.getName();
    if (name.equalsIgnoreCase("FRONTIER.EXE")) seedMain(dir);
    else seedOverlay(dir, name);
    seedCoverage(dir, name);
    analyzeChanges(currentProgram);
    for (int pass = 0; pass < 8; pass++) {
      int n = fm.getFunctionCount();
      seedCallTargets();
      seedJumpTables();
      analyzeChanges(currentProgram);
      if (fm.getFunctionCount() == n) break;
    }
    if (name.equalsIgnoreCase("FRONTIER.EXE")) annotateMain(dir);
    else annotateOverlay(dir);
    println("SEED DONE " + name + " functions " + before + " -> " + fm.getFunctionCount());
  }

  // ---------------------------------------------------------------- helpers

  void fn(Address a, String label) {
    try {
      if (listing.getInstructionAt(a) == null) {
        if (listing.getDefinedDataContaining(a) != null) return; // never turn data into code
        disassemble(a);
      }
      if (listing.getInstructionAt(a) == null) return;
      Function f = fm.getFunctionAt(a);
      if (f == null) { f = createFunction(a, null); seeded++; }
      if (label != null) {
        if (f != null && f.getSymbol().getSource() == SourceType.DEFAULT) f.setName(label, SourceType.ANALYSIS);
        else if (f == null || !f.getName().equals(label)) createLabel(a, label, false, SourceType.ANALYSIS);
      }
    } catch (Exception e) {
      println("seed failed at " + a + ": " + e.getMessage());
    }
  }

  void dword(Address a, String label, String comment) throws Exception {
    clearListing(a, a.add(3));
    createDWord(a);
    if (label != null) createLabel(a, label, true, SourceType.ANALYSIS);
    if (comment != null) setEOLComment(a, comment);
  }

  void word(Address a, String label) throws Exception {
    clearListing(a, a.add(1));
    createWord(a);
    if (label != null) createLabel(a, label, true, SourceType.ANALYSIS);
  }

  /** Ghidra address of a flat address (segment * 16 + offset), in the block that holds it. */
  Address flatAddr(int flat) {
    for (MemoryBlock b : currentProgram.getMemory().getBlocks()) {
      if (!(b.getStart() instanceof SegmentedAddress)) continue;
      SegmentedAddress s = (SegmentedAddress) b.getStart();
      int start = s.getSegment() * 16 + s.getSegmentOffset();
      if (flat >= start && flat < start + b.getSize()) return sp.getAddress(s.getSegment(), flat - s.getSegment() * 16);
    }
    return null;
  }

  static List<Integer> readHex(File f) throws IOException {
    List<Integer> out = new ArrayList<>();
    if (!f.exists()) return out;
    for (String l : Files.readAllLines(f.toPath())) if (!l.isBlank()) out.add(Integer.parseInt(l.trim(), 16));
    return out;
  }

  /** Executed code from the runtime's coverage logs: disassemble it, and make functions at entries. */
  void seedCoverage(File dir, String name) throws Exception {
    File seeds = new File(dir.getParentFile(), "derived/coverage/seeds");
    List<Integer> ran = readHex(new File(seeds, name + ".cov"));
    List<Integer> entries = readHex(new File(seeds, name + ".entries"));
    if (ran.isEmpty() && entries.isEmpty()) return;
    int dis = 0, before = seeded;
    for (int flat : ran) {
      if (monitor.isCancelled()) return;
      Address a = flatAddr(flat);
      if (a == null || listing.getInstructionAt(a) != null) continue;
      // Executed on the runtime, so code: clear whatever analysis or seeding put there.
      Data d = listing.getDefinedDataContaining(a);
      if (d != null) clearListing(d.getMinAddress(), d.getMaxAddress());
      if (listing.getInstructionContaining(a) != null) continue; // inside another instruction
      if (disassemble(a)) dis++;
    }
    for (int flat : entries) {
      Address a = flatAddr(flat);
      if (a != null) fn(a, null);
    }
    println("COVERAGE " + name + ": " + ran.size() + " executed, " + dis + " disassembly starts, "
        + entries.size() + " entries, " + (seeded - before) + " functions created");
  }

  /** Every call/jump destination that is code without a function gets one. */
  void seedCallTargets() {
    ReferenceManager rm = currentProgram.getReferenceManager();
    List<Address> todo = new ArrayList<>();
    for (Address to : rm.getReferenceDestinationIterator(currentProgram.getMemory(), true)) {
      if (monitor.isCancelled()) return;
      for (Reference r : rm.getReferencesTo(to)) {
        if (r.getReferenceType().isCall() && fm.getFunctionAt(to) == null) { todo.add(to); break; }
      }
    }
    for (Address a : todo) fn(a, null);
  }

  /** Near tables used by `call/jmp cs:[bx+disp]`: read words while they point at plausible code. */
  void seedJumpTables() throws Exception {
    for (Instruction ins : listing.getInstructions(true)) {
      if (monitor.isCancelled()) return;
      byte[] b = ins.getBytes();
      if (b.length < 5 || (b[0] & 0xff) != 0x2e || (b[1] & 0xff) != 0xff) continue;
      int modrm = b[2] & 0xff;
      int reg = (modrm >> 3) & 7, mod = modrm >> 6;
      if (mod != 2 || (reg != 2 && reg != 4)) continue; // call/jmp near [r+disp16]
      if (ins.getAddress().getAddressSpace() != sp) continue;
      int seg = ((SegmentedAddress) ins.getAddress()).getSegment();
      int table = (b[3] & 0xff) | (b[4] & 0xff) << 8;
      Address t = at(seg, table);
      setEOLComment(ins.getAddress(), "near table at " + t);
      for (int i = 0; i < 64; i++) {
        Address e = t.add(2 * i);
        if (listing.getInstructionContaining(e) != null) break;
        int off = u16(e);
        if (off == 0 || off >= 0xfff0) break;
        Address target = at(seg, off);
        if (!currentProgram.getMemory().contains(target)) break;
        if (target.compareTo(t) >= 0 && target.compareTo(t.add(2 * i + 1)) <= 0) break;
        word(e, null);
        currentProgram.getReferenceManager().addMemoryReference(e, target,
          reg == 2 ? RefType.COMPUTED_CALL : RefType.COMPUTED_JUMP, SourceType.ANALYSIS, 0);
        if (reg == 2) fn(target, null);
        else if (listing.getInstructionAt(target) == null) disassemble(target);
      }
    }
  }

  // ---------------------------------------------------------------- overlays on disk

  static class Export { int slot; int off; Export(int s, int o) { slot = s; off = o; } }

  /** Driver export list: [psp][dest][far ptr…][0:0] at 005e → DS slots dest, dest+4, … */
  static List<Export> driverExports(Mz m) {
    List<Export> l = new ArrayList<>();
    int dest = m.u16(0x60);
    for (int p = 0x62; m.u16(p) != 0 || m.u16(p + 2) != 0; p += 4) l.add(new Export(dest + (p - 0x62), m.u16(p)));
    return l;
  }

  static String tag(String file) { return file.replace("EL2", "").replace(".OVL", ""); }

  // ---------------------------------------------------------------- FRONTIER.EXE

  Map<Integer, String> slotNotes = new HashMap<>();
  Map<Integer, Address> mainVec = new HashMap<>();

  void seedMain(File dir) throws Exception {
    int ds = MAIN_DS_RAW + LOAD_SEG;
    String[][] named = {
      { "06f1", "startup_init" }, { "0be5", "modules_reset" }, { "0c13", "modules_init_all" },
      { "d34c", "load_overlay" }, { "d608", "module_thunk_common" }, { "d6be", "load_module" },
      { "d7f0", "modules_evict_unlocked" }, { "d816", "modules_evict_round_robin" },
      { "d85f", "module_unload" }, { "e29d", "open_file_retry" }, { "e4c2", "load_video_driver" },
      { "ee5b", "load_sound_driver" }, { "ee5a", "sound_stub_retf" } };
    for (String[] n : named) fn(at(LOAD_SEG, Integer.parseInt(n[0], 16)), n[1]);
    for (int i = 0; i < 12; i++) fn(at(LOAD_SEG, 0xd5b4 + 7 * i), String.format("module_thunk_h%02x", 4 + 4 * i));

    // Main vector table: far pointers to the program's own routines (relocated in memory).
    for (int o = 0; o < MAIN_VEC_END; o += 4) {
      Address slot = at(ds, o);
      int off = u16(slot), seg = u16(slot.add(2));
      Address target = at(seg, off);
      if (!currentProgram.getMemory().contains(target)) continue;
      mainVec.put(o, target);
      dword(slot, String.format("vec_%04x", o), "-> " + target);
      currentProgram.getReferenceManager().addMemoryReference(slot, target, RefType.DATA, SourceType.ANALYSIS, 0);
      fn(target, null);
      Function f = fm.getFunctionAt(target);
      if (f != null && f.getSymbol().getSource() == SourceType.DEFAULT)
        f.setName(String.format("v%04x_%s", o, target.toString().replace(":", "_")), SourceType.ANALYSIS);
    }

    // Overlay-filled slots: label each with the routines that can occupy it.
    for (String file : DRIVERS) {
      File f = new File(dir, file);
      if (!f.isFile()) continue;
      for (Export e : driverExports(new Mz(f))) {
        String note = file + " 1000:" + String.format("%04x", e.off);
        slotNotes.merge(e.slot, note, (a, b) -> a + " | " + b);
      }
    }
    for (Map.Entry<Integer, String> e : slotNotes.entrySet()) {
      Address slot = at(ds, e.getKey());
      String t = e.getKey() < 0x270 ? "MCGA3" : e.getKey() < 0x32c ? "MATH3" : e.getKey() < 0x358 ? "SOUND" : "SETUP";
      dword(slot, String.format("%s_v%04x", t, e.getKey()), "filled at load: " + e.getValue());
    }
    for (int i = 0; i < 64; i++) {
      Address s = at(ds, MODULE_TABLE + 8 * i);
      dword(s, String.format("module_hdr_%02d", i), "far ptr to EL2M" + String.format("%02d", i) + " header (or DS:035c stub)");
      dword(s.add(4), String.format("module_word_%02d", i), null);
    }
    createLabel(at(ds, MODULE_STUB), "module_stub_header", true, SourceType.ANALYSIS);
    createLabel(at(ds, 0x596), "module_lock", true, SourceType.ANALYSIS);
  }

  /** Comment and override `lcall [disp16]` / module-handler calls in the analysed code. */
  void annotateMain(File dir) throws Exception {
    Map<Integer, Mz> mods = new HashMap<>();
    for (int i = 0; i < 64; i++) {
      File f = new File(dir, String.format("EL2M%02d.OVL", i));
      if (f.isFile()) mods.put(i, new Mz(f));
    }
    ReferenceManager rm = currentProgram.getReferenceManager();
    int overrides = 0;
    Instruction prev2 = null, prev = null;
    for (Instruction ins : listing.getInstructions(true)) {
      byte[] b = ins.getBytes();
      if (b.length == 4 && (b[0] & 0xff) == 0xff && ((b[1] & 0xff) == 0x1e || (b[1] & 0xff) == 0x2e)) {
        int disp = (b[2] & 0xff) | (b[3] & 0xff) << 8;
        boolean call = (b[1] & 0xff) == 0x1e;
        Address t = mainVec.get(disp);
        if (t != null) {
          for (Reference r : ins.getReferencesFrom())
            if (r.getReferenceType().isOverride()) rm.delete(r);
          Reference r = rm.addMemoryReference(ins.getAddress(), t,
            call ? RefType.CALL_OVERRIDE_UNCONDITIONAL : RefType.JUMP_OVERRIDE_UNCONDITIONAL, SourceType.ANALYSIS, 0);
          rm.setPrimary(r, true);
          overrides++;
        } else if (slotNotes.containsKey(disp)) {
          setEOLComment(ins.getAddress(), "overlay slot DS:" + String.format("%04x", disp) + " = " + slotNotes.get(disp));
        }
      }
      // mov si,imm16 ; les bp,[si+0396h] ; lcall es:[bp+N]
      if (b.length == 4 && (b[0] & 0xff) == 0x26 && (b[1] & 0xff) == 0xff && ((b[2] & 0xff) == 0x5e || (b[2] & 0xff) == 0x6e)
          && prev != null && prev.getLength() == 4 && (prev.getByte(0) & 0xff) == 0xc4 && (prev.getByte(1) & 0xff) == 0xac) {
        int h = b[3] & 0xff;
        String c = String.format("module handler +%02x (slot = SI/8)", h);
        if (prev2 != null && prev2.getLength() == 3 && (prev2.getByte(0) & 0xff) == 0xbe) {
          int slot = ((prev2.getByte(1) & 0xff) | (prev2.getByte(2) & 0xff) << 8) / 8;
          Mz m = mods.get(slot);
          c = String.format("EL2M%02d handler +%02x", slot, h)
            + (m != null && h >= 4 && h <= 0x30 ? String.format(" -> EL2M%02d 1000:%04x", slot, m.u16(0x6a + h)) : "");
        }
        setEOLComment(ins.getAddress(), c);
      }
      prev2 = prev;
      prev = ins;
    }
    println("call overrides " + overrides);
  }

  // ---------------------------------------------------------------- EL2*.OVL

  Map<Integer, Address> mainTargets = new HashMap<>();

  void seedOverlay(File dir, String name) throws Exception {
    String tg = tag(name);
    Address base = at(LOAD_SEG, 0);
    // Ghidra runs the stub's code on into the export block; drop that and redo it as data.
    Function entry = fm.getFunctionAt(base);
    if (entry != null) {
      AddressSetView body = entry.getBody();
      fm.removeFunction(base);
      clearListing(body);
    }
    int dest = u16(at(LOAD_SEG, 0x60));
    word(at(LOAD_SEG, 0x5e), "ovl_psp");
    word(at(LOAD_SEG, 0x60), "ovl_export_dest");
    int p = 0x62;
    for (; u16(at(LOAD_SEG, p)) != 0 || u16(at(LOAD_SEG, p + 2)) != 0; p += 4) {
      Address slot = at(LOAD_SEG, p);
      Address t = at(u16(slot.add(2)), u16(slot));
      String label = String.format("%s_v%04x", tg, dest + p - 0x62);
      dword(slot, "export_" + label, "-> main DS:" + String.format("%04x", dest + p - 0x62));
      currentProgram.getReferenceManager().addMemoryReference(slot, t, RefType.DATA, SourceType.ANALYSIS, 0);
      fn(t, label);
    }
    dword(at(LOAD_SEG, p), "ovl_export_end", null);
    if (dest == 0 && p == 0x62) {
      // EL2Mnn module: header H at 006a (main stores 005e+0ch); H-4 far ptr, H+0 dword,
      // H+04..H+30 twelve far-pointer handlers, H+34 dword.
      dword(at(LOAD_SEG, 0x66), "mod_patch_table_ptr", "byte pairs, applied when DS:890e bit 4 set");
      dword(at(LOAD_SEG, 0x6a), "mod_header", "H+00: copied to module_word_nn on first load");
      for (int h = 4; h <= 0x30; h += 4) {
        Address slot = at(LOAD_SEG, 0x6a + h);
        Address t = at(u16(slot.add(2)), u16(slot));
        String label = String.format("%s_h%02x", tg, h);
        dword(slot, "hdr_" + label, null);
        currentProgram.getReferenceManager().addMemoryReference(slot, t, RefType.DATA, SourceType.ANALYSIS, 0);
        fn(t, label);
      }
      dword(at(LOAD_SEG, 0x6a + 0x34), "mod_header_34", "H+34: copied to main DS:05d6+slot*4");
    }
    fn(base, "ovl_tsr_stub");
  }

  void annotateOverlay(File dir) throws Exception {
    File exe = new File(dir, "FRONTIER.EXE");
    if (!exe.isFile()) return;
    Mz m = new Mz(exe);
    int ds = MAIN_DS_RAW * 16;
    for (Instruction ins : listing.getInstructions(true)) {
      byte[] b = ins.getBytes();
      if (b.length != 4 || (b[0] & 0xff) != 0xff || ((b[1] & 0xff) != 0x1e && (b[1] & 0xff) != 0x2e)) continue;
      int disp = (b[2] & 0xff) | (b[3] & 0xff) << 8;
      if (disp >= MAIN_VEC_END || (disp & 3) != 0) continue;
      int off = m.u16(ds + disp), seg = m.u16(ds + disp + 2) + LOAD_SEG;
      setEOLComment(ins.getAddress(), String.format("main vec_%04x -> FRONTIER.EXE %04x:%04x", disp, seg, off));
    }
  }
}
