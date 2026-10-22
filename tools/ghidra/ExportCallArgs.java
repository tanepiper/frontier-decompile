// List every call to chosen functions with the arguments the decompiler can resolve at each
// call: constants, strings and global addresses. Answers "every sound the game plays and its
// id", "every file it opens", "every random draw and its range" in one pass.
//
// Usage: analyzeHeadless PROJECT NAME -process PROGRAM -noanalysis -scriptPath tools/ghidra \
//   -postScript ExportCallArgs.java OUTPUT.tsv TARGET [TARGET ...]
// TARGET is a function name (as Ghidra or your labels name it) or an address. The output has
// one line per call: callee, call address, calling function, then each argument. An argument
// the decompiler cannot reduce to a constant shows as the expression's variable name.
// It works on the decompiler's P-Code, so it is the same for x86, 68000, MIPS or any other
// processor Ghidra supports. scripts/ghidra-run.sh BINARY callargs OUT TARGET... runs it.
import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.*;
import ghidra.program.model.pcode.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class ExportCallArgs extends GhidraScript {
  private String describe(Varnode v) {
    if (v == null) return "?";
    if (v.isConstant()) {
      long value = v.getOffset();
      String text = "0x" + Long.toHexString(value);
      try {
        Address a = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(value);
        Data d = getDataAt(a);
        if (d != null && d.hasStringValue())
          return text + " \"" + String.valueOf(d.getValue()).replace("\t", "\\t").replace("\n", "\\n") + "\"";
        Symbol s = getSymbolAt(a);
        if (s != null && value > 0xFFFF) return text + " " + s.getName(true);
      } catch (Exception e) { /* not an address */ }
      return text + " (" + value + ")";
    }
    if (v.isAddress()) {
      Symbol s = getSymbolAt(v.getAddress());
      return "[" + v.getAddress() + (s == null ? "" : " " + s.getName(true)) + "]";
    }
    HighVariable h = v.getHigh();
    PcodeOp def = v.getDef();
    // A pointer to a global often reaches the call as a COPY or a PTRSUB of a constant.
    if (def != null && (def.getOpcode() == PcodeOp.COPY || def.getOpcode() == PcodeOp.CAST)) return describe(def.getInput(0));
    if (def != null && def.getOpcode() == PcodeOp.PTRSUB && def.getInput(0).isConstant() && def.getInput(0).getOffset() == 0)
      return describe(def.getInput(1));
    if (def != null && (def.getOpcode() == PcodeOp.PTRSUB || def.getOpcode() == PcodeOp.INT_ADD)
        && def.getInput(0).isRegister()) return "&local";        // the address of a stack variable
    return h == null || h.getName() == null || h.getName().equals("UNNAMED") ? "?" : h.getName();
  }

  @Override
  public void run() throws Exception {
    String[] args = getScriptArgs();
    File output = new File(args[0]);
    if (output.getParentFile() != null) output.getParentFile().mkdirs();
    FunctionManager functions = currentProgram.getFunctionManager();
    Map<Address, String> targets = new LinkedHashMap<>();
    for (int i = 1; i < args.length; i++) {
      List<Function> found = new ArrayList<>();
      Function f = null;
      try { f = functions.getFunctionAt(toAddr(args[i])); } catch (Exception e) { /* a name */ }
      if (f != null) found.add(f);
      else for (Function g : functions.getFunctions(true)) if (g.getName().equals(args[i])) found.add(g);
      for (Function g : functions.getExternalFunctions()) if (g.getName().equals(args[i])) found.add(g);
      if (found.isEmpty()) println("no function named " + args[i]);
      for (Function g : found) {
        targets.put(g.getEntryPoint(), g.getName(true));
        for (Address thunk : g.getFunctionThunkAddresses(true) == null ? new Address[0] : g.getFunctionThunkAddresses(true))
          targets.put(thunk, g.getName(true));
      }
    }
    // The call sites: every instruction that refers to a target, directly or through the
    // pointer slot that holds it (an import table entry, or a thunk's jump slot).
    Map<Address, String> sites = new HashMap<>();
    Set<Function> callers = new LinkedHashSet<>();
    for (Map.Entry<Address, String> t : targets.entrySet()) {
      Deque<Address> todo = new ArrayDeque<>(List.of(t.getKey()));
      Set<Address> seen = new HashSet<>();
      while (!todo.isEmpty()) {
        Address to = todo.pop();
        if (!seen.add(to)) continue;
        for (Reference r : getReferencesTo(to)) {
          Address from = r.getFromAddress();
          if (getInstructionAt(from) == null) { todo.push(from); continue; }   // a pointer slot
          Function c = functions.getFunctionContaining(from);
          if (c != null && c.isThunk()) { todo.push(c.getEntryPoint()); continue; }
          sites.put(from, t.getValue());
          if (c != null) callers.add(c);
        }
      }
    }
    DecompInterface decompiler = new DecompInterface();
    decompiler.openProgram(currentProgram);
    int calls = 0;
    try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(output)))) {
      w.println("# callee\tcall address\tcalling function\targuments...");
      for (Function caller : callers) {
        if (monitor.isCancelled()) break;
        DecompileResults res = decompiler.decompileFunction(caller, 60, monitor);
        HighFunction high = res.getHighFunction();
        if (high == null) { w.println("# decompile failed: " + caller.getName(true)); continue; }
        Iterator<PcodeOpAST> ops = high.getPcodeOps();
        while (ops.hasNext()) {
          PcodeOpAST op = ops.next();
          if (op.getOpcode() != PcodeOp.CALL && op.getOpcode() != PcodeOp.CALLIND) continue;
          String name = sites.get(op.getSeqnum().getTarget());
          if (name == null && op.getOpcode() == PcodeOp.CALL) name = targets.get(op.getInput(0).getAddress());
          if (name == null) continue;
          StringBuilder line = new StringBuilder(name + "\t" + op.getSeqnum().getTarget() + "\t" + caller.getName(true));
          for (int k = 1; k < op.getNumInputs(); k++) line.append("\t").append(describe(op.getInput(k)));
          w.println(line);
          calls++;
        }
      }
    } finally {
      decompiler.dispose();
    }
    println("Call arguments (" + calls + " calls) written to " + output);
  }
}
