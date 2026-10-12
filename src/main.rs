use std::{env, path::PathBuf, process::ExitCode};
use studio_2016::{bundle::Bundle, geometry::RenderScene, runtime};

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "Usage: studio-2016 <audit-runtime|audit-geometry> [RobloxStudio directory]\nRead-only exact-version import or recovered static geometry diagnostic.\nEvidence logs stay in RobloxStudio/analysis."
        );
        return ExitCode::SUCCESS;
    }
    if args.is_empty()
        || (args[0] != "audit-runtime" && args[0] != "audit-geometry")
        || args.len() > 2
    {
        eprintln!("Usage: studio-2016 <audit-runtime|audit-geometry> [RobloxStudio directory]");
        return ExitCode::from(2);
    }
    let root = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("RobloxStudio"));
    let result = (|| {
        let bundle = Bundle::open(&root)?;
        let result = if args[0] == "audit-geometry" {
            RenderScene::character(&bundle).map(|scene| scene.report)
        } else {
            runtime::audit(&bundle, 120)
        };
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                let path = runtime::save_log(&bundle, &format!("runtime_audit_failed: {error}\n"))?;
                return Err(format!("{error}; evidence_log={}", path.display()));
            }
        };
        let path = runtime::save_log(&bundle, &report)?;
        print!("{report}");
        println!("evidence_log={}", path.display());
        Ok::<_, String>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("runtime_audit_failed: {error}");
            ExitCode::FAILURE
        }
    }
}
