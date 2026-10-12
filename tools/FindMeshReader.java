// Research utility: all generated output must stay in RobloxStudio/analysis.
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.Reference;
import java.io.PrintWriter;
import java.nio.file.Path;
import java.nio.file.Files;
import java.nio.file.StandardOpenOption;
import java.util.HashSet;

public class FindMeshReader extends GhidraScript {
    public void run() throws Exception {
        Path root = Path.of(System.getProperty("user.dir"), "RobloxStudio", "analysis").toRealPath();
        Path target = Path.of(getScriptArgs()[0]).toAbsolutePath().normalize();
        if (!target.getParent().toRealPath().startsWith(root)) {
            throw new IllegalArgumentException("Output must stay under RobloxStudio/analysis");
        }
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        HashSet<Function> functions = new HashSet<>();
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(target, StandardOpenOption.CREATE_NEW))) {
            output.println("PROGRAM " + currentProgram.getName() + " FUNCTIONS " +
                currentProgram.getFunctionManager().getFunctionCount());
            for (Data data : currentProgram.getListing().getDefinedData(true)) {
                if (!(data.getValue() instanceof String)) continue;
                String value = (String) data.getValue();
                if (!(value.contains("version 2.00") || value.contains("version 1.00") ||
                      value.contains("version 1.01") || value.contains("ReadFileMesh"))) continue;
                output.println("STRING " + data.getAddress() + " " + value);
                for (Reference ref : getReferencesTo(data.getAddress())) {
                    Function function = getFunctionContaining(ref.getFromAddress());
                    output.println("REFERENCE " + ref.getFromAddress() + " " +
                        (function == null ? "NO_FUNCTION" : function.getName(true) + " " + function.getEntryPoint()));
                    if (function != null) functions.add(function);
                }
            }
            for (Function function : functions) {
                output.println("FUNCTION " + function.getName(true) + " ENTRY " + function.getEntryPoint());
                DecompileResults result = decompiler.decompileFunction(function, 60, monitor);
                if (result.decompileCompleted()) output.println(result.getDecompiledFunction().getC());
                else output.println("DECOMPILE_FAILED " + result.getErrorMessage());
            }
        } finally { decompiler.dispose(); }
    }
}
