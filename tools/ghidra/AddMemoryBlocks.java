// Build a program's memory from raw pieces: a booter game's load map, an unpacked memory dump,
// a console's ROM banks, or overlays loaded at run time. Each piece becomes a memory block at
// its address, so references between the pieces resolve and Ghidra can analyse them as one.
//
// Usage: analyzeHeadless PROJECT NAME -import FIRST.bin -loader BinaryLoader \
//   -loader-baseAddr ADDR -processor LANGUAGE -postScript AddMemoryBlocks.java MAP.tsv
// or, on a program already in a project: scripts/ghidra-run.sh BINARY blocks OUT MAP.tsv
//
// MAP.tsv has one line per piece: file<TAB>address[<TAB>offset[<TAB>length[<TAB>name[<TAB>code]]]].
// "code" in the last column disassembles from the piece's first byte and makes a function there.
// Paths are relative to the map file. Addresses take Ghidra's form, such as 0x8000 or 1000:0000
// for 16-bit segmented code. A piece that overlaps an existing block is added as an overlay
// block, as for code banks that share an address range. Lines starting with # are comments.
// The script writes a short report to OUTPUT when one is given before the map (ghidra-run.sh).
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.mem.*;
import java.io.*;
import java.nio.file.*;

public class AddMemoryBlocks extends GhidraScript {
  @Override
  public void run() throws Exception {
    String[] args = getScriptArgs();
    File map = new File(args[args.length - 1]);
    PrintWriter report = args.length > 1 ? new PrintWriter(new FileWriter(args[0])) : null;
    Memory memory = currentProgram.getMemory();
    int added = 0;
    for (String line : Files.readAllLines(map.toPath())) {
      if (line.isBlank() || line.startsWith("#")) continue;
      String[] f = line.split("\t");
      File piece = new File(f[0]).isAbsolute() ? new File(f[0]) : new File(map.getParentFile(), f[0]);
      Address at = toAddr(f[1]);
      long offset = f.length > 2 && !f[2].isEmpty() ? Long.decode(f[2]) : 0;
      byte[] all = Files.readAllBytes(piece.toPath());
      int length = f.length > 3 && !f[3].isEmpty() ? Long.decode(f[3]).intValue() : (int) (all.length - offset);
      String name = f.length > 4 ? f[4] : piece.getName() + "@" + f[1];
      byte[] bytes = java.util.Arrays.copyOfRange(all, (int) offset, (int) offset + length);
      boolean overlap = memory.intersects(at, at.add(length - 1));
      MemoryBlock block = memory.createInitializedBlock(name.replaceAll("[^A-Za-z0-9_.@:-]", "_"), at,
        new ByteArrayInputStream(bytes), length, monitor, overlap);
      block.setRead(true); block.setWrite(true); block.setExecute(true);
      if (f.length > 5 && f[5].trim().equals("code")) { disassemble(block.getStart()); createFunction(block.getStart(), null); }
      String msg = (overlap ? "overlay " : "block   ") + block.getStart() + " - " + block.getEnd() + "  " + name;
      println(msg);
      if (report != null) report.println(msg);
      added++;
    }
    if (report != null) { report.println(added + " blocks added"); report.close(); }
    analyzeChanges(currentProgram);     // auto-analysis over the new blocks
    println(added + " memory blocks added and analysed");
  }
}
