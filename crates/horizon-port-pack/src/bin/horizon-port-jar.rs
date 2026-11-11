//! `horizon-port-jar`: Multi-channel Native Rust Mod & Modpack converter and
//! manager for BlockWay Rust / Horizon Engine.
//!
//! Commands:
//! - `convert --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--resources <26.3>] [--enable] [--json] [--jar <file>...] [<file>...]`
//! - `scan-dir --mods-dir <mods> --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--resources <26.3>] [--enable] [--json]`
//! - `port-pack --pack <dir_or_mrpack_or_zip> --game-dir <game> [--resources <26.3>] [--enable] [--json]`
//! - `list --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--json]`
//! - `toggle --mods-txt <horizon-mods.txt> [--out-dir <horizon-mods>] --id <mod_id> (--enable | --disable) [--json]`
use horizon_port_pack::{
    check::Verdict,
    port_jar::{
        InstalledDatapack, InstalledHmod, InstalledResourcePack, ModpackPortReport,
        NativeModStatus, PortedMod, list_installed_datapacks, list_installed_hmods,
        list_native_mods, list_ported_resourcepacks, port_jar_with_game_dir, port_modpack,
        set_mod_enabled,
    },
};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const VERSION: &str = "horizon-port-jar 2.0.0 (blockway-rust)";
const USAGE: &str = "\
usage:
  horizon-port-jar convert --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--resources <prepared-26.3>] [--enable] [--json] [--jar <file.jar|zip>...] [<file.jar|zip>...]
  horizon-port-jar scan-dir --mods-dir <java-mods> --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--resources <prepared-26.3>] [--enable] [--json]
  horizon-port-jar port-pack --pack <dir_or_mrpack_or_zip> --game-dir <game> [--resources <prepared-26.3>] [--enable] [--json]
  horizon-port-jar list --out-dir <horizon-mods> [--game-dir <game>] [--mods-txt <horizon-mods.txt>] [--json]
  horizon-port-jar toggle --mods-txt <horizon-mods.txt> [--out-dir <horizon-mods>] --id <mod_id> (--enable | --disable) [--json]";

#[derive(Serialize)]
struct ConvertOutput {
    ok: bool,
    engine: &'static str,
    mode: &'static str,
    total: usize,
    ported: usize,
    works: usize,
    partly: usize,
    skipped_code_only: usize,
    hmods_created: usize,
    datapacks_created: usize,
    resourcepacks_created: usize,
    native_rust_enabled: usize,
    builtin_engine_matched: usize,
    results: Vec<PortedMod>,
}

#[derive(Serialize)]
struct ListOutput {
    ok: bool,
    engine: &'static str,
    mode: &'static str,
    mods: Vec<InstalledHmod>,
    native_mods: Vec<NativeModStatus>,
    datapacks: Vec<InstalledDatapack>,
    resourcepacks: Vec<InstalledResourcePack>,
}

#[derive(Serialize)]
struct ToggleOutput {
    ok: bool,
    engine: &'static str,
    mode: &'static str,
    id: String,
    enabled: bool,
    clash: Option<String>,
}

#[derive(Serialize)]
struct PortPackOutput {
    ok: bool,
    engine: &'static str,
    mode: &'static str,
    #[serde(flatten)]
    report: ModpackPortReport,
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{VERSION}");
        return ExitCode::SUCCESS;
    }

    let json_mode = take_flag(&mut args, "--json");
    let result = run(args, json_mode);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if json_mode {
                let payload = serde_json::json!({
                    "ok": false,
                    "engine": "blockway-rust",
                    "error": err,
                });
                println!("{payload}");
            } else {
                eprintln!("error: {err}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: Vec<String>, json_mode: bool) -> Result<(), String> {
    let enable = take_flag(&mut args, "--enable");
    let disable = take_flag(&mut args, "--disable");
    let out_dir = take_option(&mut args, "--out-dir")?.map(PathBuf::from);
    let game_dir = take_option(&mut args, "--game-dir")?.map(PathBuf::from);
    let mods_dir = take_option(&mut args, "--mods-dir")?.map(PathBuf::from);
    let mods_txt = take_option(&mut args, "--mods-txt")?.map(PathBuf::from);
    let resources = take_option(&mut args, "--resources")?.map(PathBuf::from);
    let pack_path = take_option(&mut args, "--pack")?.map(PathBuf::from);
    let mod_id = take_option(&mut args, "--id")?;
    let mut explicit_jars: Vec<PathBuf> = Vec::new();
    while let Some(j) = take_option(&mut args, "--jar")? {
        explicit_jars.push(PathBuf::from(j));
    }

    let Some(cmd) = args.first().cloned() else {
        return Err(USAGE.to_owned());
    };

    match cmd.as_str() {
        "convert" => {
            let out_dir = out_dir
                .or_else(|| game_dir.as_ref().map(|g| g.join("horizon-mods")))
                .ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            let resolved_game_dir = game_dir.or_else(|| out_dir.parent().map(Path::to_path_buf));
            let resolved_mods_txt = mods_txt.or_else(|| {
                resolved_game_dir
                    .as_ref()
                    .map(|g| g.join("horizon-mods.txt"))
            });
            for positional in args.into_iter().skip(1) {
                if positional.starts_with('-') {
                    return Err(format!("unknown flag {positional}\n{USAGE}"));
                }
                explicit_jars.push(PathBuf::from(positional));
            }
            if explicit_jars.is_empty() {
                return Err("no .jar/.zip files specified for convert".into());
            }
            run_batch_convert(
                "convert",
                &explicit_jars,
                &out_dir,
                resolved_game_dir.as_deref(),
                resolved_mods_txt.as_deref(),
                resources.as_deref(),
                enable,
                json_mode,
            )
        }
        "scan-dir" => {
            let mods_dir = mods_dir.ok_or_else(|| "missing --mods-dir <java-mods>".to_owned())?;
            let out_dir = out_dir
                .or_else(|| game_dir.as_ref().map(|g| g.join("horizon-mods")))
                .ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            let resolved_game_dir = game_dir.or_else(|| out_dir.parent().map(Path::to_path_buf));
            let resolved_mods_txt = mods_txt.or_else(|| {
                resolved_game_dir
                    .as_ref()
                    .map(|g| g.join("horizon-mods.txt"))
            });

            let mut archives = Vec::new();
            let mut seen = BTreeSet::new();
            let mut collect_from = |dir: &Path, allow_zip: bool| -> Result<(), String> {
                if !dir.is_dir() {
                    return Ok(());
                }
                let entries =
                    fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let mut found = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let ok = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| {
                            e.eq_ignore_ascii_case("jar")
                                || (allow_zip && e.eq_ignore_ascii_case("zip"))
                        });
                    if ok && seen.insert(path.clone()) {
                        found.push(path);
                    }
                }
                found.sort();
                archives.extend(found);
                Ok(())
            };

            collect_from(&mods_dir, true)?;
            if let Some(gdir) = &resolved_game_dir {
                collect_from(&gdir.join("datapacks"), true)?;
            }

            run_batch_convert(
                "scan-dir",
                &archives,
                &out_dir,
                resolved_game_dir.as_deref(),
                resolved_mods_txt.as_deref(),
                resources.as_deref(),
                enable,
                json_mode,
            )
        }
        "port-pack" => {
            let pack = pack_path
                .or_else(|| args.get(1).map(PathBuf::from))
                .ok_or_else(|| "missing --pack <dir_or_mrpack_or_zip>".to_owned())?;
            let target_game_dir =
                game_dir.ok_or_else(|| "missing --game-dir <target-game-dir>".to_owned())?;
            let report = port_modpack(&pack, &target_game_dir, resources.as_deref(), enable)?;
            if json_mode {
                let payload = PortPackOutput {
                    ok: true,
                    engine: "blockway-rust",
                    mode: "port-pack",
                    report,
                };
                println!(
                    "{}",
                    serde_json::to_string(&payload).map_err(|e| e.to_string())?
                );
            } else {
                println!(
                    "Ported modpack '{}' ({}) -> {}: {} hmods, {} datapacks, {} resourcepacks, {} native Rust mods, {} built-in engine mods.",
                    report.pack_name,
                    report.pack_format,
                    report.game_dir,
                    report.hmods_created,
                    report.datapacks_created,
                    report.resourcepacks_created,
                    report.native_rust_enabled,
                    report.builtin_engine_matched
                );
            }
            Ok(())
        }
        "list" => {
            let out_dir = out_dir
                .or_else(|| game_dir.as_ref().map(|g| g.join("horizon-mods")))
                .ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            let resolved_game_dir = game_dir.or_else(|| out_dir.parent().map(Path::to_path_buf));
            let resolved_mods_txt = mods_txt.or_else(|| {
                resolved_game_dir
                    .as_ref()
                    .map(|g| g.join("horizon-mods.txt"))
            });
            let mods = list_installed_hmods(&out_dir, resolved_mods_txt.as_deref())?;
            let native_mods = list_native_mods(resolved_mods_txt.as_deref());
            let (datapacks, resourcepacks) = match resolved_game_dir.as_deref() {
                Some(gdir) => (
                    list_installed_datapacks(gdir),
                    list_ported_resourcepacks(gdir),
                ),
                None => (Vec::new(), Vec::new()),
            };
            if json_mode {
                let payload = ListOutput {
                    ok: true,
                    engine: "blockway-rust",
                    mode: "list",
                    mods,
                    native_mods,
                    datapacks,
                    resourcepacks,
                };
                println!(
                    "{}",
                    serde_json::to_string(&payload).map_err(|e| e.to_string())?
                );
            } else {
                println!("Built-in Native Rust Mods:");
                for nm in &native_mods {
                    println!(
                        "  [{}] {} — {}",
                        if nm.enabled { "ON " } else { "OFF" },
                        nm.id,
                        nm.name
                    );
                }
                println!("Installed .hmod Content Packages ({}):", mods.len());
                for m in &mods {
                    let state = if m.enabled { "ON " } else { "OFF" };
                    println!(
                        "  [{state}] {} ({} {}) — {} blocks, {} items, {} files",
                        m.id, m.name, m.version, m.blocks, m.items, m.files
                    );
                }
                if !datapacks.is_empty() {
                    println!("Installed Rust 26.3 Datapacks ({}):", datapacks.len());
                    for dp in &datapacks {
                        println!("  [ON ] {} — {} files", dp.id, dp.files);
                    }
                }
            }
            Ok(())
        }
        "toggle" => {
            let mods_txt =
                mods_txt.ok_or_else(|| "missing --mods-txt <horizon-mods.txt>".to_owned())?;
            let id = mod_id.ok_or_else(|| "missing --id <mod_id>".to_owned())?;
            if enable == disable {
                return Err("pass either --enable or --disable to toggle".into());
            }
            let clash = set_mod_enabled(&mods_txt, out_dir.as_deref(), &id, enable)?;
            let final_enabled = enable && clash.is_none();
            if json_mode {
                let payload = ToggleOutput {
                    ok: clash.is_none(),
                    engine: "blockway-rust",
                    mode: "toggle",
                    id,
                    enabled: final_enabled,
                    clash,
                };
                println!(
                    "{}",
                    serde_json::to_string(&payload).map_err(|e| e.to_string())?
                );
            } else if let Some(reason) = clash {
                println!("Namespace clash: {reason}");
            } else {
                println!("mod.{id}:{}", if final_enabled { "true" } else { "false" });
            }
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_batch_convert(
    mode: &'static str,
    jars: &[PathBuf],
    out_dir: &Path,
    game_dir: Option<&Path>,
    mods_txt: Option<&Path>,
    resources: Option<&Path>,
    auto_enable: bool,
    json_mode: bool,
) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut results = Vec::with_capacity(jars.len());
    let mut works = 0usize;
    let mut partly = 0usize;
    let mut skipped_code_only = 0usize;
    let mut hmods_created = 0usize;
    let mut datapacks_created = 0usize;
    let mut resourcepacks_created = 0usize;
    let mut native_rust_enabled = 0usize;
    let mut builtin_engine_matched = 0usize;

    for jar_path in jars {
        let mut item = port_jar_with_game_dir(jar_path, out_dir, game_dir, resources);
        match item.verdict {
            Verdict::Works => works += 1,
            Verdict::Partly => partly += 1,
            Verdict::No => skipped_code_only += 1,
        }
        match item.kind.as_str() {
            "hmod" => {
                hmods_created += 1;
                if auto_enable && let Some(txt) = mods_txt {
                    match set_mod_enabled(txt, Some(out_dir), &item.mod_id, true) {
                        Ok(None) => item.enabled = true,
                        Ok(Some(clash)) => {
                            item.enabled = false;
                            item.clash_warning = Some(clash);
                        }
                        Err(e) => {
                            item.clash_warning = Some(e);
                        }
                    }
                }
            }
            "native_rust" => {
                native_rust_enabled += 1;
                if auto_enable
                    && let Some(txt) = mods_txt
                    && let Some(nid) = item.native_mod_id.clone()
                    && set_mod_enabled(txt, Some(out_dir), &nid, true).is_ok()
                {
                    item.enabled = true;
                }
            }
            "datapack" => datapacks_created += 1,
            "resourcepack" => resourcepacks_created += 1,
            "builtin_engine" => builtin_engine_matched += 1,
            _ => {}
        }
        results.push(item);
    }

    let ported = works + partly;
    if json_mode {
        let payload = ConvertOutput {
            ok: true,
            engine: "blockway-rust",
            mode,
            total: results.len(),
            ported,
            works,
            partly,
            skipped_code_only,
            hmods_created,
            datapacks_created,
            resourcepacks_created,
            native_rust_enabled,
            builtin_engine_matched,
            results,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "Processed {} file(s): {} ready on Rust ({} hmod, {} datapack, {} resourcepack, {} native Rust, {} built-in engine), {} deep Java code skipped.",
            results.len(),
            ported,
            hmods_created,
            datapacks_created,
            resourcepacks_created,
            native_rust_enabled,
            builtin_engine_matched,
            skipped_code_only
        );
        for r in &results {
            println!(
                "  [{} | {}] {} -> {}",
                r.verdict_label, r.kind, r.file_name, r.summary
            );
        }
    }
    Ok(())
}

fn take_option(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    match args.iter().position(|a| a == flag) {
        Some(i) if i + 1 < args.len() => {
            let val = args.remove(i + 1);
            args.remove(i);
            Ok(Some(val))
        }
        Some(_) => Err(format!("missing value after {flag}")),
        None => Ok(None),
    }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}
