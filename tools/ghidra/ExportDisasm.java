// Write a disassembly listing of every function in an analyzed program (address, bytes,
// instruction), for porting arithmetic exactly. Output stays under analysis/ (git-ignored).
// Usage: analyzeHeadless PROJECT NAME -process PROGRAM -noanalysis -scriptPath tools/ghidra \
//   -postScript ExportDisasm.java analysis/export/PROGRAM/disasm.txt
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.io.*;

public class ExportDisasm extends GhidraScript {
  @Override
  public void run() throws Exception {
    File output = new File(getScriptArgs()[0]);
    output.getParentFile().mkdirs();
    Listing listing = currentProgram.getListing();
    try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(output)))) {
      for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
        w.println("// ==== " + f.getEntryPoint() + " " + f.getName());
        for (Instruction ins : listing.getInstructions(f.getBody(), true)) {
          StringBuilder hex = new StringBuilder();
          for (byte b : ins.getBytes()) hex.append(String.format("%02x", b & 0xff));
          w.printf("%s  %-16s %s%n", ins.getAddress(), hex, ins);
        }
      }
    }
    println("Disassembly written to " + output);
  }
}
