// Local evidence only. Arguments: new output file, then function addresses.
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.symbol.Reference;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

public class TraceFunctions extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        Path root = Path.of(System.getProperty("user.dir"), "RobloxStudio", "analysis").toRealPath();
        Path outputPath = Path.of(args[0]).toAbsolutePath().normalize();
        if (!outputPath.getParent().toRealPath().startsWith(root)) throw new IllegalArgumentException("Output escapes analysis directory");
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(outputPath, StandardOpenOption.CREATE_NEW))) {
            for (int i = 1; i < args.length; i++) {
                Function function = getFunctionAt(toAddr(args[i]));
                if (function == null) { output.println("NO_FUNCTION " + args[i]); continue; }
                output.println("FUNCTION " + function.getName(true) + " ENTRY " + function.getEntryPoint());
                for (Reference reference : getReferencesTo(function.getEntryPoint())) {
                    Function caller = getFunctionContaining(reference.getFromAddress());
                    output.println("CALLER " + reference.getFromAddress() + " " +
                        (caller == null ? "DATA" : caller.getName(true) + " " + caller.getEntryPoint()));
                }
                DecompileResults result = decompiler.decompileFunction(function, 60, monitor);
                if (result.decompileCompleted()) output.println(result.getDecompiledFunction().getC());
                else output.println("DECOMPILE_FAILED " + result.getErrorMessage());
                for (Instruction instruction : currentProgram.getListing().getInstructions(function.getBody(), true)) {
                    output.println(instruction.getAddress() + " " + instruction);
                }
            }
        } finally { decompiler.dispose(); }
    }
}
