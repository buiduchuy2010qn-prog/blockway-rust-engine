//! Direct `.jar` -> `.hmod` native Rust converter for standalone Java mods
//! (`1.16.5` through `26.3`, NeoForge / Forge / Fabric / Quilt).
//!
//! Carries a Java mod jar's content (`assets/` and `data/`), upgrades legacy
//! pre-1.21 plural data directories and pre-1.20.5 recipe result shapes into
//! Horizon's 26.3 format, generates a Creative Mode tab when the mod declares
//! blocks or items, validates the result with `horizon_mods::Content::read`,
//! and packs an atomic `<mod_id>.hmod` archive ready for `horizon-client`.
use crate::{carry, check::Verdict, classes, jar, pipeline};
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
    path::{Path, PathBuf},
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

fn is_valid_mod_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && !RESERVED_IDS.contains(&s)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Picks the best `[a-z0-9_]{1,64}` mod id for carrying `names`: keeps
/// `sanitize_mod_id(raw_id)` whenever the jar already holds `assets/<id>/` or
/// `data/<id>/`, and otherwise falls back to the jar's primary content
/// namespace when unambiguous.
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

/// Upgrades pre-1.20.5 recipes left in `recipes_kept` inside `carry_dir`:
/// - `"result": {"item": "<id>", "count": N}` -> `"result": {"id": "<id>", "count": N}`
/// - `minecraft:stonecutting` with bare string `"result"` and top-level `"count"`
/// - `minecraft:smelting`/`blasting`/`smoking`/`campfire_cooking` with bare string `"result"`
/// - Strips loader condition keys on vanilla `minecraft:*` recipe types so
///   `carry::upgrade_recipe` can rewrite `1.16.5`–`1.20.4` ingredients (`{"item": ...}`, `{"tag": ...}`).
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
                let kind = recipe["type"].as_str().unwrap_or("").to_owned();
                if !kind.starts_with("minecraft:") {
                    continue;
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

                // 1.16.5 - 1.20.4 stonecutting: `"result": "ns:id", "count": 2` at top level.
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

                // 1.16.5 - 1.20.4 object result: `{"item": "ns:id", "count": 4, "data": 0}`
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

                changed |= carry::renamed_ids(&mut recipe);
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

/// Writes `data/<id>/horizon/creative_tab/main.json` when the carried mod adds
/// blocks (with items) or standalone items and has no creative tab yet, so
/// converted `.jar` items appear directly in Horizon's Creative Inventory.
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

    // Verify the written `.hmod` archive opens cleanly with Horizon's own reader.
    let verified_pkg = Package::read(&final_path)?;
    let verified_content = Content::read(&final_path, &verified_pkg.manifest)?;
    Ok((final_path, verified_content))
}

/// Converts a single Java `.jar` mod into `<out_dir>/<mod_id>.hmod` when it
/// contains portable content (blocks, items, recipes, loot tables, structures,
/// worldgen, tags, textures). Pure-code JVM mods return `Verdict::No` cleanly
/// without writing an empty `.hmod`.
pub fn port_jar_to_hmod(
    jar_path: &Path,
    out_dir: &Path,
    resources: Option<&Path>,
) -> PortedMod {
    let file_name = jar_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| jar_path.display().to_string());
    let mut base = PortedMod {
        jar_path: jar_path.display().to_string(),
        file_name,
        mod_id: String::new(),
        original_mod_id: String::new(),
        name: String::new(),
        version: String::new(),
        loader: String::new(),
        license: String::new(),
        legacy_mc_layout: false,
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
            base.summary = format!("Unreadable jar: {e}");
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
    let names: Vec<String> = match zip
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

    info.mod_id = resolve_mod_id(&info.mod_id, &names);
    if info.version.trim().is_empty() {
        info.version = "1.0.0".into();
    }
    if info.name.trim().is_empty() {
        info.name = info.mod_id.clone();
    }

    base.mod_id = info.mod_id.clone();
    base.name = info.name.clone();
    base.version = info.version.clone();
    base.license = info.license.clone();
    base.loader = detect_loader(&names).to_owned();
    base.legacy_mc_layout = carry::old_layout(&names);

    // Bytecode census (same 16 MiB per class & 1 GiB archive budget as `check::check`).
    let mut budget = crate::entry::Budget::archive();
    let mut class_bytes = Vec::new();
    for entry_name in names.iter().filter(|n| n.ends_with(".class")) {
        let Ok(entry) = zip.by_name(entry_name) else {
            continue;
        };
        let declared = entry.size();
        match budget.read(entry, declared, (1 << 24) - 1) {
            Ok(b) => class_bytes.push(b),
            Err(e) if crate::entry::is_too_big(&e) && declared < 1 << 24 => {
                let msg = format!("{}: classes unpack past 1 GiB limit", jar_path.display());
                base.summary = msg.clone();
                base.error = Some(msg);
                return base;
            }
            Err(_) => {}
        }
    }
    let code = classes::census(class_bytes.iter().map(Vec::as_slice));

    let work_dir = temp_work_dir(&info.mod_id);
    let vanilla = vanilla_blocks_from_resources(resources);
    let allowed = |block_id: &str| vanilla.as_ref().is_none_or(|set| set.contains(block_id));
    let alone = |m: &str| m == info.mod_id || m == "minecraft";
    let pack = carry::Pack {
        loaded: &alone,
        mods: &alone,
    };

    let mut report = match carry::carry(
        jar_path,
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
        let _ = fs::remove_dir_all(&work_dir);
        let reason = if code.behaviour() || code.classes > 0 {
            "Pure Java code/Mixin mod (no custom blocks, items, recipes or worldgen to pack into .hmod)."
        } else {
            "No portable blocks, items, recipes, worldgen or assets found in jar."
        };
        base.verdict = Verdict::No;
        base.verdict_label = Verdict::No.label().to_owned();
        base.summary = reason.to_owned();
        return base;
    }

    match pack_carried_dir_to_hmod(&carry_dir, out_dir) {
        Ok((hmod_path, packed_content)) => {
            let _ = fs::remove_dir_all(&work_dir);
            let size = fs::metadata(&hmod_path).map(|m| m.len()).unwrap_or(0);
            base.blocks = packed_content.blocks.len();
            base.items = packed_content.items.len();
            base.creative_tabs = packed_content.creative_tabs.len();
            base.files_packed = packed_content.files.len();
            base.namespaces = packed_content.namespaces;
            base.hmod_path = Some(hmod_path.display().to_string());
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

/// Updates `mod.<mod_id>:true` (or removes it when `enabled == false`) in
/// `horizon-mods.txt`. When `mods_dir` is provided and `enabled == true`, checks
/// for namespace clashes against other currently-enabled `.hmod` packages; if
/// another enabled mod shares a namespace, keeps `mod_id` disabled and returns
/// `Ok(Some(clash_reason))`.
pub fn set_mod_enabled(
    mods_txt: &Path,
    mods_dir: Option<&Path>,
    mod_id: &str,
    enabled: bool,
) -> Result<Option<String>, String> {
    let mut state = ModsTxtState::load(mods_txt);
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

    // Annotate active namespace clashes among enabled mods.
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

        // 1) Legacy 1.20.1 Fabric jar with hyphenated mod id, plural `recipes/` + `loot_tables/`,
        //    pre-1.20.5 `{"item": ..., "count": 4}` crafting result, and stonecutting top-level `count`.
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
        let item_model = br#"{"parent": "minecraft:item/generated", "textures": {"layer0": "ruby_gems:item/ruby"}}"#;
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
        assert_eq!(ported.mod_id, "ruby_gems");
        assert!(ported.legacy_mc_layout);
        assert_eq!(ported.blocks, 1);
        assert_eq!(ported.items, 1);
        assert_eq!(ported.recipes, 2);
        assert_eq!(ported.creative_tabs, 1);
        let hmod_path = PathBuf::from(ported.hmod_path.as_ref().unwrap());
        assert!(hmod_path.is_file());

        // Verify Horizon's own Package + Content reader loads the `.hmod` cleanly.
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

        // Toggle on in horizon-mods.txt and list installed hmods.
        let clash = set_mod_enabled(&mods_txt, Some(&hmods_dir), "ruby_gems", true).unwrap();
        assert!(clash.is_none());
        let listed = list_installed_hmods(&hmods_dir, Some(&mods_txt)).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].enabled);
        assert_eq!(listed[0].blocks, 1);
        assert_eq!(listed[0].items, 1);

        // 2) Pure Java code jar returns Verdict::No without creating an `.hmod`.
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
        assert!(no_port.hmod_path.is_none());
        assert!(!hmods_dir.join("fastmath.hmod").exists());

        fs::remove_dir_all(&dir).unwrap();
    }
}
