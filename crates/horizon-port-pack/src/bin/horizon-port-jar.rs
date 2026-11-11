//! `horizon-port-jar`: Native Rust `.jar` -> `.hmod` mod converter and `.hmod`
//! package manager for BlockWay Rust / Horizon Engine.
//!
//! Commands:
//! - `convert --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--resources <26.3>] [--enable] [--json] [--jar <file.jar>...] [<file.jar>...]`
//! - `scan-dir --mods-dir <mods> --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--resources <26.3>] [--enable] [--json]`
//! - `list --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--json]`
//! - `toggle --mods-txt <horizon-mods.txt> [--out-dir <horizon-mods>] --id <mod_id> (--enable | --disable) [--json]`
use horizon_port_pack::{
    check::Verdict,
    port_jar::{PortedMod, list_installed_hmods, port_jar_to_hmod, set_mod_enabled},
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const VERSION: &str = "horizon-port-jar 1.0.0 (blockway-rust)";
const USAGE: &str = "\
usage:
  horizon-port-jar convert --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--resources <prepared-26.3>] [--enable] [--json] [--jar <file.jar>...] [<file.jar>...]
  horizon-port-jar scan-dir --mods-dir <java-mods> --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--resources <prepared-26.3>] [--enable] [--json]
  horizon-port-jar list --out-dir <horizon-mods> [--mods-txt <horizon-mods.txt>] [--json]
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
    results: Vec<PortedMod>,
}

#[derive(Serialize)]
struct ListOutput {
    ok: bool,
    engine: &'static str,
    mode: &'static str,
    mods: Vec<horizon_port_pack::port_jar::InstalledHmod>,
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
    let mods_dir = take_option(&mut args, "--mods-dir")?.map(PathBuf::from);
    let mods_txt = take_option(&mut args, "--mods-txt")?.map(PathBuf::from);
    let resources = take_option(&mut args, "--resources")?.map(PathBuf::from);
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
            let out_dir = out_dir.ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            for positional in args.into_iter().skip(1) {
                if positional.starts_with('-') {
                    return Err(format!("unknown flag {positional}\n{USAGE}"));
                }
                explicit_jars.push(PathBuf::from(positional));
            }
            if explicit_jars.is_empty() {
                return Err("no .jar files specified for convert".into());
            }
            run_batch_convert(
                "convert",
                &explicit_jars,
                &out_dir,
                mods_txt.as_deref(),
                resources.as_deref(),
                enable,
                json_mode,
            )
        }
        "scan-dir" => {
            let mods_dir = mods_dir.ok_or_else(|| "missing --mods-dir <java-mods>".to_owned())?;
            let out_dir = out_dir.ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            let mut jars = Vec::new();
            if mods_dir.is_dir() {
                let entries = fs::read_dir(&mods_dir)
                    .map_err(|e| format!("{}: {e}", mods_dir.display()))?;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
                    {
                        jars.push(path);
                    }
                }
                jars.sort();
            }
            run_batch_convert(
                "scan-dir",
                &jars,
                &out_dir,
                mods_txt.as_deref(),
                resources.as_deref(),
                enable,
                json_mode,
            )
        }
        "list" => {
            let out_dir = out_dir.ok_or_else(|| "missing --out-dir <horizon-mods>".to_owned())?;
            let mods = list_installed_hmods(&out_dir, mods_txt.as_deref())?;
            if json_mode {
                let payload = ListOutput {
                    ok: true,
                    engine: "blockway-rust",
                    mode: "list",
                    mods,
                };
                println!(
                    "{}",
                    serde_json::to_string(&payload).map_err(|e| e.to_string())?
                );
            } else if mods.is_empty() {
                println!("No .hmod packages installed in {}", out_dir.display());
            } else {
                for m in &mods {
                    let state = if m.enabled { "ON " } else { "OFF" };
                    println!(
                        "[{state}] {} ({} {}) — {} blocks, {} items, {} files",
                        m.id, m.name, m.version, m.blocks, m.items, m.files
                    );
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

fn run_batch_convert(
    mode: &'static str,
    jars: &[PathBuf],
    out_dir: &Path,
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

    for jar_path in jars {
        let mut item = port_jar_to_hmod(jar_path, out_dir, resources);
        match item.verdict {
            Verdict::Works => works += 1,
            Verdict::Partly => partly += 1,
            Verdict::No => skipped_code_only += 1,
        }
        if item.hmod_path.is_some()
            && auto_enable
            && let Some(txt) = mods_txt
        {
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
            results,
        };
        println!(
            "{}",
            serde_json::to_string(&payload).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "Processed {} jar(s): {} ported to .hmod ({} full, {} partial), {} code-only skipped.",
            results.len(),
            ported,
            works,
            partly,
            skipped_code_only
        );
        for r in &results {
            println!("  [{}] {} -> {}", r.verdict_label, r.file_name, r.summary);
            if let Some(p) = &r.hmod_path {
                println!("    .hmod: {p} (enabled={})", r.enabled);
            }
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
