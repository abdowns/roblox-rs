use std::{env, path::PathBuf, process::ExitCode};
use studio_2016::collision;
use studio_2016::{
    animation, assembly, bundle::Bundle, geometry::RenderScene, kernel, legacy_contact, locomotion,
    physics, player, render, rig, runtime, walking,
};

fn main() -> ExitCode {
    let mut args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() {
        args.push("viewport".into());
    }
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "Usage: studio-2016 [audit-runtime|audit-geometry|audit-render|audit-player|audit-physics|audit-motion|audit-rig|audit-animation|audit-animation-render|audit-locomotion|audit-collision|audit-assembly|audit-kernel|audit-legacy-contact|audit-scheduler|audit-camera|audit-lighting|audit-sky|audit-walking|audit-walking-render|viewport-walking|viewport-walking-smoke|viewport-animation-smoke|viewport|viewport-smoke|viewport-motion-smoke] [RobloxStudio directory]\nDefault: open the rendered character viewport. Escape closes it.\nRead-only exact-version import and recovered core-client diagnostics.\nEvidence logs stay in RobloxStudio/analysis."
        );
        return ExitCode::SUCCESS;
    }
    if args.is_empty()
        || !matches!(
            args[0].to_str(),
            Some(
                "audit-runtime"
                    | "audit-kernel"
                    | "audit-legacy-contact"
                    | "audit-scheduler"
                    | "audit-camera"
                    | "audit-lighting"
                    | "audit-sky"
                    | "audit-walking"
                    | "audit-walking-render"
                    | "viewport-walking"
                    | "viewport-walking-smoke"
                    | "audit-assembly"
                    | "audit-geometry"
                    | "audit-render"
                    | "audit-player"
                    | "audit-physics"
                    | "audit-motion"
                    | "audit-rig"
                    | "audit-animation"
                    | "audit-animation-render"
                    | "audit-locomotion"
                    | "audit-collision"
                    | "viewport-animation-smoke"
                    | "viewport"
                    | "viewport-smoke"
                    | "viewport-motion-smoke"
            )
        )
        || args.len() > 2
    {
        eprintln!(
            "Usage: studio-2016 <audit-runtime|audit-geometry|audit-render|audit-player|audit-physics|audit-motion|audit-rig|audit-animation|audit-animation-render|audit-locomotion|audit-collision|audit-assembly|audit-kernel|audit-legacy-contact|audit-scheduler|audit-camera|audit-lighting|audit-sky|audit-walking|audit-walking-render|viewport-walking|viewport-walking-smoke|viewport-animation-smoke|viewport|viewport-smoke|viewport-motion-smoke> [RobloxStudio directory]"
        );
        return ExitCode::from(2);
    }
    let root = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("RobloxStudio"));
    let result = (|| {
        let bundle = Bundle::open(&root)?;
        let result = match args[0].to_str().unwrap() {
            "audit-geometry" => RenderScene::character(&bundle).map(|scene| scene.report),
            "audit-render" => render::audit(&bundle),
            "audit-player" => player::audit(&bundle),
            "audit-physics" => physics::audit(&bundle),
            "audit-motion" => render::audit_motion(&bundle),
            "audit-rig" => rig::audit(&bundle),
            "audit-animation" => animation::audit(&bundle),
            "audit-animation-render" => render::audit_animation(&bundle),
            "audit-locomotion" => locomotion::audit(&bundle),
            "audit-collision" => collision::audit(&bundle),
            "audit-assembly" => assembly::audit(&bundle),
            "audit-kernel" => kernel::audit(&bundle),
            "audit-legacy-contact" => legacy_contact::audit(&bundle),
            "audit-scheduler" => studio_2016::scheduler::audit(&bundle),
            "audit-camera" => studio_2016::camera_script::audit(&bundle),
            "audit-lighting" => render::audit_lighting(&bundle),
            "audit-sky" => render::audit_sky(&bundle),
            "audit-walking" => walking::audit(&bundle),
            "audit-walking-render" => render::audit_walking(&bundle),
            "viewport-walking" => render::viewport_walking(&bundle, None),
            "viewport-walking-smoke" => render::viewport_walking(&bundle, Some(120)),
            "viewport-animation-smoke" => render::viewport_animation_smoke(&bundle),
            "viewport" => render::viewport(&bundle, None),
            "viewport-smoke" => render::viewport(&bundle, Some(120)),
            "viewport-motion-smoke" => render::viewport_motion_smoke(&bundle),
            _ => runtime::audit(&bundle, 120),
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
