// Disassemble and create functions at chosen addresses, then write their instructions (address,
// bytes, instruction). For code the analysis did not reach (entries found in tables at run time).
// Usage: analyzeHeadless PROJECT NAME -process PROGRAM -noanalysis -scriptPath tools/ghidra \
//   -postScript DisasmAt.java OUTPUT.txt 1000:4bad 1000:4bff ...
// The project is changed (functions are saved); output belongs under analysis/ (git-ignored).
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.*;
import java.io.*;

public class DisasmAt extends GhidraScript {
  @Override
  public void run() throws Exception {
    String[] args = getScriptArgs();
    File output = new File(args[0]);
    output.getParentFile().mkdirs();
    Listing listing = currentProgram.getListing();
    try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(output)))) {
      for (int i = 1; i < args.length; i++) {
        Address a = toAddr(args[i]);
        if (listing.getInstructionAt(a) == null) disassemble(a);
        Function f = currentProgram.getFunctionManager().getFunctionAt(a);
        if (f == null) f = createFunction(a, null);
        w.println("// ==== " + a + " " + (f == null ? "<no function>" : f.getName()));
        if (f == null) continue;
        for (Instruction ins : listing.getInstructions(f.getBody(), true)) {
          StringBuilder hex = new StringBuilder();
          for (byte b : ins.getBytes()) hex.append(String.format("%02x", b & 0xff));
          w.printf("%s  %-16s %s%n", ins.getAddress(), hex, ins);
        }
      }
    }
  }
}
