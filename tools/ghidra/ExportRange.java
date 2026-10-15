// Disassemble a linear address range (code that analysis did not reach, such as interrupt
// handlers), without saving changes. Output stays under analysis/ (git-ignored).
// Usage: analyzeHeadless PROJECT NAME -process PROGRAM -noanalysis -readOnly -scriptPath tools/ghidra \
//   -postScript ExportRange.java OUTPUT.txt 1000:ceae 1000:cf80 [START END ...]
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.io.*;

public class ExportRange extends GhidraScript {
  @Override
  public void run() throws Exception {
    String[] args = getScriptArgs();
    File output = new File(args[0]);
    output.getParentFile().mkdirs();
    Listing listing = currentProgram.getListing();
    try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(output)))) {
      for (int i = 1; i + 1 < args.length; i += 2) {
        Address start = toAddr(args[i]);
        Address end = toAddr(args[i + 1]);
        AddressSet set = new AddressSet(start, end);
        Address a = start;
        while (a != null && a.compareTo(end) <= 0) {
          Instruction ins = listing.getInstructionAt(a);
          if (ins == null) {
            listing.clearCodeUnits(a, a, false);
            new DisassembleCommand(new AddressSet(a, a), set, false).applyTo(currentProgram, monitor);
            ins = listing.getInstructionAt(a);
          }
          if (ins == null) {
            w.printf("%s  %02x               ??%n", a, getByte(a) & 0xff);
            a = a.next();
            continue;
          }
          StringBuilder hex = new StringBuilder();
          for (byte b : ins.getBytes()) hex.append(String.format("%02x", b & 0xff));
          w.printf("%s  %-16s %s%n", ins.getAddress(), hex, ins);
          a = ins.getMaxAddress().next();
        }
        w.println("// ----");
      }
    }
    println("Range written to " + output);
  }
}
