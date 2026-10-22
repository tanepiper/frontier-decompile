// Apply a labels file (labels/names.tsv style: address<TAB>label[<TAB>evidence]) to the program:
//   - an address in executable memory becomes a function (created if Ghidra missed it) with that name;
//   - any other address gets the label as its primary symbol (a global variable).
// Lines starting with '#' and rows whose address is not in the program are skipped. Addresses
// are as Ghidra prints them: 140001000 or 0x140001000 (flat), 1000:873a (segmented, 16-bit).
// Usage: analyzeHeadless ... -postScript ApplyLabels.java REPORT.txt LABELS.tsv
//   (scripts/ghidra-export.sh --labels FILE runs it before the export; scripts/ghidra-run.sh
//    BINARY labels REPORT FILE runs it on an analysed project).
// It does not set types, parameters or comments.
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.nio.file.*;
import java.util.List;
public class ApplyLabels extends GhidraScript {
  public void run() throws Exception {
    String[] args = getScriptArgs();
    List<String> lines = Files.readAllLines(Paths.get(args[1]));
    int funcs = 0, created = 0, data = 0, skipped = 0;
    AddressFactory af = currentProgram.getAddressFactory();
    Memory mem = currentProgram.getMemory();
    FunctionManager fm = currentProgram.getFunctionManager();
    SymbolTable st = currentProgram.getSymbolTable();
    for (String line : lines) {
      if (line.startsWith("#") || line.isBlank()) continue;
      String[] cols = line.split("\t");
      if (cols.length < 2) continue;
      String text = cols[0].trim().toLowerCase();
      if (text.startsWith("0x")) text = text.substring(2);
      String name = cols[1].trim().replace(' ', '_');
      Address a = af.getAddress(text);
      if (a == null || !mem.contains(a)) { skipped++; continue; }
      MemoryBlock block = mem.getBlock(a);
      if (block != null && block.isExecute()) {
        Function f = fm.getFunctionAt(a);
        if (f == null) {
          if (getInstructionAt(a) == null) new DisassembleCommand(a, null, true).applyTo(currentProgram, monitor);
          f = createFunction(a, name);
          if (f == null) { skipped++; continue; }
          created++;
        } else {
          f.setName(name, SourceType.USER_DEFINED);
        }
        funcs++;
      } else {
        st.createLabel(a, name, SourceType.USER_DEFINED).setPrimary();
        data++;
      }
    }
    String report = "LABELS DONE " + funcs + " functions (" + created + " created), " + data + " data labels, " + skipped + " skipped";
    println(report);
    try (PrintWriter w = new PrintWriter(new File(args[0]))) { w.println(report); }
  }
}
