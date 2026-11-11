//! Multi-channel Java Mod & Modpack -> Native Rust converter for BlockWay Rust / Horizon Engine
//! (`1.16.5` through `26.3`, NeoForge / Forge / Fabric / Quilt / Datapack / Modpack).
//!
//! Supports 5 real native Rust conversion channels:
//! 1. `native_rust`    — Maps client HUD/utility mods (Xaero's Minimap, JourneyMap, Zoomify,
//!                       Logical Zoom, MiniHUD, Keystrokes, BetterF3) to Horizon's compiled-in
//!                       `mod_api::Hooks` Rust modules (`minimap`, `zoom`, `coords_hud`,
//!                       `keystrokes`, `fps_overlay`).
//! 2. `builtin_engine` — Recognizes optimization, shader, connected-texture & loader libraries
//!                       already built into Horizon's Vulkan/DX12 Rust engine (Sodium, Lithium,
//!                       Iris, FerriteCore, EntityCulling, Continuity, EMF/ETF, Fabric API, etc.).
//! 3. `hmod`           — Carries custom blocks, items, 3D models, textures, recipes (`1.16.5`–`26.3`)
//!                       and auto-generated Creative Tabs into validated `<mod_id>.hmod` archives.
//! 4. `datapack`       — Extracts & upgrades worldgen, biome, structure, dimension, SkyBlock /
//!                       OneBlock & function mods/zips (including `META-INF/jars/*.jar` wrapper
//!                       jars like Terralith/Incendium/Tectonic) to `26.3` format inside
//!                       `<game_dir>/new-world-datapacks/<mod_id>/`.
//! 5. `resourcepack`   — Packs vanilla asset/texture/model/CEM/CIT overrides into
//!                       `<game_dir>/resourcepacks/<mod_id>-rust.zip`.
use crate::{carry, check::Verdict, classes, fetch, import, jar, pipeline, upgrade};
use horizon_mods::{
    Capability, Content, Manifest, Package,
    content::clash,
    find,
    manifest::MANIFEST,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static WORK_COUNTER: AtomicU64 = AtomicU64::new(0);

const RESERVED_IDS: [&str; 9] = [
    "minecraft",
    "c",
    "horizon",
    "forge",
    "neoforge",
    "fabric",
    "quilt",
    "java",
    "realms",
];

pub const NATIVE_MOD_SPECS: [(&str, &str, &str); 5] = [
    (
        "minimap",
        "Rust Minimap",
        "Live terrain minimap with compass, zoom & XYZ coordinates (replaces Xaero's Minimap / JourneyMap / VoxelMap).",
    ),
    (
        "zoom",
        "Rust Smooth Zoom",
        "Smooth camera zoom on key hold + scroll-wheel magnification (replaces Zoomify / Logical Zoom / OK Zoomer).",
    ),
    (
        "fps_overlay",
        "Rust FPS Counter",
        "Lightweight live FPS & 1s min-FPS HUD overlay (replaces FPSDisplay / BetterF3 FPS).",
    ),
    (
        "coords_hud",
        "Rust Coords & Biome HUD",
        "Live XYZ coordinates, cardinal direction & biome HUD (replaces MiniHUD / Coordinates Display).",
    ),
    (
        "keystrokes",
        "Rust Keystrokes & CPS",
        "Live WASD, Space & LMB/RMB CPS PvP overlay (replaces Keystrokes / KronHUD).",
    ),
];

pub fn is_native_mod_id(id: &str) -> bool {
    NATIVE_MOD_SPECS.iter().any(|(m, _, _)| *m == id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortedMod {
    pub jar_path: String,
    pub file_name: String,
    pub mod_id: String,
    pub original_mod_id: String,
    pub name: String,
    pub version: String,
    pub loader: String,
    pub license: String,
    pub legacy_mc_layout: bool,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub native_mod_id: Option<String>,
    pub verdict: Verdict,
    pub verdict_label: String,
    pub summary: String,
    pub hmod_path: Option<String>,
    pub hmod_size_bytes: u64,
    pub enabled: bool,
    pub clash_warning: Option<String>,
    pub blocks: usize,
    pub items: usize,
    pub recipes: usize,
    pub loot_tables: usize,
    pub worldgen: usize,
    pub structures: usize,
    pub textures: usize,
    pub creative_tabs: usize,
    pub files_packed: usize,
    pub namespaces: Vec<String>,
    pub ports: Vec<String>,
    pub skipped: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledHmod {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub api: String,
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub enabled: bool,
    pub blocks: usize,
    pub items: usize,
    pub creative_tabs: usize,
    pub effects: usize,
    pub fluids: usize,
    pub entity_types: usize,
    pub files: usize,
    pub namespaces: Vec<String>,
    pub clash: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeModStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledDatapack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledResourcePack {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackPortReport {
    pub pack_name: String,
    pub pack_format: String,
    pub minecraft: String,
    pub loader: String,
    pub game_dir: String,
    pub total_mods: usize,
    pub works: usize,
    pub partly: usize,
    pub skipped_code_only: usize,
    pub hmods_created: usize,
    pub datapacks_created: usize,
    pub resourcepacks_created: usize,
    pub native_rust_enabled: usize,
    pub builtin_engine_matched: usize,
    pub overrides_extracted: usize,
    pub resourcepacks_copied: usize,
    pub shaderpacks_copied: usize,
    pub downloads_fetched: usize,
    pub download_warnings: Vec<String>,
    pub results: Vec<PortedMod>,
}

/// Normalises an arbitrary Java mod id or filename stem into Horizon's
/// `[a-z0-9_]{1,64}` manifest id rule.
pub fn sanitize_mod_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(64));
    let mut prev_underscore = false;
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            out.push(lower);
            prev_underscore = false;
        } else if !out.is_empty() && !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("ported_mod");
    }
    if out.as_bytes()[0].is_ascii_digit() || RESERVED_IDS.contains(&out.as_str()) {
        out = format!("mod_{out}");
    }
    if out.len() > 64 {
        out.truncate(64);
        while out.ends_with('_') {
            out.pop();
        }
        if out.is_empty() {
            out.push_str("ported_mod");
        }
    }
    out
}

fn normalize_token(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    let mut prev_u = false;
    for ch in raw.chars() {
        let l = ch.to_ascii_lowercase();
        if l.is_ascii_lowercase() || l.is_ascii_digit() {
            s.push(l);
            prev_u = false;
        } else if !s.is_empty() && !prev_u {
            s.push('_');
            prev_u = true;
        }
    }
    while s.ends_with('_') {
        s.pop();
    }
    s
}

/// Strip version suffixes from jar filenames (e.g. `Xaeros_Minimap_24.6.1_Fabric_1.21` -> `xaeros_minimap`).
fn clean_stem_token(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let norm = normalize_token(stem);
    let mut parts = Vec::new();
    for part in norm.split('_') {
        if part.is_empty() {
            continue;
        }
        if matches!(
            part,
            "fabric" | "forge" | "neoforge" | "quilt" | "mc" | "mod" | "jar" | "v1" | "v2"
        ) {
            break;
        }
        if part.bytes().next().is_some_and(|b| b.is_ascii_digit()) {
            break;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        norm
    } else {
        parts.join("_")
    }
}

fn detect_native_rust_equivalent(raw_id: &str, file_name: &str) -> Option<(&'static str, &'static str)> {
    let id = normalize_token(raw_id);
    let stem = clean_stem_token(file_name);
    let matches_any = |needles: &[&str]| {
        needles
            .iter()
            .any(|n| id == *n || stem == *n || id.starts_with(&format!("{n}_")) || stem.starts_with(&format!("{n}_")))
    };

    if matches_any(&[
        "xaerominimap",
        "xaeros_minimap",
        "xaero_minimap",
        "xaerominimapfair",
        "xaeros_minimap_fair",
        "xaeroworldmap",
        "xaeros_world_map",
        "xaero_world_map",
        "journeymap",
        "voxelmap",
        " antique_atlas",
        "antiqueatlas",
        "mapfrontiers",
    ]) || (id.contains("xaero") && (id.contains("map") || stem.contains("map")))
    {
        return Some(("minimap", "Rust Minimap"));
    }

    if matches_any(&[
        "zoomify",
        "logical_zoom",
        "logicalzoom",
        "ok_zoomer",
        "okzoomer",
        "wi_zoom",
        "wizoom",
        "zume",
        "justzoom",
        "just_zoom",
    ]) {
        return Some(("zoom", "Rust Smooth Zoom"));
    }

    if matches_any(&[
        "minihud",
        "coordinates_display",
        "coordinatesdisplay",
        "batty_coordinates",
        "hudcompass",
    ]) {
        return Some(("coords_hud", "Rust Coords & Biome HUD"));
    }

    if matches_any(&[
        "keystrokes",
        "keystrokesmod",
        "clean_keystrokes",
        "kronhud",
        "evergreenhud",
    ]) {
        return Some(("keystrokes", "Rust Keystrokes & CPS"));
    }

    if matches_any(&[
        "fpsdisplay",
        "fps_display",
        "betterf3",
        "better_fps_hud",
    ]) {
        return Some(("fps_overlay", "Rust FPS Counter"));
    }

    None
}

fn detect_builtin_engine_feature(raw_id: &str, file_name: &str) -> Option<&'static str> {
    let id = normalize_token(raw_id);
    let stem = clean_stem_token(file_name);
    let matches_any = |needles: &[&str]| {
        needles
            .iter()
            .any(|n| id == *n || stem == *n || id.starts_with(&format!("{n}_")) || stem.starts_with(&format!("{n}_")))
    };

    if matches_any(&[
        "sodium",
        "sodium_extra",
        "reeses_sodium_options",
        "embeddium",
        "rubidium",
        "indium",
        "lithium",
        "radium",
        "canary",
        "ferritecore",
        "modernfix",
        "immediatelyfast",
        "entityculling",
        "moreculling",
        "badoptimizations",
        "krypton",
        "c2me",
        "noisium",
        "scalablelux",
        "starlight",
        "phosphor",
        "lazydfu",
        "dynamic_fps",
        "dynamicfps",
        "fastload",
        "fastquit",
        "memoryleakfix",
        "gpumemleakfix",
        "exordium",
        "threadtweak",
        "alternate_current",
        "clumps",
    ]) {
        return Some("Multi-threaded Vulkan / DirectX 12 Rust renderer & tick optimizer");
    }

    if matches_any(&[
        "iris",
        "oculus",
        "optifine",
        "optifabric",
        "continuity",
        "fusion",
        "polytone",
        "entity_model_features",
        "emf",
        "entity_texture_features",
        "etf",
        "citresewn",
        "animatica",
        "lambdynamiclights",
        "ryoamiclights",
    ]) {
        return Some("Native Rust Shader Pipeline + OptiFine CEM/CIT/CTM & Dynamic Lights");
    }

    if matches_any(&[
        "fabric_api",
        "fabric",
        "fabric_language_kotlin",
        "architectury",
        "cloth_config",
        "cloth_config_api",
        "yet_another_config_lib",
        "yacl",
        "modmenu",
        "forgeconfigapiport",
        "malilib",
        "collective",
        "balm",
        "puzzleslib",
        "bookshelf",
        "iceberg",
        "cristellib",
        "cristel_lib",
        "lithostitched",
        "terrablender",
        "placeholder_api",
        "e4mc",
        "no_chat_reports",
        "nochatreports",
    ]) {
        return Some("Native Horizon Mod API, Worldgen Pipeline & Friends Tunnel");
    }

    None
}

fn is_valid_mod_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && !RESERVED_IDS.contains(&s)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn resolve_mod_id(raw_id: &str, names: &[String]) -> String {
    let sanitized = sanitize_mod_id(raw_id);
    let asset_prefix = format!("assets/{sanitized}/");
    let data_prefix = format!("data/{sanitized}/");
    if names
        .iter()
        .any(|n| n.starts_with(&asset_prefix) || n.starts_with(&data_prefix))
    {
        return sanitized;
    }

    let mut primary_candidates = BTreeSet::new();
    let mut all_candidates = BTreeSet::new();
    for name in names {
        let mut parts = name.splitn(3, '/');
        let (Some(root), Some(ns), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if (root != "assets" && root != "data") || rest.is_empty() || name.ends_with('/') {
            continue;
        }
        if !is_valid_mod_id(ns) {
            continue;
        }
        if root == "data" && rest.starts_with("tags/") {
            continue;
        }
        all_candidates.insert(ns.to_owned());
        if (root == "assets"
            && (rest.starts_with("blockstates/") || rest.starts_with("models/item/")))
            || (root == "data"
                && (rest.starts_with("worldgen/")
                    || rest.starts_with("recipe/")
                    || rest.starts_with("recipes/")))
        {
            primary_candidates.insert(ns.to_owned());
        }
    }
    if primary_candidates.len() == 1 {
        return primary_candidates.into_iter().next().unwrap();
    }
    if all_candidates.len() == 1 {
        return all_candidates.into_iter().next().unwrap();
    }
    sanitized
}

fn detect_loader(names: &[String]) -> &'static str {
    let has = |target: &str| names.iter().any(|x| x == target);
    if has("META-INF/neoforge.mods.toml") {
        "neoforge"
    } else if has("META-INF/mods.toml") || has("mcmod.info") {
        "forge"
    } else if has("fabric.mod.json") {
        "fabric"
    } else if has("quilt.mod.json") {
        "quilt"
    } else if has("pack.mcmeta") {
        "datapack"
    } else {
        ""
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn vanilla_blocks_from_resources(resources: Option<&Path>) -> Option<BTreeSet<String>> {
    let root = resources?;
    let facts: Value = pipeline::read_json(&root.join("facts.json")).ok()?;
    let defaults = facts.get("defaults")?.as_object()?;
    Some(defaults.keys().cloned().collect())
}

fn upgrade_recipe_json_in_place(recipe: &mut Value) -> bool {
    let kind = recipe["type"].as_str().unwrap_or("").to_owned();
    if !kind.starts_with("minecraft:") {
        return false;
    }
    let mut changed = false;
    if let Some(obj) = recipe.as_object_mut() {
        for cond_key in [
            "neoforge:conditions",
            "forge:conditions",
            "fabric:load_conditions",
        ] {
            if obj.remove(cond_key).is_some() {
                changed = true;
            }
        }
    }
    if kind == "minecraft:stonecutting"
        && let Some(result_id) = recipe["result"].as_str().map(str::to_owned)
        && let Some(obj) = recipe.as_object_mut()
    {
        let count = obj
            .remove("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 99);
        obj.insert(
            "result".into(),
            if count == 1 {
                json!({ "id": result_id })
            } else {
                json!({ "id": result_id, "count": count })
            },
        );
        changed = true;
    }
    if let Some(res_obj) = recipe.get("result").and_then(Value::as_object)
        && res_obj.get("id").is_none()
        && let Some(item_id) = res_obj.get("item").and_then(Value::as_str)
    {
        let item_id = item_id.to_owned();
        let count = res_obj
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .clamp(1, 99);
        recipe["result"] = if count == 1 {
            json!({ "id": item_id })
        } else {
            json!({ "id": item_id, "count": count })
        };
        changed = true;
    }
    changed |= carry::renamed_ids(recipe);
    let _ = carry::upgrade_recipe(recipe);
    changed
}

fn upgrade_legacy_carried_recipes(
    carry_dir: &Path,
    owned_namespaces: &[String],
    kept: &mut Vec<String>,
) -> usize {
    let mut newly_upgraded = 0usize;
    let mut upgraded_rel_paths = BTreeSet::new();

    for ns in owned_namespaces {
        let recipe_root = carry_dir.join("data").join(ns).join("recipe");
        if !recipe_root.is_dir() {
            continue;
        }
        let mut stack = vec![recipe_root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(rel) = path.strip_prefix(&recipe_root) else {
                    continue;
                };
                let rel_key = rel.to_string_lossy().replace('\\', "/");
                let was_in_kept = kept.iter().any(|k| k == &rel_key);
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                let Ok(mut recipe) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                let kind = recipe["type"].as_str().unwrap_or("");
                if !kind.starts_with("minecraft:") {
                    continue;
                }
                let changed = upgrade_recipe_json_in_place(&mut recipe);
                let ingredients_ok = if was_in_kept {
                    carry::upgrade_recipe(&mut recipe)
                } else {
                    true
                };
                if (changed || was_in_kept)
                    && ingredients_ok
                    && let Ok(pretty) = serde_json::to_vec_pretty(&recipe)
                    && fs::write(&path, pretty).is_ok()
                    && was_in_kept
                {
                    newly_upgraded += 1;
                    upgraded_rel_paths.insert(rel_key);
                }
            }
        }
    }

    if !upgraded_rel_paths.is_empty() {
        kept.retain(|k| !upgraded_rel_paths.contains(k));
    }
    newly_upgraded
}

fn ensure_creative_tab(carry_dir: &Path, mod_id: &str, display_name: &str) -> Result<bool, String> {
    let horizon_dir = carry_dir.join("data").join(mod_id).join("horizon");
    let tab_dir = horizon_dir.join("creative_tab");
    if tab_dir.is_dir()
        && fs::read_dir(&tab_dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
    {
        return Ok(false);
    }

    let mut entries = BTreeSet::new();
    for kind in ["block", "item"] {
        let dir = horizon_dir.join(kind);
        for f in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = f.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let has_item = kind == "item"
                || fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .is_none_or(|v| v.get("item") != Some(&json!(false)));
            if has_item {
                entries.insert(format!("{mod_id}:{stem}"));
            }
        }
    }
    let Some(icon) = entries.iter().next().cloned() else {
        return Ok(false);
    };
    let items: Vec<String> = entries.into_iter().collect();
    let title = if display_name.trim().is_empty() {
        mod_id
    } else {
        display_name.trim()
    };
    let payload = json!({
        "title": title,
        "icon": icon,
        "items": items,
    });
    fs::create_dir_all(&tab_dir).map_err(|e| format!("{}: {e}", tab_dir.display()))?;
    let text = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
    let target = tab_dir.join("main.json");
    fs::write(&target, text).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(true)
}

fn temp_work_dir(mod_id: &str) -> PathBuf {
    let seq = WORK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "horizon-port-jar-{}-{mod_id}-{seq}-{nanos}",
        std::process::id()
    ))
}

fn safe_rel_path(raw: &str) -> Option<PathBuf> {
    let norm = raw.replace('\\', "/");
    let p = Path::new(&norm);
    let mut clean = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg.to_str()?;
                if s.is_empty() || s.contains(':') || s.contains('\0') {
                    return None;
                }
                clean.push(s);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!clean.as_os_str().is_empty()).then_some(clean)
}

fn pack_carried_dir_to_hmod(carry_dir: &Path, out_dir: &Path) -> Result<(PathBuf, Content), String> {
    let manifest_path = carry_dir.join(MANIFEST);
    let manifest_text =
        fs::read_to_string(&manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::parse(&manifest_text)?;
    if !manifest.capabilities.contains(&Capability::Registries) {
        return Err(format!(
            "{MANIFEST}: content mod requires capability registries"
        ));
    }
    let content = Content::read(carry_dir, &manifest)?;
    let mut reader = content.reader()?;
    let mut loaded_files: Vec<(String, Vec<u8>)> = Vec::with_capacity(content.files.len());
    for name in &content.files {
        loaded_files.push((name.clone(), reader.read(name)?));
    }
    let borrowed: Vec<(&str, &[u8])> = loaded_files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let archive_bytes = Package::write_files(&manifest_text, None, &borrowed)?;

    fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let final_path = out_dir.join(format!("{}.hmod", manifest.id));
    let partial_path = out_dir.join(format!("{}.hmod.partial", manifest.id));
    fs::write(&partial_path, &archive_bytes)
        .map_err(|e| format!("{}: {e}", partial_path.display()))?;
    if final_path.exists() {
        let _ = fs::remove_file(&final_path);
    }
    fs::rename(&partial_path, &final_path)
        .map_err(|e| format!("{}: {e}", final_path.display()))?;

    let verified_pkg = Package::read(&final_path)?;
    let verified_content = Content::read(&final_path, &verified_pkg.manifest)?;
    Ok((final_path, verified_content))
}

#[derive(Default)]
struct ArchiveDataSummary {
    data_files: usize,
    worldgen_files: usize,
    structure_files: usize,
    function_files: usize,
    recipe_files: usize,
    loot_files: usize,
    vanilla_worldgen_or_dim: bool,
    vanilla_asset_files: usize,
    namespaces: BTreeSet<String>,
}

fn inspect_archive_paths(names: &[String]) -> ArchiveDataSummary {
    let mut s = ArchiveDataSummary::default();
    for raw in names.iter().filter(|n| !n.ends_with('/')) {
        let mapped = carry::renamed_in_1_21(raw).unwrap_or_else(|| raw.clone());
        if let Some(rest) = mapped.strip_prefix("data/")
            && let Some((ns, tail)) = rest.split_once('/')
            && !ns.is_empty()
            && !tail.is_empty()
        {
            s.data_files += 1;
            s.namespaces.insert(ns.to_owned());
            if tail.starts_with("worldgen/")
                || tail.starts_with("dimension/")
                || tail.starts_with("dimension_type/")
            {
                s.worldgen_files += 1;
                if ns == "minecraft" {
                    s.vanilla_worldgen_or_dim = true;
                }
            } else if tail.starts_with("structure/") {
                s.structure_files += 1;
                if ns == "minecraft" {
                    s.vanilla_worldgen_or_dim = true;
                }
            } else if tail.starts_with("function/") {
                s.function_files += 1;
            } else if tail.starts_with("recipe/") {
                s.recipe_files += 1;
            } else if tail.starts_with("loot_table/") {
                s.loot_files += 1;
            }
        } else if let Some(rest) = raw.strip_prefix("assets/")
            && let Some((ns, tail)) = rest.split_once('/')
            && ns == "minecraft"
            && !tail.starts_with("lang/")
        {
            s.vanilla_asset_files += 1;
        }
    }
    s
}

/// If an outer Fabric/Forge wrapper jar holds no `data/` or `assets/` content of its own
/// (common on Modrinth for packaged datapacks like Terralith / Incendium / Nullscape),
/// extracts the largest non-library inner jar under `META-INF/jars/*.jar`.
fn maybe_extract_nested_content_jar(outer_path: &Path, names: &[String], work_dir: &Path) -> Option<PathBuf> {
    let has_direct_content = names.iter().any(|n| {
        !n.ends_with('/')
            && (n.starts_with("data/")
                || (n.starts_with("assets/") && !n.ends_with("icon.png") && !n.contains("/lang/")))
    });
    if has_direct_content {
        return None;
    }
    let nested_candidates: Vec<&String> = names
        .iter()
        .filter(|n| {
            n.starts_with("META-INF/jars/")
                && (n.ends_with(".jar") || n.ends_with(".zip"))
                && !n.contains("mixinextras")
        })
        .collect();
    if nested_candidates.is_empty() {
        return None;
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(outer_path).ok()?).ok()?;
    let mut best: Option<(u64, String, Vec<u8>)> = None;
    let mut budget = crate::entry::Budget::archive();
    for entry_name in nested_candidates {
        let Ok(entry) = zip.by_name(entry_name) else {
            continue;
        };
        let sz = entry.size();
        let Ok(bytes) = budget.read(entry, sz, 128 << 20) else {
            continue;
        };
        if best.as_ref().is_none_or(|(b_sz, _, _)| sz > *b_sz) {
            let fname = entry_name
                .rsplit('/')
                .next()
                .unwrap_or("nested.jar")
                .to_owned();
            best = Some((sz, fname, bytes));
        }
    }
    let (_, fname, bytes) = best?;
    fs::create_dir_all(work_dir).ok()?;
    let out = work_dir.join(format!("nested-{fname}"));
    fs::write(&out, bytes).ok()?;
    Some(out)
}

fn needs_26_3_worldgen_upgrade(files: &BTreeMap<String, Vec<u8>>) -> bool {
    for (path, bytes) in files {
        if path.contains("/configured_feature/") || path.contains("/configured_carver/") {
            return true;
        }
        if !path.ends_with(".json") || !path.starts_with("data/") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            continue;
        };
        if path.contains("/worldgen/noise/") && v.get("firstOctave").is_some() && v.get("amplitudes").is_some() && v.get("base_amplitude").is_none() {
            return true;
        }
        if path.contains("/worldgen/biome/") && v.get("spawners").is_some() {
            return true;
        }
        if path.contains("/worldgen/noise_settings/") && v.get("surface_rule").is_some() {
            return true;
        }
        if path.contains("/worldgen/density_function/")
            && let Ok(txt) = std::str::from_utf8(bytes)
            && (txt.contains("\"argument1\"") || txt.contains("\"argument2\""))
        {
            return true;
        }
    }
    false
}

/// Extracts `data/` from `archive_path`, upgrades pre-1.21 folder names, pre-1.20.5 recipes,
/// and pre-26.3 worldgen schemas into `<game_dir>/new-world-datapacks/<mod_id>/`.
fn write_upgraded_datapack(
    archive_path: &Path,
    mod_id: &str,
    display_name: &str,
    game_dir: &Path,
) -> Result<(PathBuf, usize, usize), String> {
    let file = fs::File::open(archive_path).map_err(|e| format!("{}: {e}", archive_path.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", archive_path.display()))?;
    let mut budget = crate::entry::Budget::archive();
    let mut raw_files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut renamed_folders = 0usize;

    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Ok(name) = entry.name() else {
            continue;
        };
        let raw_name = name.replace('\\', "/");
        if raw_name.ends_with('/') {
            continue;
        }
        if raw_name != "pack.mcmeta" && raw_name != "pack.png" && !raw_name.starts_with("data/") {
            continue;
        }
        let target_name = if let Some(renamed) = carry::renamed_in_1_21(&raw_name) {
            renamed_folders += 1;
            renamed
        } else {
            raw_name
        };
        if safe_rel_path(&target_name).is_none() {
            continue;
        }
        let declared = entry.size();
        let Ok(mut bytes) = budget.read(entry, declared, 32 << 20) else {
            continue;
        };
        if target_name.starts_with("data/")
            && target_name.contains("/recipe/")
            && target_name.ends_with(".json")
            && let Ok(mut v) = serde_json::from_slice::<Value>(&bytes)
        {
            if upgrade_recipe_json_in_place(&mut v)
                && let Ok(pretty) = serde_json::to_vec_pretty(&v)
            {
                bytes = pretty;
            }
        }
        raw_files.insert(target_name, bytes);
    }

    let data_count = raw_files.keys().filter(|k| k.starts_with("data/")).count();
    if data_count == 0 {
        return Err("archive holds no data/ files for datapack".into());
    }

    let (mut final_files, upg_report) = if needs_26_3_worldgen_upgrade(&raw_files) {
        upgrade::upgrade_pack(&raw_files)
    } else {
        (raw_files, upgrade::Report::default())
    };
    let upgraded_worldgen: usize = upg_report.upgraded.values().sum::<usize>() + upg_report.added.len();

    let title = if display_name.trim().is_empty() {
        mod_id
    } else {
        display_name.trim()
    };
    let mcmeta = json!({
        "pack": {
            "description": format!("{title} (BlockWay Rust 26.3 Datapack)"),
            "pack_format": upgrade::PACK_FORMAT,
            "min_format": upgrade::PACK_FORMAT,
            "max_format": upgrade::PACK_FORMAT
        }
    });
    if let Ok(pretty) = serde_json::to_vec_pretty(&mcmeta) {
        final_files.insert("pack.mcmeta".into(), pretty);
    }

    let dp_root = game_dir.join("new-world-datapacks").join(mod_id);
    if dp_root.exists() {
        let _ = fs::remove_dir_all(&dp_root);
    }
    fs::create_dir_all(&dp_root).map_err(|e| format!("{}: {e}", dp_root.display()))?;

    let mut written_count = 0usize;
    for (rel_str, bytes) in &final_files {
        let Some(rel) = safe_rel_path(rel_str) else {
            continue;
        };
        let dest = dp_root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
        written_count += 1;
    }

    Ok((dp_root, written_count, renamed_folders + upgraded_worldgen))
}

/// Packs `assets/` + `pack.mcmeta` into `<game_dir>/resourcepacks/<mod_id>-rust.zip`.
fn write_vanilla_resourcepack(
    archive_path: &Path,
    mod_id: &str,
    display_name: &str,
    game_dir: &Path,
) -> Result<(PathBuf, usize, u64), String> {
    let file = fs::File::open(archive_path).map_err(|e| format!("{}: {e}", archive_path.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", archive_path.display()))?;
    let mut budget = crate::entry::Budget::archive();
    let mut asset_entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut icon_bytes: Option<Vec<u8>> = None;

    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Ok(name) = entry.name() else {
            continue;
        };
        let raw_name = name.replace('\\', "/");
        if raw_name.ends_with('/') {
            continue;
        }
        let declared = entry.size();
        if raw_name == "pack.png" || raw_name.ends_with("/icon.png") {
            if icon_bytes.is_none()
                && let Ok(b) = budget.read(entry, declared, 4 << 20)
            {
                icon_bytes = Some(b);
            }
            continue;
        }
        if !raw_name.starts_with("assets/") {
            continue;
        }
        if safe_rel_path(&raw_name).is_none() {
            continue;
        }
        if let Ok(b) = budget.read(entry, declared, 64 << 20) {
            asset_entries.insert(raw_name, b);
        }
    }

    if asset_entries.is_empty() {
        return Err("no assets/ entries found for resource pack".into());
    }

    let rp_dir = game_dir.join("resourcepacks");
    fs::create_dir_all(&rp_dir).map_err(|e| format!("{}: {e}", rp_dir.display()))?;
    let final_path = rp_dir.join(format!("{mod_id}-rust.zip"));
    let partial_path = rp_dir.join(format!("{mod_id}-rust.zip.partial"));

    {
        let out_file =
            fs::File::create(&partial_path).map_err(|e| format!("{}: {e}", partial_path.display()))?;
        let mut writer = zip::ZipWriter::new(out_file);
        let opts = zip::write::SimpleFileOptions::default();

        let title = if display_name.trim().is_empty() {
            mod_id
        } else {
            display_name.trim()
        };
        let mcmeta = json!({
            "pack": {
                "pack_format": 55,
                "supported_formats": [15, 999],
                "min_format": 15,
                "max_format": 999,
                "description": format!("{title} (BlockWay Rust Resource Pack)")
            }
        });
        let mcmeta_bytes = serde_json::to_vec_pretty(&mcmeta).map_err(|e| e.to_string())?;
        writer
            .start_file("pack.mcmeta", opts)
            .map_err(|e| e.to_string())?;
        writer.write_all(&mcmeta_bytes).map_err(|e| e.to_string())?;

        if let Some(icon) = &icon_bytes {
            writer
                .start_file("pack.png", opts)
                .map_err(|e| e.to_string())?;
            writer.write_all(icon).map_err(|e| e.to_string())?;
        }

        for (name, bytes) in &asset_entries {
            writer.start_file(name, opts).map_err(|e| e.to_string())?;
            writer.write_all(bytes).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }

    if final_path.exists() {
        let _ = fs::remove_file(&final_path);
    }
    fs::rename(&partial_path, &final_path)
        .map_err(|e| format!("{}: {e}", final_path.display()))?;
    let size = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    Ok((final_path, asset_entries.len(), size))
}

/// Backward-compatible wrapper calling [`port_jar_with_game_dir`] with `out_dir.parent()`.
pub fn port_jar_to_hmod(
    jar_path: &Path,
    out_dir: &Path,
    resources: Option<&Path>,
) -> PortedMod {
    port_jar_with_game_dir(jar_path, out_dir, out_dir.parent(), resources)
}

/// Converts a single Java `.jar` or `.zip` mod/pack across all 5 Horizon Rust channels.
pub fn port_jar_with_game_dir(
    jar_path: &Path,
    out_dir: &Path,
    game_dir: Option<&Path>,
    resources: Option<&Path>,
) -> PortedMod {
    let effective_game_dir = game_dir
        .map(Path::to_path_buf)
        .or_else(|| out_dir.parent().map(Path::to_path_buf));
    let file_name = jar_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| jar_path.display().to_string());
    let mut base = PortedMod {
        jar_path: jar_path.display().to_string(),
        file_name: file_name.clone(),
        mod_id: String::new(),
        original_mod_id: String::new(),
        name: String::new(),
        version: String::new(),
        loader: String::new(),
        license: String::new(),
        legacy_mc_layout: false,
        kind: "java_code".into(),
        output_path: None,
        native_mod_id: None,
        verdict: Verdict::No,
        verdict_label: Verdict::No.label().to_owned(),
        summary: String::new(),
        hmod_path: None,
        hmod_size_bytes: 0,
        enabled: false,
        clash_warning: None,
        blocks: 0,
        items: 0,
        recipes: 0,
        loot_tables: 0,
        worldgen: 0,
        structures: 0,
        textures: 0,
        creative_tabs: 0,
        files_packed: 0,
        namespaces: Vec::new(),
        ports: Vec::new(),
        skipped: Vec::new(),
        error: None,
    };

    let mut info = match jar::read(jar_path) {
        Ok(i) => i,
        Err(e) => {
            base.summary = format!("Unreadable archive: {e}");
            base.error = Some(e);
            return base;
        }
    };
    base.original_mod_id = info.mod_id.clone();

    let file = match fs::File::open(jar_path) {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("{}: {e}", jar_path.display());
            base.summary = msg.clone();
            base.error = Some(msg);
            return base;
        }
    };
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(z) => z,
        Err(e) => {
            let msg = format!("{}: {e}", jar_path.display());
            base.summary = msg.clone();
            base.error = Some(msg);
            return base;
        }
    };
    let outer_names: Vec<String> = match zip
        .file_names()
        .map(|n| n.map(|n| n.into_owned()))
        .collect::<Result<_, _>>()
    {
        Ok(list) => list,
        Err(e) => {
            let msg = format!("{}: {e}", jar_path.display());
            base.summary = msg.clone();
            base.error = Some(msg);
            return base;
        }
    };

    let raw_id = info.mod_id.clone();
    base.loader = detect_loader(&outer_names).to_owned();
    if info.version.trim().is_empty() {
        info.version = "1.0.0".into();
    }
    if info.name.trim().is_empty() {
        info.name = raw_id.clone();
    }
    base.name = info.name.clone();
    base.version = info.version.clone();
    base.license = info.license.clone();

    // Channel 1: Native Rust Client Equivalent (`minimap`, `zoom`, `coords_hud`, `keystrokes`, `fps_overlay`).
    if let Some((native_id, native_title)) = detect_native_rust_equivalent(&raw_id, &file_name) {
        base.mod_id = native_id.to_owned();
        base.kind = "native_rust".into();
        base.native_mod_id = Some(native_id.to_owned());
        base.verdict = Verdict::Works;
        base.verdict_label = Verdict::Works.label().to_owned();
        base.ports
            .push(format!("100% Native Rust `{native_title}` ({native_id})"));
        base.summary = format!(
            "Runs natively via built-in 100% Rust `{native_title}` ({native_id})."
        );
        return base;
    }

    // Channel 2: Built-in Rust Engine Feature (Sodium, Iris, Lithium, FerriteCore, Fabric API, etc.).
    if let Some(engine_feature) = detect_builtin_engine_feature(&raw_id, &file_name) {
        base.mod_id = sanitize_mod_id(&raw_id);
        base.kind = "builtin_engine".into();
        base.enabled = true;
        base.verdict = Verdict::Works;
        base.verdict_label = Verdict::Works.label().to_owned();
        base.ports.push(engine_feature.to_owned());
        base.summary = format!("Already built into Horizon Rust Engine ({engine_feature}).");
        return base;
    }

    let work_dir = temp_work_dir(&sanitize_mod_id(&raw_id));

    // Unwrap nested `META-INF/jars/*.jar` if the outer jar is a pure Modrinth datapack wrapper.
    let nested_jar = maybe_extract_nested_content_jar(jar_path, &outer_names, &work_dir);
    let content_archive_path: &Path = nested_jar.as_deref().unwrap_or(jar_path);
    let names: Vec<String> = if let Some(inner) = &nested_jar
        && let Ok(f) = fs::File::open(inner)
        && let Ok(z) = zip::ZipArchive::new(f)
        && let Ok(inner_names) = z
            .file_names()
            .map(|n| n.map(|n| n.into_owned()))
            .collect::<Result<Vec<_>, _>>()
    {
        if let Ok(inner_info) = jar::read(inner) {
            info.census = inner_info.census;
        }
        inner_names
    } else {
        outer_names.clone()
    };

    info.mod_id = resolve_mod_id(&raw_id, &names);
    base.mod_id = info.mod_id.clone();
    base.legacy_mc_layout = carry::old_layout(&names);

    // Bytecode census on outer archive.
    let mut budget = crate::entry::Budget::archive();
    let mut class_bytes = Vec::new();
    for entry_name in outer_names.iter().filter(|n| n.ends_with(".class")) {
        let Ok(entry) = zip.by_name(entry_name) else {
            continue;
        };
        let declared = entry.size();
        match budget.read(entry, declared, (1 << 24) - 1) {
            Ok(b) => class_bytes.push(b),
            Err(e) if crate::entry::is_too_big(&e) && declared < 1 << 24 => {
                let _ = fs::remove_dir_all(&work_dir);
                let msg = format!("{}: classes unpack past 1 GiB limit", jar_path.display());
                base.summary = msg.clone();
                base.error = Some(msg);
                return base;
            }
            Err(_) => {}
        }
    }
    let code = classes::census(class_bytes.iter().map(Vec::as_slice));
    let archive_summary = inspect_archive_paths(&names);

    let vanilla = vanilla_blocks_from_resources(resources);
    let allowed = |block_id: &str| vanilla.as_ref().is_none_or(|set| set.contains(block_id));
    let alone = |m: &str| m == info.mod_id || m == "minecraft";
    let pack = carry::Pack {
        loaded: &alone,
        mods: &alone,
    };

    let mut report = match carry::carry(
        content_archive_path,
        &info,
        &work_dir,
        None,
        &allowed,
        &pack,
        &BTreeMap::new(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_dir_all(&work_dir);
            base.summary = format!("Can't be carried over: {e}");
            base.error = Some(e);
            return base;
        }
    };

    let carry_dir = work_dir.join("carry");
    let owned_namespaces: Vec<String> = std::iter::once(info.mod_id.clone())
        .chain(report.namespaces.iter().cloned())
        .collect();

    let legacy_upgraded =
        upgrade_legacy_carried_recipes(&carry_dir, &owned_namespaces, &mut report.recipes_kept);
    report.recipes_upgraded += legacy_upgraded;
    report.recipes_old_result = report.recipes_old_result.saturating_sub(legacy_upgraded);

    if let Err(e) = ensure_creative_tab(&carry_dir, &info.mod_id, &info.name) {
        let _ = fs::remove_dir_all(&work_dir);
        base.summary = format!("Failed writing creative tab: {e}");
        base.error = Some(e);
        return base;
    }

    let c = &info.census;
    let recipes = report
        .recipes_upgraded
        .saturating_sub(report.recipes_unregistered.len());
    let loot = c.loot_tables + report.loot_tables_added;

    // Channel 4: Worldgen / Biome / Structure / Dimension / SkyBlock / OneBlock Datapack!
    // Triggered when the mod adds NO custom block/item registries, and holds worldgen/structure/
    // function/vanilla-override data (which `.hmod` forbids on `minecraft:*` namespaces, whereas
    // `<game_dir>/new-world-datapacks/<id>/` runs 100% natively in Horizon!).
    let prefers_datapack = report.blocks.is_empty()
        && report.items == 0
        && report.blocks_unknown.is_empty()
        && archive_summary.data_files > 0
        && (archive_summary.vanilla_worldgen_or_dim
            || archive_summary.worldgen_files > 0
            || archive_summary.structure_files > 0
            || archive_summary.function_files > 0
            || !code.behaviour());

    if prefers_datapack
        && let Some(gdir) = effective_game_dir.as_deref()
        && let Ok((dp_path, files_written, upgraded_count)) =
            write_upgraded_datapack(content_archive_path, &info.mod_id, &info.name, gdir)
    {
        if archive_summary.vanilla_asset_files > 0 {
            let _ = write_vanilla_resourcepack(content_archive_path, &info.mod_id, &info.name, gdir);
        }
        let _ = fs::remove_dir_all(&work_dir);
        base.kind = "datapack".into();
        base.output_path = Some(dp_path.display().to_string());
        base.enabled = true;
        base.files_packed = files_written;
        base.worldgen = archive_summary.worldgen_files;
        base.structures = archive_summary.structure_files;
        base.recipes = archive_summary.recipe_files;
        base.loot_tables = archive_summary.loot_files;
        base.namespaces = archive_summary.namespaces.into_iter().collect();

        for (n, one, many) in [
            (archive_summary.worldgen_files, "worldgen file", "worldgen files"),
            (archive_summary.structure_files, "structure", "structures"),
            (archive_summary.function_files, "function", "functions"),
            (archive_summary.recipe_files, "recipe", "recipes"),
            (archive_summary.loot_files, "loot table", "loot tables"),
        ] {
            if n > 0 {
                base.ports.push(plural(n, one, many));
            }
        }
        if base.ports.is_empty() {
            base.ports
                .push(plural(files_written, "datapack file", "datapack files"));
        }
        if upgraded_count > 0 {
            base.ports
                .push(format!("{upgraded_count} files upgraded to 26.3 format"));
        }

        if code.behaviour() {
            base.verdict = Verdict::Partly;
            base.verdict_label = Verdict::Partly.label().to_owned();
            let preview = base.ports[..base.ports.len().min(3)].join(", ");
            base.summary = format!(
                "Converted to Rust 26.3 Datapack ({preview}); Java helper classes ignored."
            );
        } else {
            base.verdict = Verdict::Works;
            base.verdict_label = Verdict::Works.label().to_owned();
            let preview = base.ports[..base.ports.len().min(3)].join(", ");
            base.summary = format!("100% Rust 26.3 Datapack ready ({preview}).");
        }
        return base;
    }

    // Channel 5: Vanilla Texture / Model / Sound / CEM / CIT Resource Pack!
    let prefers_resourcepack = report.blocks.is_empty()
        && report.items == 0
        && archive_summary.data_files == 0
        && (archive_summary.vanilla_asset_files > 0 || (c.textures > 0 && !code.behaviour()));

    if prefers_resourcepack
        && let Some(gdir) = effective_game_dir.as_deref()
        && let Ok((rp_path, asset_count, rp_size)) =
            write_vanilla_resourcepack(content_archive_path, &info.mod_id, &info.name, gdir)
    {
        let _ = fs::remove_dir_all(&work_dir);
        base.kind = "resourcepack".into();
        base.output_path = Some(rp_path.display().to_string());
        base.hmod_size_bytes = rp_size;
        base.enabled = true;
        base.files_packed = asset_count;
        base.textures = c.textures;
        base.ports
            .push(plural(asset_count, "asset file", "asset files"));
        if code.behaviour() {
            base.verdict = Verdict::Partly;
            base.verdict_label = Verdict::Partly.label().to_owned();
            base.summary = format!(
                "Extracted {asset_count} textures/models to Rust Resource Pack; Java code skipped."
            );
        } else {
            base.verdict = Verdict::Works;
            base.verdict_label = Verdict::Works.label().to_owned();
            base.summary = format!(
                "100% Rust Resource Pack ready ({asset_count} asset files)."
            );
        }
        return base;
    }

    // Channel 3: Custom Block / Item / Content `.hmod` Package!
    let pure_asset_or_data_files = if !code.behaviour() && code.classes == 0 {
        report.copied
    } else {
        0
    };
    let content_score = report.blocks.len()
        + report.items
        + recipes
        + loot
        + report.worldgen
        + c.structures
        + pure_asset_or_data_files;

    for (n, one, many) in [
        (report.blocks.len(), "block", "blocks"),
        (report.items, "item", "items"),
        (recipes, "recipe", "recipes"),
        (loot, "loot table", "loot tables"),
        (report.worldgen, "worldgen file", "worldgen files"),
        (c.structures, "structure", "structures"),
        (c.textures, "texture", "textures"),
    ] {
        if n > 0 {
            base.ports.push(plural(n, one, many));
        }
    }
    if base.legacy_mc_layout && report.folders_renamed > 0 {
        base.ports.push(format!(
            "{} pre-1.21 data files upgraded to 26.3 layout",
            report.folders_renamed
        ));
    }

    let mut skipped = Vec::new();
    if !report.blocks_unknown.is_empty() {
        skipped.push(plural(
            report.blocks_unknown.len(),
            "block with custom state properties (left out)",
            "blocks with custom state properties (left out)",
        ));
    }
    if !report.recipes_unregistered.is_empty() {
        skipped.push(plural(
            report.recipes_unregistered.len(),
            "recipe for a left-out block",
            "recipes for left-out blocks",
        ));
    }
    if !report.recipes_kept.is_empty() {
        skipped.push(plural(
            report.recipes_kept.len(),
            "custom Java recipe serializer",
            "custom Java recipe serializers",
        ));
    }
    if !report.unreadable.is_empty() {
        skipped.push(plural(
            report.unreadable.len(),
            "unreadable data/asset file",
            "unreadable data/asset files",
        ));
    }
    let wg = &report.worldgen_types;
    let dropped = wg.processors_dropped.values().sum::<usize>();
    if dropped > 0 {
        skipped.push(plural(
            dropped,
            "Java structure processor",
            "Java structure processors",
        ));
    }
    let unsupported = wg.unsupported.values().sum::<usize>();
    if unsupported > 0 {
        skipped.push(plural(
            unsupported,
            "Java worldgen serializer",
            "Java worldgen serializers",
        ));
    }
    for (feature, count) in &code.features {
        let mut line = feature.describe(*count);
        if *feature == classes::Feature::Events {
            let sample: Vec<&str> = code.events.iter().take(3).map(String::as_str).collect();
            if !sample.is_empty() {
                line.push_str(&format!(" ({}", sample.join(", ")));
                if code.events.len() > 3 {
                    line.push_str(", ...");
                }
                line.push(')');
            }
        }
        skipped.push(line);
    }
    base.skipped = skipped;
    base.namespaces = owned_namespaces;
    base.blocks = report.blocks.len();
    base.items = report.items;
    base.recipes = recipes;
    base.loot_tables = loot;
    base.worldgen = report.worldgen;
    base.structures = c.structures;
    base.textures = c.textures;

    if content_score == 0 {
        // Fallback: if a Java code mod still ships vanilla texture/model overrides, rescue its assets!
        if archive_summary.vanilla_asset_files > 0
            && let Some(gdir) = effective_game_dir.as_deref()
            && let Ok((rp_path, asset_count, rp_size)) =
                write_vanilla_resourcepack(content_archive_path, &info.mod_id, &info.name, gdir)
        {
            let _ = fs::remove_dir_all(&work_dir);
            base.kind = "resourcepack".into();
            base.output_path = Some(rp_path.display().to_string());
            base.hmod_size_bytes = rp_size;
            base.enabled = true;
            base.files_packed = asset_count;
            base.ports
                .push(plural(asset_count, "asset file", "asset files"));
            base.verdict = Verdict::Partly;
            base.verdict_label = Verdict::Partly.label().to_owned();
            base.summary = format!(
                "Extracted {asset_count} vanilla textures/models to Resource Pack; Java bytecode skipped."
            );
            return base;
        }

        let _ = fs::remove_dir_all(&work_dir);
        let reason = if code.behaviour() || code.classes > 0 {
            "Deep Java bytecode/Mixin mod (requires Java Loader or native Rust equivalent)."
        } else {
            "No portable blocks, items, recipes, worldgen or assets found in archive."
        };
        base.kind = "java_code".into();
        base.verdict = Verdict::No;
        base.verdict_label = Verdict::No.label().to_owned();
        base.summary = reason.to_owned();
        return base;
    }

    match pack_carried_dir_to_hmod(&carry_dir, out_dir) {
        Ok((hmod_path, packed_content)) => {
            let _ = fs::remove_dir_all(&work_dir);
            let size = fs::metadata(&hmod_path).map(|m| m.len()).unwrap_or(0);
            base.kind = "hmod".into();
            base.blocks = packed_content.blocks.len();
            base.items = packed_content.items.len();
            base.creative_tabs = packed_content.creative_tabs.len();
            base.files_packed = packed_content.files.len();
            base.namespaces = packed_content.namespaces;
            let p_str = hmod_path.display().to_string();
            base.output_path = Some(p_str.clone());
            base.hmod_path = Some(p_str);
            base.hmod_size_bytes = size;
            if base.skipped.is_empty() {
                base.verdict = Verdict::Works;
                base.verdict_label = Verdict::Works.label().to_owned();
                let preview = base.ports[..base.ports.len().min(3)].join(", ");
                base.summary = format!("100% Rust .hmod ready ({preview}).");
            } else {
                base.verdict = Verdict::Partly;
                base.verdict_label = Verdict::Partly.label().to_owned();
                let ported_preview = base.ports[..base.ports.len().min(3)].join(", ");
                let skipped_preview = base.skipped[..base.skipped.len().min(2)].join(", ");
                base.summary =
                    format!("Ported {ported_preview} to Rust .hmod; skipped {skipped_preview}.");
            }
            base
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&work_dir);
            base.kind = "java_code".into();
            base.verdict = Verdict::No;
            base.verdict_label = Verdict::No.label().to_owned();
            base.summary = format!("Failed validation for Horizon .hmod: {e}");
            base.error = Some(e);
            base
        }
    }
}

#[derive(Default)]
struct ModsTxtState {
    enabled: BTreeSet<String>,
    disabled: BTreeSet<String>,
    other_lines: Vec<String>,
}

impl ModsTxtState {
    fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        let mut state = Self::default();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once(':')
                && let Some(id) = k.trim().strip_prefix("mod.")
            {
                match v.trim() {
                    "true" => {
                        state.enabled.insert(id.to_owned());
                        continue;
                    }
                    "false" => {
                        state.disabled.insert(id.to_owned());
                        continue;
                    }
                    _ => {}
                }
            }
            state.other_lines.push(trimmed.to_owned());
        }
        state
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut out = String::new();
        for id in &self.enabled {
            out.push_str(&format!("mod.{id}:true\n"));
        }
        for id in &self.disabled {
            out.push_str(&format!("mod.{id}:false\n"));
        }
        for line in &self.other_lines {
            out.push_str(line);
            out.push('\n');
        }
        fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Updates `mod.<mod_id>:true` / `mod.<mod_id>:false` in `horizon-mods.txt`.
/// Native built-in Rust mods (`minimap`, `zoom`, `fps_overlay`, `coords_hud`, `keystrokes`)
/// explicitly write `mod.<id>:false` when disabled (matching `ModList::is_on`), whereas
/// `.hmod` packages are off when absent from `enabled`.
pub fn set_mod_enabled(
    mods_txt: &Path,
    mods_dir: Option<&Path>,
    mod_id: &str,
    enabled: bool,
) -> Result<Option<String>, String> {
    let mut state = ModsTxtState::load(mods_txt);
    if is_native_mod_id(mod_id) {
        if enabled {
            state.disabled.remove(mod_id);
            state.enabled.insert(mod_id.to_owned());
        } else {
            state.enabled.remove(mod_id);
            state.disabled.insert(mod_id.to_owned());
        }
        state.save(mods_txt)?;
        return Ok(None);
    }

    if !enabled {
        state.enabled.remove(mod_id);
        state.disabled.remove(mod_id);
        state.save(mods_txt)?;
        return Ok(None);
    }

    if let Some(dir) = mods_dir {
        let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for path in find(dir) {
            if let Ok(pkg) = Package::read(&path) {
                let ns: Vec<String> = pkg
                    .manifest
                    .owned_namespaces()
                    .map(str::to_owned)
                    .collect();
                by_id.insert(pkg.manifest.id.clone(), ns);
            }
        }
        if let Some(target_ns) = by_id.get(mod_id) {
            for active_id in &state.enabled {
                if active_id == mod_id {
                    continue;
                }
                if let Some(other_ns) = by_id.get(active_id)
                    && let Some(reason) = clash((mod_id, target_ns), (active_id, other_ns))
                {
                    state.enabled.remove(mod_id);
                    state.save(mods_txt)?;
                    return Ok(Some(reason));
                }
            }
        }
    }

    state.disabled.remove(mod_id);
    state.enabled.insert(mod_id.to_owned());
    state.save(mods_txt)?;
    Ok(None)
}

pub fn list_native_mods(mods_txt: Option<&Path>) -> Vec<NativeModStatus> {
    let state = mods_txt.map(ModsTxtState::load).unwrap_or_default();
    NATIVE_MOD_SPECS
        .iter()
        .map(|(id, name, desc)| NativeModStatus {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            description: (*desc).to_owned(),
            enabled: !state.disabled.contains(*id),
        })
        .collect()
}

pub fn list_installed_datapacks(game_dir: &Path) -> Vec<InstalledDatapack> {
    let root = game_dir.join("new-world-datapacks");
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let id = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            let mut files = 0usize;
            let mut stack = vec![path.clone()];
            while let Some(d) = stack.pop() {
                if let Ok(sub) = fs::read_dir(d) {
                    for s in sub.flatten() {
                        let sp = s.path();
                        if sp.is_dir() {
                            stack.push(sp);
                        } else {
                            files += 1;
                        }
                    }
                }
            }
            let desc = fs::read(path.join("pack.mcmeta"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["pack"]["description"].as_str().map(str::to_owned))
                .unwrap_or_else(|| id.clone());
            items.push(InstalledDatapack {
                name: id.clone(),
                id,
                description: desc,
                path: path.display().to_string(),
                files,
            });
        } else if path.extension().and_then(|e| e.to_str()) == Some("zip") {
            items.push(InstalledDatapack {
                name: id.clone(),
                id,
                description: "Datapack zip".into(),
                path: path.display().to_string(),
                files: 1,
            });
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
}

pub fn list_ported_resourcepacks(game_dir: &Path) -> Vec<InstalledResourcePack> {
    let root = game_dir.join("resourcepacks");
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let fname = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if fname.ends_with("-rust.zip") {
            let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            items.push(InstalledResourcePack {
                file_name: fname,
                path: path.display().to_string(),
                size_bytes,
            });
        }
    }
    items.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    items
}

/// Lists all `.hmod` packages and folder mods inside `mods_dir` (`horizon-mods/`),
/// validated through `horizon_mods::Package::read` and `Content::read`.
pub fn list_installed_hmods(
    mods_dir: &Path,
    mods_txt: Option<&Path>,
) -> Result<Vec<InstalledHmod>, String> {
    let enabled_set = mods_txt
        .map(ModsTxtState::load)
        .map(|s| s.enabled)
        .unwrap_or_default();

    let paths = find(mods_dir);
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        match Package::read(&path) {
            Ok(pkg) => {
                let m = &pkg.manifest;
                let enabled = enabled_set.contains(&m.id);
                match Content::read(&path, m) {
                    Ok(content) => items.push(InstalledHmod {
                        id: m.id.clone(),
                        name: m.name.clone(),
                        version: m.version.clone(),
                        description: m.description.clone(),
                        api: m.api.clone(),
                        file_name,
                        path: path.display().to_string(),
                        size_bytes,
                        enabled,
                        blocks: content.blocks.len(),
                        items: content.items.len(),
                        creative_tabs: content.creative_tabs.len(),
                        effects: content.effects.len(),
                        fluids: content.fluids.len(),
                        entity_types: content.entity_types.len(),
                        files: content.files.len(),
                        namespaces: content.namespaces,
                        clash: None,
                        problem: None,
                    }),
                    Err(err) => items.push(InstalledHmod {
                        id: m.id.clone(),
                        name: m.name.clone(),
                        version: m.version.clone(),
                        description: m.description.clone(),
                        api: m.api.clone(),
                        file_name,
                        path: path.display().to_string(),
                        size_bytes,
                        enabled,
                        blocks: 0,
                        items: 0,
                        creative_tabs: 0,
                        effects: 0,
                        fluids: 0,
                        entity_types: 0,
                        files: 0,
                        namespaces: m.owned_namespaces().map(str::to_owned).collect(),
                        clash: None,
                        problem: Some(err),
                    }),
                }
            }
            Err(err) => {
                let fallback_id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                items.push(InstalledHmod {
                    enabled: enabled_set.contains(&fallback_id),
                    id: fallback_id.clone(),
                    name: fallback_id,
                    version: String::new(),
                    description: String::new(),
                    api: String::new(),
                    file_name,
                    path: path.display().to_string(),
                    size_bytes,
                    blocks: 0,
                    items: 0,
                    creative_tabs: 0,
                    effects: 0,
                    fluids: 0,
                    entity_types: 0,
                    files: 0,
                    namespaces: Vec::new(),
                    clash: None,
                    problem: Some(err),
                });
            }
        }
    }

    for i in 0..items.len() {
        if !items[i].enabled {
            continue;
        }
        for j in 0..items.len() {
            if i == j || !items[j].enabled {
                continue;
            }
            if let Some(reason) = clash(
                (&items[i].id, &items[i].namespaces),
                (&items[j].id, &items[j].namespaces),
            ) {
                items[i].clash = Some(reason);
                break;
            }
        }
    }
    Ok(items)
}

fn copy_dir_files_shallow(src: &Path, dst: &Path, exts: &[&str]) -> usize {
    if !src.is_dir() || src == dst {
        return 0;
    }
    let Ok(entries) = fs::read_dir(src) else {
        return 0;
    };
    let _ = fs::create_dir_all(dst);
    let mut copied = 0usize;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ok_ext = exts.is_empty()
            || p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.iter().any(|w| e.eq_ignore_ascii_case(w)));
        if !ok_ext {
            continue;
        }
        if let Some(fname) = p.file_name() {
            let dest = dst.join(fname);
            if !dest.exists() && fs::copy(&p, &dest).is_ok() {
                copied += 1;
            }
        }
    }
    copied
}

fn extract_pack_overrides_from_zip(pack_zip: &Path, game_dir: &Path) -> Result<usize, String> {
    let file = fs::File::open(pack_zip).map_err(|e| format!("{}: {e}", pack_zip.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", pack_zip.display()))?;
    let custom_root = zip
        .by_name("manifest.json")
        .ok()
        .and_then(|mut entry| {
            let mut s = String::new();
            entry.read_to_string(&mut s).ok()?;
            let v: Value = serde_json::from_str(&s).ok()?;
            v["overrides"].as_str().map(str::to_owned)
        })
        .unwrap_or_else(|| "overrides".into());

    let roots = [
        custom_root.as_str(),
        "overrides",
        "client-overrides",
    ];
    let mut budget = crate::entry::Budget::archive();
    let mut extracted = 0usize;

    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Ok(name) = entry.name() else {
            continue;
        };
        let raw_name = name.replace('\\', "/");
        if raw_name.ends_with('/') {
            continue;
        }
        let mut matched_rel: Option<&str> = None;
        for r in roots {
            if let Some(rest) = raw_name.strip_prefix(r).and_then(|s| s.strip_prefix('/')) {
                matched_rel = Some(rest);
                break;
            }
        }
        let Some(rel_str) = matched_rel else {
            continue;
        };
        let Some(rel) = safe_rel_path(rel_str) else {
            continue;
        };
        let dest = game_dir.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let declared = entry.size();
        let Ok(bytes) = budget.read(entry, declared, 128 << 20) else {
            continue;
        };
        fs::write(&dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
        extracted += 1;
    }
    Ok(extracted)
}

/// Ports an entire Modpack (`.mrpack`, CurseForge `.zip`, or installed Modpack instance directory)
/// into `game_dir` for 100% Native Rust Horizon execution.
pub fn port_modpack(
    pack_path: &Path,
    game_dir: &Path,
    resources: Option<&Path>,
    auto_enable: bool,
) -> Result<ModpackPortReport, String> {
    fs::create_dir_all(game_dir).map_err(|e| format!("{}: {e}", game_dir.display()))?;
    let hmods_dir = game_dir.join("horizon-mods");
    fs::create_dir_all(&hmods_dir).map_err(|e| format!("{}: {e}", hmods_dir.display()))?;
    let mods_txt = game_dir.join("horizon-mods.txt");

    let pack = import::import(pack_path)?;
    let mut overrides_extracted = 0usize;
    let mut resourcepacks_copied = 0usize;
    let mut shaderpacks_copied = 0usize;
    let mut downloads_fetched = 0usize;
    let mut download_warnings = Vec::new();
    let mut candidate_archives: Vec<PathBuf> = Vec::new();
    let mut seen_paths = BTreeSet::new();

    let mut push_candidate = |p: PathBuf| {
        if p.is_file() && seen_paths.insert(p.clone()) {
            candidate_archives.push(p);
        }
    };

    if pack_path.is_dir() {
        resourcepacks_copied += copy_dir_files_shallow(
            &pack_path.join("resourcepacks"),
            &game_dir.join("resourcepacks"),
            &["zip"],
        );
        shaderpacks_copied += copy_dir_files_shallow(
            &pack_path.join("shaderpacks"),
            &game_dir.join("shaderpacks"),
            &["zip"],
        );
        for sub in ["mods", "datapacks"] {
            let dir = pack_path.join(sub);
            if let Ok(entries) = fs::read_dir(&dir) {
                let mut list: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .and_then(|x| x.to_str())
                                .is_some_and(|x| {
                                    x.eq_ignore_ascii_case("jar") || x.eq_ignore_ascii_case("zip")
                                })
                    })
                    .collect();
                list.sort();
                for p in list {
                    push_candidate(p);
                }
            }
        }
    } else {
        overrides_extracted = extract_pack_overrides_from_zip(pack_path, game_dir)?;
        let cache_dir = game_dir.join(".horizon-pack-cache");
        let _ = fs::create_dir_all(&cache_dir);

        for pf in &pack.files {
            if pf.client == "unsupported" {
                continue;
            }
            let Some(rel) = safe_rel_path(&pf.path) else {
                continue;
            };
            let kind = pf.kind();
            let fname = pf.file_name();
            if fname.is_empty() || fname.contains(['/', '\\', ':', '\0']) {
                continue;
            }

            if kind == "resourcepacks" || kind == "shaderpacks" {
                let dest = game_dir.join(&rel);
                if !dest.exists()
                    && let Some(url) = pf.downloads.first()
                {
                    if let Some(parent) = dest.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    match fetch::download(url, pf.sha1.as_deref(), &dest, true) {
                        Ok(()) => {
                            downloads_fetched += 1;
                            if kind == "resourcepacks" {
                                resourcepacks_copied += 1;
                            } else {
                                shaderpacks_copied += 1;
                            }
                        }
                        Err(e) => download_warnings.push(format!("{fname}: {e}")),
                    }
                }
                continue;
            }

            if kind == "mods" || kind == "datapacks" {
                let existing_in_game = game_dir.join(&rel);
                if existing_in_game.is_file() {
                    push_candidate(existing_in_game);
                    continue;
                }
                let cached = cache_dir.join(fname);
                if cached.is_file() {
                    push_candidate(cached);
                    continue;
                }
                if let Some(url) = pf.downloads.first() {
                    match fetch::download(url, pf.sha1.as_deref(), &cached, true) {
                        Ok(()) => {
                            downloads_fetched += 1;
                            push_candidate(cached);
                        }
                        Err(e) => download_warnings.push(format!("{fname}: {e}")),
                    }
                } else if pf.curseforge.is_some() {
                    download_warnings.push(format!(
                        "{fname}: CurseForge-only manifest entry (install pack first or use Modrinth .mrpack)"
                    ));
                }
            }
        }

        // Also pick up any override jars/datapacks extracted into `<game_dir>/mods` or `datapacks`.
        for sub in ["mods", "datapacks"] {
            let dir = game_dir.join(sub);
            if let Ok(entries) = fs::read_dir(&dir) {
                let mut list: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .and_then(|x| x.to_str())
                                .is_some_and(|x| {
                                    x.eq_ignore_ascii_case("jar") || x.eq_ignore_ascii_case("zip")
                                })
                    })
                    .collect();
                list.sort();
                for p in list {
                    push_candidate(p);
                }
            }
        }
    }

    let mut results = Vec::with_capacity(candidate_archives.len());
    let mut works = 0usize;
    let mut partly = 0usize;
    let mut skipped_code_only = 0usize;
    let mut hmods_created = 0usize;
    let mut datapacks_created = 0usize;
    let mut resourcepacks_created = 0usize;
    let mut native_rust_enabled = 0usize;
    let mut builtin_engine_matched = 0usize;

    for archive_path in &candidate_archives {
        let mut item =
            port_jar_with_game_dir(archive_path, &hmods_dir, Some(game_dir), resources);
        match item.verdict {
            Verdict::Works => works += 1,
            Verdict::Partly => partly += 1,
            Verdict::No => skipped_code_only += 1,
        }
        match item.kind.as_str() {
            "hmod" => {
                hmods_created += 1;
                if auto_enable {
                    match set_mod_enabled(&mods_txt, Some(&hmods_dir), &item.mod_id, true) {
                        Ok(None) => item.enabled = true,
                        Ok(Some(clash)) => item.clash_warning = Some(clash),
                        Err(e) => item.clash_warning = Some(e),
                    }
                }
            }
            "native_rust" => {
                native_rust_enabled += 1;
                if auto_enable
                    && let Some(nid) = item.native_mod_id.clone()
                {
                    if set_mod_enabled(&mods_txt, Some(&hmods_dir), &nid, true).is_ok() {
                        item.enabled = true;
                    }
                }
            }
            "datapack" => datapacks_created += 1,
            "resourcepack" => resourcepacks_created += 1,
            "builtin_engine" => builtin_engine_matched += 1,
            _ => {}
        }
        results.push(item);
    }

    Ok(ModpackPortReport {
        pack_name: if pack.name.is_empty() {
            pack_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Modpack".into())
        } else {
            pack.name
        },
        pack_format: pack.format,
        minecraft: pack.minecraft,
        loader: pack.loader,
        game_dir: game_dir.display().to_string(),
        total_mods: results.len(),
        works,
        partly,
        skipped_code_only,
        hmods_created,
        datapacks_created,
        resourcepacks_created,
        native_rust_enabled,
        builtin_engine_matched,
        overrides_extracted,
        resourcepacks_copied,
        shaderpacks_copied,
        downloads_fetched,
        download_warnings,
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_jar(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(name);
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (entry_name, bytes) in entries {
            zip.start_file(*entry_name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-port-jar-test-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ports_modern_and_legacy_1_20_jars_into_valid_hmods() {
        let dir = temp("modern-and-legacy");
        let hmods_dir = dir.join("horizon-mods");
        let mods_txt = dir.join("horizon-mods.txt");

        let fabric_json = br#"{
            "schemaVersion": 1,
            "id": "ruby-gems",
            "version": "1.4.2",
            "name": "Ruby Gems",
            "license": "MIT"
        }"#;
        let blockstate = br#"{"variants": {"": {"model": "ruby_gems:block/ruby_block"}}}"#;
        let block_model =
            br#"{"parent": "minecraft:block/cube_all", "textures": {"all": "ruby_gems:block/ruby_block"}}"#;
        let item_model =
            br#"{"parent": "minecraft:item/generated", "textures": {"layer0": "ruby_gems:item/ruby"}}"#;
        let legacy_shaped = br#"{
            "type": "minecraft:crafting_shapeless",
            "ingredients": [{"item": "ruby_gems:ruby_block"}],
            "result": {"item": "ruby_gems:ruby", "count": 9}
        }"#;
        let legacy_stonecut = br#"{
            "type": "minecraft:stonecutting",
            "ingredient": {"item": "ruby_gems:ruby_block"},
            "result": "ruby_gems:ruby",
            "count": 9
        }"#;
        let legacy_loot = br#"{
            "type": "minecraft:block",
            "pools": [{
                "rolls": 1,
                "entries": [{"type": "minecraft:item", "name": "ruby_gems:ruby_block"}],
                "conditions": [{"condition": "minecraft:survives_explosion"}]
            }]
        }"#;
        let jar_path = make_jar(
            &dir,
            "ruby-gems-1.20.1.jar",
            &[
                ("fabric.mod.json", fabric_json),
                ("assets/ruby_gems/blockstates/ruby_block.json", blockstate),
                ("assets/ruby_gems/models/block/ruby_block.json", block_model),
                ("assets/ruby_gems/models/item/ruby.json", item_model),
                ("data/ruby_gems/recipes/ruby_from_block.json", legacy_shaped),
                ("data/ruby_gems/recipes/ruby_cut.json", legacy_stonecut),
                (
                    "data/ruby_gems/loot_tables/blocks/ruby_block.json",
                    legacy_loot,
                ),
            ],
        );

        let ported = port_jar_to_hmod(&jar_path, &hmods_dir, None);
        assert_eq!(ported.verdict, Verdict::Works, "{ported:?}");
        assert_eq!(ported.kind, "hmod");
        assert_eq!(ported.mod_id, "ruby_gems");
        assert!(ported.legacy_mc_layout);
        assert_eq!(ported.blocks, 1);
        assert_eq!(ported.items, 1);
        assert_eq!(ported.recipes, 2);
        assert_eq!(ported.creative_tabs, 1);
        let hmod_path = PathBuf::from(ported.hmod_path.as_ref().unwrap());
        assert!(hmod_path.is_file());

        let pkg = Package::read(&hmod_path).unwrap();
        let content = Content::read(&hmod_path, &pkg.manifest).unwrap();
        assert_eq!(content.blocks.len(), 1);
        assert_eq!(content.items.len(), 1);
        assert_eq!(content.creative_tabs.len(), 1);
        let upgraded_recipe: Value = serde_json::from_slice(
            &content
                .file("data/ruby_gems/recipe/ruby_from_block.json")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            upgraded_recipe["result"],
            json!({"id": "ruby_gems:ruby", "count": 9})
        );
        let upgraded_cut: Value = serde_json::from_slice(
            &content.file("data/ruby_gems/recipe/ruby_cut.json").unwrap(),
        )
        .unwrap();
        assert_eq!(
            upgraded_cut["result"],
            json!({"id": "ruby_gems:ruby", "count": 9})
        );
        assert!(upgraded_cut.get("count").is_none());

        let clash = set_mod_enabled(&mods_txt, Some(&hmods_dir), "ruby_gems", true).unwrap();
        assert!(clash.is_none());
        let listed = list_installed_hmods(&hmods_dir, Some(&mods_txt)).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].enabled);

        let class_bytes =
            crate::classes::tests_support::class_file("opt/FastMath", "java/lang/Object");
        let code_jar = make_jar(
            &dir,
            "fastmath.jar",
            &[
                (
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"id":"fastmath","version":"1.0","name":"FastMath"}"#,
                ),
                ("opt/FastMath.class", &class_bytes),
            ],
        );
        let no_port = port_jar_to_hmod(&code_jar, &hmods_dir, None);
        assert_eq!(no_port.verdict, Verdict::No);
        assert_eq!(no_port.kind, "java_code");
        assert!(no_port.hmod_path.is_none());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ports_all_five_channels_and_modpack_mrpack() {
        let dir = temp("five-channels-and-pack");
        let game_dir = dir.join("game");
        let hmods_dir = game_dir.join("horizon-mods");
        let mods_txt = game_dir.join("horizon-mods.txt");

        // Channel 1: Xaero's Minimap -> native_rust `minimap`
        let xaero_jar = make_jar(
            &dir,
            "Xaeros_Minimap_24.6.1_Fabric_1.21.11.jar",
            &[(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"xaerominimap","version":"24.6.1","name":"Xaero's Minimap"}"#,
            )],
        );
        let r1 = port_jar_with_game_dir(&xaero_jar, &hmods_dir, Some(&game_dir), None);
        assert_eq!(r1.verdict, Verdict::Works);
        assert_eq!(r1.kind, "native_rust");
        assert_eq!(r1.native_mod_id.as_deref(), Some("minimap"));

        // Toggle native mod ON then OFF -> writes `mod.minimap:false` when disabled!
        set_mod_enabled(&mods_txt, Some(&hmods_dir), "minimap", true).unwrap();
        assert!(
            list_native_mods(Some(&mods_txt))
                .iter()
                .find(|m| m.id == "minimap")
                .unwrap()
                .enabled
        );
        set_mod_enabled(&mods_txt, Some(&hmods_dir), "minimap", false).unwrap();
        assert!(
            !list_native_mods(Some(&mods_txt))
                .iter()
                .find(|m| m.id == "minimap")
                .unwrap()
                .enabled
        );

        // Channel 2: Sodium -> builtin_engine
        let sodium_jar = make_jar(
            &dir,
            "sodium-fabric-0.6.5.jar",
            &[(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"sodium","version":"0.6.5","name":"Sodium"}"#,
            )],
        );
        let r2 = port_jar_with_game_dir(&sodium_jar, &hmods_dir, Some(&game_dir), None);
        assert_eq!(r2.verdict, Verdict::Works);
        assert_eq!(r2.kind, "builtin_engine");

        // Channel 4: Worldgen / SkyBlock jar overriding `data/minecraft/worldgen/...` -> datapack
        let worldgen_jar = make_jar(
            &dir,
            "sky-islands-1.21.jar",
            &[
                (
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"id":"sky_islands","version":"2.0","name":"Sky Islands"}"#,
                ),
                (
                    "data/minecraft/worldgen/noise/continentalness.json",
                    br#"{"firstOctave": -9, "amplitudes": [1.0, 1.0, 2.0]}"#,
                ),
                (
                    "data/sky_islands/functions/start.mcfunction",
                    b"say Welcome to Sky Islands!\n",
                ),
            ],
        );
        let r4 = port_jar_with_game_dir(&worldgen_jar, &hmods_dir, Some(&game_dir), None);
        assert_eq!(r4.verdict, Verdict::Works, "{r4:?}");
        assert_eq!(r4.kind, "datapack");
        assert!(
            game_dir
                .join("new-world-datapacks/sky_islands/data/sky_islands/function/start.mcfunction")
                .is_file()
        );
        assert!(
            game_dir
                .join("new-world-datapacks/sky_islands/pack.mcmeta")
                .is_file()
        );

        // Channel 5: Vanilla texture override jar -> resourcepack
        let tex_jar = make_jar(
            &dir,
            "hd-apples.jar",
            &[
                (
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"id":"hd_apples","version":"1.0","name":"HD Apples"}"#,
                ),
                (
                    "assets/minecraft/models/item/apple.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/apple"}}"#,
                ),
            ],
        );
        let r5 = port_jar_with_game_dir(&tex_jar, &hmods_dir, Some(&game_dir), None);
        assert_eq!(r5.verdict, Verdict::Works);
        assert_eq!(r5.kind, "resourcepack");
        assert!(game_dir.join("resourcepacks/hd_apples-rust.zip").is_file());

        // `.mrpack` Modpack with overrides -> `port_modpack`
        let pack_target = dir.join("pack-game");
        let mrpack = make_jar(
            &dir,
            "skyblock-pro.mrpack",
            &[
                (
                    "modrinth.index.json",
                    br#"{
                        "formatVersion": 1,
                        "game": "minecraft",
                        "versionId": "1.0.0",
                        "name": "SkyBlock Pro Rust",
                        "dependencies": { "minecraft": "1.21.11", "fabric-loader": "0.16.9" },
                        "files": []
                    }"#,
                ),
                (
                    "overrides/mods/Xaeros_Minimap_24.6.1_Fabric_1.21.11.jar",
                    &fs::read(&xaero_jar).unwrap(),
                ),
                (
                    "overrides/mods/sky-islands-1.21.jar",
                    &fs::read(&worldgen_jar).unwrap(),
                ),
                ("overrides/config/skyblock.toml", b"islands = true\n"),
            ],
        );
        let pack_report = port_modpack(&mrpack, &pack_target, None, true).unwrap();
        assert_eq!(pack_report.pack_name, "SkyBlock Pro Rust");
        assert_eq!(pack_report.total_mods, 2);
        assert_eq!(pack_report.native_rust_enabled, 1);
        assert_eq!(pack_report.datapacks_created, 1);
        assert!(pack_target.join("config/skyblock.toml").is_file());

        fs::remove_dir_all(&dir).unwrap();
    }
}
