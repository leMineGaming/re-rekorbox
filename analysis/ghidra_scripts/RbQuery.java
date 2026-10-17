// Read-only query script for the persistent rekordbox Ghidra project.
// Usage (headless, -noanalysis -readOnly):
//   -postScript RbQuery.java <outfile> <command> [args...]
// Commands:
//   strings <regex>            defined strings matching regex + referencing functions
//   xrefs <addr>               references to an address (with containing function)
//   decomp <addr|name> ...     decompiled C of the containing function(s)
//   callers <addr|name>        functions calling the given function
//   callees <addr|name>        functions called by the given function
//   bytes <addr> <len>         hex dump of memory
// @category rekordbox-rs
import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;
import java.util.regex.*;

public class RbQuery extends GhidraScript {
    private PrintWriter out;

    @Override
    public void run() throws Exception {
        String[] a = getScriptArgs();
        if (a.length == 1 && a[0].startsWith("@")) {
            // args file: first line = outfile, then one argument per line (avoids cmd.exe metacharacters)
            a = java.nio.file.Files.readAllLines(new File(a[0].substring(1)).toPath(),
                java.nio.charset.StandardCharsets.UTF_8).toArray(new String[0]);
        }
        out =new PrintWriter(new OutputStreamWriter(new FileOutputStream(a[0]), "UTF-8"));
        try {
            out.println("# program=" + currentProgram.getName() + " sha256=" + currentProgram.getExecutableSHA256()
                + " ghidra=" + getGhidraVersion() + " cmd=" + String.join(" ", Arrays.copyOfRange(a, 1, a.length)));
            switch (a[1]) {
                case "strings": strings(a[2]); break;
                case "xrefs": xrefs(toAddr(a[2])); break;
                case "decomp": for (int i = 2; i < a.length; i++) decomp(a[i]); break;
                case "callers": callers(a[2]); break;
                case "callees": callees(a[2]); break;
                case "bytes": bytes(toAddr(a[2]), Integer.decode(a[3])); break;
                case "fourcc": fourcc(Arrays.copyOfRange(a, 2, a.length)); break;
                case "ptrtab": ptrtab(toAddr(a[2]), Integer.decode(a[3])); break;
                case "funcs": funcs(toAddr(a[2]), toAddr(a[3])); break;
                case "syms": syms(a[2]); break;
                case "vtable": vtable(toAddr(a[2]), Integer.decode(a[3])); break;
                case "rtti": for (int i = 2; i < a.length; i++) rtti(a[i]); break;
                case "xrefrange": xrefrange(toAddr(a[2]), toAddr(a[3]), Integer.decode(a[4])); break;
                case "strrange": strrange(toAddr(a[2]), toAddr(a[3])); break;
                case "asm": asm(a[2]); break;
                case "imm": imm(Arrays.copyOfRange(a, 2, a.length)); break;
                default: out.println("unknown command " + a[1]);
            }
        } finally {
            out.close();
        }
    }

    private Function fn(String sel) {
        if (sel.startsWith("0x")) {
            Address ad = toAddr(sel);
            Function f = getFunctionAt(ad);
            return f != null ? f : getFunctionContaining(ad);
        }
        List<Function> fs = getGlobalFunctions(sel);
        return fs.isEmpty() ? null : fs.get(0);
    }

    private String where(Address from) {
        Function f = getFunctionContaining(from);
        return f == null ? "<no function>" : f.getName() + "@" + f.getEntryPoint();
    }

    private void strings(String regex) {
        Pattern p = Pattern.compile(regex);
        DataIterator it = currentProgram.getListing().getDefinedData(true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Data d = it.next();
            if (!d.hasStringValue()) continue;
            Object v = d.getValue();
            String s = v == null ? "" : v.toString();
            if (!p.matcher(s).find()) continue;
            out.println("STR " + d.getAddress() + " " + d.getDataType().getName() + " \"" + s.replace("\n", "\\n") + "\"");
            for (Reference r : getReferencesTo(d.getAddress()))
                out.println("    ref " + r.getFromAddress() + " " + r.getReferenceType() + " in " + where(r.getFromAddress()));
        }
    }

    private void xrefs(Address ad) {
        for (Reference r : getReferencesTo(ad))
            out.println("ref " + r.getFromAddress() + " " + r.getReferenceType() + " in " + where(r.getFromAddress()));
    }

    private void decomp(String sel) {
        Function f = fn(sel);
        if (f == null) { out.println("## no function for " + sel); return; }
        DecompInterface ifc = new DecompInterface();
        ifc.openProgram(currentProgram);
        DecompileResults r = ifc.decompileFunction(f, 180, monitor);
        out.println("## FUNCTION " + f.getName() + " @ " + f.getEntryPoint() + " body=" + f.getBody().getMinAddress()
            + ".." + f.getBody().getMaxAddress() + " sig=" + f.getSignature().getPrototypeString());
        if (r != null && r.decompileCompleted()) out.println(r.getDecompiledFunction().getC());
        else out.println("## decompile failed: " + (r == null ? "null" : r.getErrorMessage()));
        ifc.dispose();
    }

    private void callers(String sel) {
        Function f = fn(sel);
        if (f == null) { out.println("## no function for " + sel); return; }
        for (Function c : f.getCallingFunctions(monitor)) out.println("caller " + c.getName() + "@" + c.getEntryPoint());
    }

    private void callees(String sel) {
        Function f = fn(sel);
        if (f == null) { out.println("## no function for " + sel); return; }
        for (Function c : f.getCalledFunctions(monitor)) out.println("callee " + c.getName() + "@" + c.getEntryPoint());
    }

    // Find instructions whose scalar operands equal a four-character code, in big- or little-endian order.
    private void fourcc(String[] codes) {
        Map<Long, String> want = new HashMap<>();
        for (String c : codes) {
            byte[] b = c.getBytes(java.nio.charset.StandardCharsets.US_ASCII);
            long be = ((b[0] & 0xffL) << 24) | ((b[1] & 0xffL) << 16) | ((b[2] & 0xffL) << 8) | (b[3] & 0xffL);
            long le = ((b[3] & 0xffL) << 24) | ((b[2] & 0xffL) << 16) | ((b[1] & 0xffL) << 8) | (b[0] & 0xffL);
            want.put(be, c + "/BE");
            want.put(le, c + "/LE");
        }
        Map<String, Integer> perFn = new TreeMap<>();
        InstructionIterator it = currentProgram.getListing().getInstructions(true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Instruction ins = it.next();
            for (int i = 0; i < ins.getNumOperands(); i++) {
                for (Object o : ins.getOpObjects(i)) {
                    if (!(o instanceof ghidra.program.model.scalar.Scalar)) continue;
                    long v = ((ghidra.program.model.scalar.Scalar) o).getUnsignedValue() & 0xffffffffL;
                    String tag = want.get(v);
                    if (tag == null) continue;
                    String w = where(ins.getAddress());
                    out.println("IMM " + tag + " " + ins.getAddress() + " " + ins + " in " + w);
                    perFn.merge(w, 1, Integer::sum);
                }
            }
        }
        out.println("# per function:");
        for (Map.Entry<String, Integer> e : perFn.entrySet()) out.println("FN " + e.getValue() + " " + e.getKey());
    }

    // Read n 64-bit pointers starting at ad and print the string (if any) each one points to.
    private void ptrtab(Address ad, int n) throws Exception {
        for (int i = 0; i < n; i++) {
            Address slot = ad.add(i * 8L);
            long p = getLong(slot);
            String s = "";
            try {
                Address t = toAddr(p);
                Data d = getDataAt(t);
                if (d != null && d.hasStringValue()) s = "\"" + d.getValue() + "\"";
                else if (currentProgram.getMemory().contains(t)) {
                    StringBuilder sb = new StringBuilder();
                    for (int k = 0; k < 64; k++) { byte c = getByte(t.add(k)); if (c == 0) break; sb.append((char) (c & 0xff)); }
                    s = "raw\"" + sb + "\"";
                }
            } catch (Exception e) { s = "?"; }
            out.println("PTR " + slot + " [" + i + "] -> 0x" + Long.toHexString(p) + " " + s);
            for (Reference r : getReferencesTo(slot))
                out.println("    ref " + r.getFromAddress() + " in " + where(r.getFromAddress()));
        }
    }

    // List functions in [start, end): entry, size, strings referenced from the body, callers count.
    private void funcs(Address start, Address end) {
        FunctionIterator it = currentProgram.getFunctionManager().getFunctions(start, true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Function f = it.next();
            if (f.getEntryPoint().compareTo(end) >= 0) break;
            List<String> strs = new ArrayList<>();
            for (Address ad : f.getBody().getAddresses(true)) {
                for (Reference r : getReferencesFrom(ad)) {
                    Data d = getDataAt(r.getToAddress());
                    if (d != null && d.hasStringValue() && strs.size() < 6) strs.add("\"" + d.getValue() + "\"");
                }
            }
            int callers = f.getCallingFunctions(monitor).size();
            out.println("FN " + f.getEntryPoint() + " size=" + f.getBody().getNumAddresses() + " callers=" + callers
                + (strs.isEmpty() ? "" : " str=" + String.join(",", strs)));
        }
    }

    // Symbols (labels, namespaces) whose full name matches regex.
    private void syms(String regex) {
        Pattern p = Pattern.compile(regex);
        SymbolIterator it = currentProgram.getSymbolTable().getAllSymbols(true);
        int n = 0;
        while (it.hasNext() && !monitor.isCancelled() && n < 400) {
            Symbol s = it.next();
            String full = s.getName(true);
            if (!p.matcher(full).find()) continue;
            out.println("SYM " + s.getAddress() + " " + s.getSymbolType() + " " + full);
            n++;
        }
    }

    // Read n vtable slots starting at ad, print the target function of each slot.
    private void vtable(Address ad, int n) throws Exception {
        for (int i = 0; i < n; i++) {
            long p = getLong(ad.add(i * 8L));
            Function f = getFunctionAt(toAddr(p));
            out.println("VT [" + i + "] +0x" + Integer.toHexString(i * 8) + " -> 0x" + Long.toHexString(p) + " "
                + (f == null ? "<no function>" : f.getName() + " size=" + f.getBody().getNumAddresses()));
        }
    }

    private static byte[] le32(long v) { return new byte[]{(byte) v, (byte) (v >> 8), (byte) (v >> 16), (byte) (v >> 24)}; }
    private static byte[] le64(long v) {
        byte[] b = new byte[8];
        for (int i = 0; i < 8; i++) b[i] = (byte) (v >> (8 * i));
        return b;
    }

    // MSVC x64 RTTI: find the vtable(s) of the class whose mangled name contains `mangled` (e.g. "SongStructInformation@djplay").
    private void rtti(String mangled) throws Exception {
        ghidra.program.model.mem.Memory mem = currentProgram.getMemory();
        long base = currentProgram.getImageBase().getOffset();
        byte[] nameBytes = (".?AV" + mangled + "@@").getBytes("US-ASCII");
        Address name = mem.findBytes(mem.getMinAddress(), nameBytes, null, true, monitor);
        if (name == null) {
            nameBytes = (".?AU" + mangled + "@@").getBytes("US-ASCII");
            name = mem.findBytes(mem.getMinAddress(), nameBytes, null, true, monitor);
        }
        if (name == null) { out.println("## no type descriptor for " + mangled); return; }
        Address td = name.subtract(0x10);
        out.println("## class " + mangled + " typeDescriptor=" + td);
        byte[] rva = le32(td.getOffset() - base);
        Address cur = mem.getMinAddress();
        while (true) {
            Address hit = mem.findBytes(cur, rva, null, true, monitor);
            if (hit == null) break;
            cur = hit.add(1);
            Address col = hit.subtract(12);
            if (getInt(col) != 1) continue;            // COL signature 1 on x64
            int offset = getInt(col.add(4));
            Address vref = mem.findBytes(mem.getMinAddress(), le64(col.getOffset()), null, true, monitor);
            while (vref != null) {
                Address vt = vref.add(8);
                out.println("COL " + col + " thisOffset=" + offset + " vtable=" + vt);
                vtable(vt, 24);
                vref = mem.findBytes(vref.add(1), le64(col.getOffset()), null, true, monitor);
            }
        }
    }

    // For each slot in [start, end) with the given step: writers/readers of the slot, plus the nearest
    // preceding string literal loaded in the referencing function (helps map runtime-initialised string tables).
    private void xrefrange(Address start, Address end, int step) {
        for (Address s = start; s.compareTo(end) < 0; s = s.add(step)) {
            for (Reference r : getReferencesTo(s)) {
                Address from = r.getFromAddress();
                String lit = "";
                Instruction ins = getInstructionAt(from);
                for (int k = 0; k < 6 && ins != null; k++) {
                    for (Reference rr : ins.getReferencesFrom()) {
                        Data d = getDataAt(rr.getToAddress());
                        if (d != null && d.hasStringValue() && lit.isEmpty()) lit = "\"" + d.getValue() + "\"";
                    }
                    ins = ins.getPrevious();
                }
                out.println("SLOT " + s + " ref " + from + " " + r.getReferenceType() + " in " + where(from) + " " + lit);
            }
        }
    }

    // Defined strings in [start, end) with the functions that reference each one.
    private void strrange(Address start, Address end) {
        DataIterator it = currentProgram.getListing().getDefinedData(new AddressSet(start, end.subtract(1)), true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Data d = it.next();
            if (!d.hasStringValue()) continue;
            Set<String> fns = new TreeSet<>();
            for (Reference r : getReferencesTo(d.getAddress())) {
                Function f = getFunctionContaining(r.getFromAddress());
                fns.add(f == null ? "?" : f.getEntryPoint().toString());
            }
            out.println("S " + d.getAddress() + " \"" + d.getValue() + "\" <- " + String.join(",", fns));
        }
    }

    // Functions whose instructions use all of the given scalar constants (hex); prints per-function hit counts.
    private void imm(String[] values) {
        long[] want = new long[values.length];
        for (int i = 0; i < values.length; i++) want[i] = Long.decode(values[i]);
        Map<String, long[]> hits = new TreeMap<>();
        InstructionIterator it = currentProgram.getListing().getInstructions(true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Instruction ins = it.next();
            for (int i = 0; i < ins.getNumOperands(); i++) {
                for (Object o : ins.getOpObjects(i)) {
                    if (!(o instanceof ghidra.program.model.scalar.Scalar)) continue;
                    long v = ((ghidra.program.model.scalar.Scalar) o).getUnsignedValue();
                    for (int k = 0; k < want.length; k++) {
                        if (v == want[k]) {
                            String w = where(ins.getAddress());
                            hits.computeIfAbsent(w, x -> new long[want.length])[k]++;
                        }
                    }
                }
            }
        }
        for (Map.Entry<String, long[]> e : hits.entrySet()) {
            boolean all = true;
            for (long c : e.getValue()) all &= c > 0;
            if (all) out.println("FN " + e.getKey() + " " + Arrays.toString(e.getValue()));
        }
    }

    // Disassembly of the function containing the selector, with referenced strings/symbols.
    private void asm(String sel) {
        Function f = fn(sel);
        if (f == null) { out.println("## no function for " + sel); return; }
        out.println("## ASM " + f.getName() + " @ " + f.getEntryPoint());
        InstructionIterator it = currentProgram.getListing().getInstructions(f.getBody(), true);
        while (it.hasNext()) {
            Instruction ins = it.next();
            StringBuilder note = new StringBuilder();
            for (Reference r : ins.getReferencesFrom()) {
                Data d = getDataAt(r.getToAddress());
                if (d != null && d.hasStringValue()) note.append(" \"").append(d.getValue()).append('"');
                else {
                    Function t = getFunctionAt(r.getToAddress());
                    if (t != null) note.append(" ").append(t.getName());
                }
            }
            out.println(ins.getAddress() + "  " + ins + (note.length() > 0 ? "   ;" + note : ""));
        }
    }

    private void bytes(Address ad, int len) throws Exception {
        byte[] b = getBytes(ad, len);
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < b.length; i++) {
            if (i % 16 == 0) sb.append(String.format("%n%s ", ad.add(i)));
            sb.append(String.format("%02x ", b[i] & 0xff));
        }
        out.println(sb);
    }
}
