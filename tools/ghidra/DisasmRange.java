// Disassemble an address range linearly (no functions made) and write it (address, bytes,
// instruction). For blocks of short stubs that share code, such as the text formatters.
// Usage: analyzeHeadless PROJECT NAME -process PROGRAM -noanalysis -readOnly -scriptPath tools/ghidra \
//   -postScript DisasmRange.java OUTPUT.txt 1000:e9b0 1000:ed00
// Output belongs under analysis/ (git-ignored).
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.*;
import java.io.*;

public class DisasmRange extends GhidraScript {
  @Override
  public void run() throws Exception {
    String[] args = getScriptArgs();
    File output = new File(args[0]);
    Address a = toAddr(args[1]);
    Address end = toAddr(args[2]);
    Listing listing = currentProgram.getListing();
    try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(output)))) {
      while (a.compareTo(end) < 0) {
        if (listing.getInstructionAt(a) == null) {
          listing.clearCodeUnits(a, a, false);
          disassemble(a);
        }
        Instruction ins = listing.getInstructionAt(a);
        if (ins == null) {
          w.printf("%s  %02x               ??%n", a, getByte(a) & 0xff);
          a = a.add(1);
          continue;
        }
        StringBuilder hex = new StringBuilder();
        for (byte b : ins.getBytes()) hex.append(String.format("%02x", b & 0xff));
        w.printf("%s  %-16s %s%n", ins.getAddress(), hex, ins);
        a = a.add(ins.getLength());
      }
    }
  }
}
