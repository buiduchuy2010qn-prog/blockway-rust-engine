//! Built-in native Rust client mods for BlockWay Rust / Horizon Engine.
//!
//! Linked directly through `crate::mod_api::Hooks` (`entry::main`), so players
//! get 100% native Rust client features (`minimap`, `zoom`, `fps_overlay`,
//! `coords_hud`, `keystrokes`) configurable both from `Options > Mods` in-game
//! and from `<game_dir>/horizon-mods.txt` in BlockWay Launcher.
use crate::mod_api::{
    Corner, Corners, Flow, Hooks, Host, HudBatch, HudEvent, KeyEvent, MinimapEvent, Mods,
    ModsEvent, NativeMinimap, NativeMod, NativeZoom, SettingsClickEvent, SettingsEvent, WheelEvent,
    ZoomEvent,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MOD_IDS: [&str; 5] = [
    "minimap",
    "zoom",
    "fps_overlay",
    "coords_hud",
    "keystrokes",
];

const HUD_BG: u32 = 0x7800_0000;
const KEY_UP_BG: u32 = 0x6800_0000;
const KEY_DOWN_BG: u32 = 0xCCFF_FFFF;
const TEXT_WHITE: u32 = 0x00FF_FFFF;
const TEXT_MUTED: u32 = 0x00BF_C7D5;
const TEXT_ACCENT: u32 = 0x007E_F29D;
const TEXT_DARK: u32 = 0x0014_1820;

const MINIMAP_SIZES: [(f32, &str); 3] = [(64.0, "Small"), (96.0, "Medium"), (128.0, "Large")];
const MINIMAP_ZOOMS: [(f32, &str); 4] = [
    (0.5, "0.5x (Close)"),
    (1.0, "1x (Normal)"),
    (2.0, "2x (Wide)"),
    (4.0, "4x (Far)"),
];
const ZOOM_DEFAULTS: [f32; 4] = [3.0, 4.0, 6.0, 8.0];

/// Instantiates the 5 built-in native Rust mods in `MOD_IDS` order.
pub fn mods() -> Mods {
    let list: Vec<Box<dyn Hooks>> = vec![
        Box::new(MinimapMod),
        Box::new(ZoomMod::default()),
        Box::new(FpsOverlayMod::default()),
        Box::new(CoordsHudMod),
        Box::new(KeystrokesMod::default()),
    ];
    debug_assert_eq!(list.len(), MOD_IDS.len());
    Arc::from(list)
}

fn corner_shown(host: &dyn Host, corner: Corner) -> bool {
    let Some(state) = host.client_state() else {
        return true;
    };
    if state.gui_hidden {
        return false;
    }
    !(state.debug_screen && corner.top())
}

fn cycle_corner(current: Corner) -> Corner {
    let idx = Corner::ALL
        .iter()
        .position(|c| *c == current)
        .unwrap_or(0);
    Corner::ALL[(idx + 1) % Corner::ALL.len()]
}

fn read_corner(host: &dyn Host, mod_id: &str, default: Corner) -> Corner {
    host.setting(mod_id, "corner")
        .map(|s| Corner::parse(s.trim(), default))
        .unwrap_or(default)
}

fn read_bool(host: &dyn Host, mod_id: &str, key: &str, default: bool) -> bool {
    match host.setting(mod_id, key).as_deref().map(str::trim) {
        Some("true" | "1" | "on") => true,
        Some("false" | "0" | "off") => false,
        _ => default,
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "ON" } else { "OFF" }
}

fn draw_panel(
    host: &mut dyn Host,
    batch: &mut HudBatch,
    corners: &mut Corners,
    corner: Corner,
    lines: &[(String, u32)],
) {
    if lines.is_empty() || !corner_shown(host, corner) {
        return;
    }
    let width = lines
        .iter()
        .map(|(t, _)| host.text_width(t))
        .fold(0.0_f32, f32::max)
        + 6.0;
    let height = 10.0 * lines.len() as f32 + 3.0;
    let (x, y) = corners.place(corner, width, height);
    host.fill(batch, [x, y, width, height], HUD_BG);
    for (i, (line, rgb)) in lines.iter().enumerate() {
        host.text(
            batch,
            line,
            [(x + 3.0).round(), (y + 2.0 + 10.0 * i as f32).round()],
            *rgb,
            true,
        );
    }
}

// ---------------------------------------------------------------------------
// 1. Minimap (`minimap`)

struct MinimapMod;

impl MinimapMod {
    fn size_index(host: &dyn Host) -> usize {
        host.setting("minimap", "size")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|i| *i < MINIMAP_SIZES.len())
            .unwrap_or(1)
    }

    fn zoom_index(host: &dyn Host) -> usize {
        host.setting("minimap", "zoom")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|i| *i < MINIMAP_ZOOMS.len())
            .unwrap_or(1)
    }
}

impl Hooks for MinimapMod {
    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: "minimap".into(),
            title: "Rust Minimap 1.0".into(),
            description: "Live terrain minimap with cardinal compass, zoom & coordinates.".into(),
            keys: vec![[
                "zoom_cycle".into(),
                "Cycle Minimap Zoom".into(),
                "key.keyboard.b".into(),
            ]],
        });
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if event.mod_id != "minimap" {
            return;
        }
        let corner = read_corner(host, "minimap", Corner::TopRight);
        let (_, size_label) = MINIMAP_SIZES[Self::size_index(host)];
        let (_, zoom_label) = MINIMAP_ZOOMS[Self::zoom_index(host)];
        let round = read_bool(host, "minimap", "round", false);
        let rotate = read_bool(host, "minimap", "rotate", true);
        let coords = read_bool(host, "minimap", "coords", true);
        event.buttons.push(format!("Corner: {}", corner.label()));
        event.buttons.push(format!("Size: {size_label}"));
        event.buttons.push(format!(
            "Shape: {}",
            if round { "Round" } else { "Square" }
        ));
        event.buttons.push(format!("Zoom: {zoom_label}"));
        event.buttons.push(format!(
            "Orientation: {}",
            if rotate { "Rotate with Player" } else { "North Up" }
        ));
        event.buttons.push(format!("Coordinates Under Map: {}", on_off(coords)));
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        if event.mod_id != "minimap" {
            return;
        }
        match event.button {
            0 => {
                let next = cycle_corner(read_corner(host, "minimap", Corner::TopRight));
                host.set_setting("minimap", "corner", next.name());
            }
            1 => {
                let next = (Self::size_index(host) + 1) % MINIMAP_SIZES.len();
                host.set_setting("minimap", "size", &next.to_string());
            }
            2 => {
                let next = !read_bool(host, "minimap", "round", false);
                host.set_setting("minimap", "round", if next { "true" } else { "false" });
            }
            3 => {
                let next = (Self::zoom_index(host) + 1) % MINIMAP_ZOOMS.len();
                host.set_setting("minimap", "zoom", &next.to_string());
            }
            4 => {
                let next = !read_bool(host, "minimap", "rotate", true);
                host.set_setting("minimap", "rotate", if next { "true" } else { "false" });
            }
            5 => {
                let next = !read_bool(host, "minimap", "coords", true);
                host.set_setting("minimap", "coords", if next { "true" } else { "false" });
            }
            _ => {}
        }
    }

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        if !event.pressed || !host.on("minimap") || !host.playing() {
            return;
        }
        let bind = host.mod_key("minimap", "zoom_cycle", "key.keyboard.b");
        if event.key == bind {
            let next = (Self::zoom_index(host) + 1) % MINIMAP_ZOOMS.len();
            host.set_setting("minimap", "zoom", &next.to_string());
        }
    }

    fn minimap(&self, host: &dyn Host, _event: &mut MinimapEvent) -> Option<NativeMinimap> {
        if !host.on("minimap") || !host.playing() {
            return None;
        }
        let corner = read_corner(host, "minimap", Corner::TopRight);
        if !corner_shown(host, corner) {
            return None;
        }
        let (size, _) = MINIMAP_SIZES[Self::size_index(host)];
        let (blocks_per_pixel, _) = MINIMAP_ZOOMS[Self::zoom_index(host)];
        let rotate = read_bool(host, "minimap", "rotate", true);
        let round = read_bool(host, "minimap", "round", false);
        Some(NativeMinimap {
            corner,
            size,
            blocks_per_pixel,
            rotate,
            round,
        })
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !host.on("minimap") || !host.playing() || !read_bool(host, "minimap", "coords", true) {
            return;
        }
        let Some(p) = host.player() else {
            return;
        };
        let corner = read_corner(host, "minimap", Corner::TopRight);
        let line = format!(
            "{}  {}  {}",
            p.x.floor() as i32,
            p.y.floor() as i32,
            p.z.floor() as i32
        );
        draw_panel(
            host,
            event.batch,
            event.corners,
            corner,
            &[(line, TEXT_WHITE)],
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Smooth Zoom (`zoom`)

#[derive(Default)]
struct ZoomState {
    held: bool,
    scroll_factor: Option<f32>,
}

#[derive(Default)]
struct ZoomMod {
    state: Mutex<ZoomState>,
}

impl ZoomMod {
    fn base_index(host: &dyn Host) -> usize {
        host.setting("zoom", "level")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|i| *i < ZOOM_DEFAULTS.len())
            .unwrap_or(1)
    }
}

impl Hooks for ZoomMod {
    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: "zoom".into(),
            title: "Rust Smooth Zoom 1.0".into(),
            description: "OptiFine / Zoomify smooth camera zoom with mouse-wheel magnification."
                .into(),
            keys: vec![[
                "hold".into(),
                "Hold Zoom".into(),
                "key.keyboard.z".into(),
            ]],
        });
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if event.mod_id != "zoom" {
            return;
        }
        let base = ZOOM_DEFAULTS[Self::base_index(host)];
        let smooth = read_bool(host, "zoom", "smooth", true);
        let scale_turn = read_bool(host, "zoom", "scale_turn", true);
        event
            .buttons
            .push(format!("Default Magnification: {base:.0}x"));
        event
            .buttons
            .push(format!("Smooth Camera Transition: {}", on_off(smooth)));
        event.buttons.push(format!(
            "Scale Mouse Sensitivity: {}",
            on_off(scale_turn)
        ));
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        if event.mod_id != "zoom" {
            return;
        }
        match event.button {
            0 => {
                let next = (Self::base_index(host) + 1) % ZOOM_DEFAULTS.len();
                host.set_setting("zoom", "level", &next.to_string());
            }
            1 => {
                let next = !read_bool(host, "zoom", "smooth", true);
                host.set_setting("zoom", "smooth", if next { "true" } else { "false" });
            }
            2 => {
                let next = !read_bool(host, "zoom", "scale_turn", true);
                host.set_setting("zoom", "scale_turn", if next { "true" } else { "false" });
            }
            _ => {}
        }
    }

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        let bind = host.mod_key("zoom", "hold", "key.keyboard.z");
        if event.key != bind {
            return;
        }
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if event.pressed && host.on("zoom") && host.playing() {
            st.held = true;
        } else {
            st.held = false;
            st.scroll_factor = None;
        }
    }

    fn wheel(&self, host: &mut dyn Host, event: &mut WheelEvent) -> Flow {
        if !host.on("zoom") || !host.playing() || event.steps == 0.0 {
            return Flow::Continue;
        }
        let Ok(mut st) = self.state.lock() else {
            return Flow::Continue;
        };
        if !st.held {
            return Flow::Continue;
        }
        let base = ZOOM_DEFAULTS[Self::base_index(host)];
        let current = st.scroll_factor.unwrap_or(base);
        let factor = (current * 1.15_f32.powf(event.steps)).clamp(1.5, 16.0);
        st.scroll_factor = Some(factor);
        Flow::Cancel
    }

    fn zoom(&self, host: &dyn Host, _event: &mut ZoomEvent) -> Option<NativeZoom> {
        let Ok(mut st) = self.state.lock() else {
            return None;
        };
        if !host.on("zoom") || !host.playing() {
            st.held = false;
            st.scroll_factor = None;
            return None;
        }
        if !st.held {
            return None;
        }
        let base = ZOOM_DEFAULTS[Self::base_index(host)];
        let factor = st.scroll_factor.unwrap_or(base);
        let smooth = read_bool(host, "zoom", "smooth", true);
        let turn = if read_bool(host, "zoom", "scale_turn", true) {
            1.0 / factor.sqrt()
        } else {
            1.0
        };
        Some(NativeZoom {
            factor,
            smooth,
            turn,
        })
    }
}

// ---------------------------------------------------------------------------
// 3. FPS Overlay (`fps_overlay`)

#[derive(Default)]
struct FpsOverlayMod {
    frames: Mutex<VecDeque<Instant>>,
}

impl Hooks for FpsOverlayMod {
    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: "fps_overlay".into(),
            title: "Rust FPS Counter 1.0".into(),
            description: "Lightweight live FPS and frame-pacing (min FPS) HUD counter.".into(),
            keys: Vec::new(),
        });
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if event.mod_id != "fps_overlay" {
            return;
        }
        let corner = read_corner(host, "fps_overlay", Corner::TopLeft);
        let lows = read_bool(host, "fps_overlay", "lows", true);
        event.buttons.push(format!("Corner: {}", corner.label()));
        event
            .buttons
            .push(format!("Show 1s Min FPS: {}", on_off(lows)));
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        if event.mod_id != "fps_overlay" {
            return;
        }
        match event.button {
            0 => {
                let next = cycle_corner(read_corner(host, "fps_overlay", Corner::TopLeft));
                host.set_setting("fps_overlay", "corner", next.name());
            }
            1 => {
                let next = !read_bool(host, "fps_overlay", "lows", true);
                host.set_setting("fps_overlay", "lows", if next { "true" } else { "false" });
            }
            _ => {}
        }
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !host.on("fps_overlay") || !host.playing() {
            return;
        }
        let now = Instant::now();
        let (fps, min_fps) = if let Ok(mut q) = self.frames.lock() {
            let window = Duration::from_secs(1);
            while q.front().is_some_and(|t| now.saturating_duration_since(*t) > window) {
                q.pop_front();
            }
            let max_dt = q
                .iter()
                .zip(q.iter().skip(1))
                .map(|(a, b)| b.saturating_duration_since(*a).as_secs_f32())
                .fold(0.0_f32, f32::max);
            q.push_back(now);
            let count = q.len();
            let low = if max_dt > 0.0001 {
                (1.0 / max_dt).round() as usize
            } else {
                count
            };
            (count, low.min(count))
        } else {
            return;
        };

        let corner = read_corner(host, "fps_overlay", Corner::TopLeft);
        let line = if read_bool(host, "fps_overlay", "lows", true) {
            format!("{fps} FPS (low {min_fps})")
        } else {
            format!("{fps} FPS")
        };
        draw_panel(
            host,
            event.batch,
            event.corners,
            corner,
            &[(line, TEXT_ACCENT)],
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Coordinates & Biome HUD (`coords_hud`)

struct CoordsHudMod;

fn cardinal_from_yaw(yaw: f32) -> &'static str {
    let norm = yaw.rem_euclid(360.0);
    if !(45.0..315.0).contains(&norm) {
        "S (+Z)"
    } else if norm < 135.0 {
        "W (-X)"
    } else if norm < 225.0 {
        "N (-Z)"
    } else {
        "E (+X)"
    }
}

fn pretty_biome(raw: &str) -> String {
    let short = raw.rsplit(':').next().unwrap_or(raw);
    short
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Hooks for CoordsHudMod {
    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: "coords_hud".into(),
            title: "Rust Coords & Biome HUD 1.0".into(),
            description: "MiniHUD-style live XYZ coordinates, cardinal direction and biome panel."
                .into(),
            keys: Vec::new(),
        });
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if event.mod_id != "coords_hud" {
            return;
        }
        let corner = read_corner(host, "coords_hud", Corner::TopLeft);
        let biome = read_bool(host, "coords_hud", "biome", true);
        let facing = read_bool(host, "coords_hud", "facing", true);
        event.buttons.push(format!("Corner: {}", corner.label()));
        event
            .buttons
            .push(format!("Show Biome: {}", on_off(biome)));
        event
            .buttons
            .push(format!("Show Facing Direction: {}", on_off(facing)));
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        if event.mod_id != "coords_hud" {
            return;
        }
        match event.button {
            0 => {
                let next = cycle_corner(read_corner(host, "coords_hud", Corner::TopLeft));
                host.set_setting("coords_hud", "corner", next.name());
            }
            1 => {
                let next = !read_bool(host, "coords_hud", "biome", true);
                host.set_setting("coords_hud", "biome", if next { "true" } else { "false" });
            }
            2 => {
                let next = !read_bool(host, "coords_hud", "facing", true);
                host.set_setting("coords_hud", "facing", if next { "true" } else { "false" });
            }
            _ => {}
        }
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !host.on("coords_hud") || !host.playing() {
            return;
        }
        let Some(p) = host.player() else {
            return;
        };
        let bx = p.x.floor() as i32;
        let by = p.y.floor() as i32;
        let bz = p.z.floor() as i32;
        let mut lines = vec![(format!("XYZ: {bx}, {by}, {bz}"), TEXT_WHITE)];
        if read_bool(host, "coords_hud", "facing", true) {
            lines.push((format!("Facing: {}", cardinal_from_yaw(p.yaw)), TEXT_MUTED));
        }
        if read_bool(host, "coords_hud", "biome", true)
            && let Some(b) = host.biome([bx, by, bz])
        {
            lines.push((format!("Biome: {}", pretty_biome(&b)), TEXT_ACCENT));
        }
        let corner = read_corner(host, "coords_hud", Corner::TopLeft);
        draw_panel(host, event.batch, event.corners, corner, &lines);
    }
}

// ---------------------------------------------------------------------------
// 5. Keystrokes & CPS HUD (`keystrokes`)

#[derive(Default)]
struct KeystrokesState {
    w: bool,
    a: bool,
    s: bool,
    d: bool,
    space: bool,
    lmb: bool,
    rmb: bool,
    left_clicks: VecDeque<Instant>,
    right_clicks: VecDeque<Instant>,
}

impl KeystrokesState {
    fn clear_held(&mut self) {
        self.w = false;
        self.a = false;
        self.s = false;
        self.d = false;
        self.space = false;
        self.lmb = false;
        self.rmb = false;
    }
}

#[derive(Default)]
struct KeystrokesMod {
    state: Mutex<KeystrokesState>,
}

impl Hooks for KeystrokesMod {
    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent<'_>) {
        event.mods.push(NativeMod {
            id: "keystrokes".into(),
            title: "Rust Keystrokes & CPS 1.0".into(),
            description: "PvP WASD, Spacebar and LMB/RMB CPS overlay rendered natively.".into(),
            keys: Vec::new(),
        });
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if event.mod_id != "keystrokes" {
            return;
        }
        let corner = read_corner(host, "keystrokes", Corner::BottomRight);
        let cps = read_bool(host, "keystrokes", "cps", true);
        event.buttons.push(format!("Corner: {}", corner.label()));
        event
            .buttons
            .push(format!("Show LMB/RMB CPS: {}", on_off(cps)));
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        if event.mod_id != "keystrokes" {
            return;
        }
        match event.button {
            0 => {
                let next = cycle_corner(read_corner(host, "keystrokes", Corner::BottomRight));
                host.set_setting("keystrokes", "corner", next.name());
            }
            1 => {
                let next = !read_bool(host, "keystrokes", "cps", true);
                host.set_setting("keystrokes", "cps", if next { "true" } else { "false" });
            }
            _ => {}
        }
    }

    fn key(&self, _host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        let now = Instant::now();
        match event.key {
            "key.keyboard.w" => st.w = event.pressed,
            "key.keyboard.a" => st.a = event.pressed,
            "key.keyboard.s" => st.s = event.pressed,
            "key.keyboard.d" => st.d = event.pressed,
            "key.keyboard.space" => st.space = event.pressed,
            "key.mouse.left" => {
                if event.pressed && !st.lmb {
                    st.left_clicks.push_back(now);
                }
                st.lmb = event.pressed;
            }
            "key.mouse.right" => {
                if event.pressed && !st.rmb {
                    st.right_clicks.push_back(now);
                }
                st.rmb = event.pressed;
            }
            _ => {}
        }
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        if !host.on("keystrokes") {
            return;
        }
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if !host.playing() {
            st.clear_held();
            return;
        }
        let corner = read_corner(host, "keystrokes", Corner::BottomRight);
        if !corner_shown(host, corner) {
            return;
        }

        let now = Instant::now();
        let one_sec = Duration::from_secs(1);
        while st
            .left_clicks
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > one_sec)
        {
            st.left_clicks.pop_front();
        }
        while st
            .right_clicks
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > one_sec)
        {
            st.right_clicks.pop_front();
        }
        let show_cps = read_bool(host, "keystrokes", "cps", true);

        let key_s = 18.0_f32;
        let gap = 2.0_f32;
        let total_w = key_s * 3.0 + gap * 2.0; // 58 px
        let mouse_h = if show_cps { 20.0 } else { 14.0 };
        let space_h = 10.0;
        let total_h = key_s * 2.0 + mouse_h + space_h + gap * 3.0;
        let (ox, oy) = event.corners.place(corner, total_w, total_h);

        let draw_key =
            |host: &mut dyn Host, batch: &mut HudBatch, rect: [f32; 4], label: &str, down: bool| {
                let (bg, fg) = if down {
                    (KEY_DOWN_BG, TEXT_DARK)
                } else {
                    (KEY_UP_BG, TEXT_WHITE)
                };
                host.fill(batch, rect, bg);
                let tw = host.text_width(label);
                let tx = (rect[0] + (rect[2] - tw) * 0.5).round();
                let ty = (rect[1] + (rect[3] - 8.0) * 0.5).round();
                host.text(batch, label, [tx, ty], fg, !down);
            };

        draw_key(
            host,
            event.batch,
            [ox + key_s + gap, oy, key_s, key_s],
            "W",
            st.w,
        );
        let row2_y = oy + key_s + gap;
        draw_key(host, event.batch, [ox, row2_y, key_s, key_s], "A", st.a);
        draw_key(
            host,
            event.batch,
            [ox + key_s + gap, row2_y, key_s, key_s],
            "S",
            st.s,
        );
        draw_key(
            host,
            event.batch,
            [ox + (key_s + gap) * 2.0, row2_y, key_s, key_s],
            "D",
            st.d,
        );

        let row3_y = row2_y + key_s + gap;
        let half_w = (total_w - gap) * 0.5;
        let lmb_label = if show_cps {
            format!("L {}", st.left_clicks.len())
        } else {
            "LMB".into()
        };
        let rmb_label = if show_cps {
            format!("R {}", st.right_clicks.len())
        } else {
            "RMB".into()
        };
        draw_key(
            host,
            event.batch,
            [ox, row3_y, half_w, mouse_h],
            &lmb_label,
            st.lmb,
        );
        draw_key(
            host,
            event.batch,
            [ox + half_w + gap, row3_y, half_w, mouse_h],
            &rmb_label,
            st.rmb,
        );

        let row4_y = row3_y + mouse_h + gap;
        draw_key(
            host,
            event.batch,
            [ox, row4_y, total_w, space_h],
            "---",
            st.space,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_api::{NativeClientState, NativeScreenKind};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockHost {
        playing: bool,
        enabled: BTreeMap<String, bool>,
        settings: Mutex<BTreeMap<(String, String), String>>,
    }

    impl Host for MockHost {
        fn on(&self, mod_id: &str) -> bool {
            self.enabled.get(mod_id).copied().unwrap_or(true)
        }
        fn playing(&self) -> bool {
            self.playing
        }
        fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
            self.settings
                .lock()
                .expect("settings lock")
                .get(&(mod_id.to_owned(), key.to_owned()))
                .cloned()
        }
        fn set_setting(&self, mod_id: &str, key: &str, value: &str) {
            self.settings
                .lock()
                .expect("settings lock")
                .insert((mod_id.to_owned(), key.to_owned()), value.to_owned());
        }
        fn mod_key(&self, _mod_id: &str, _name: &str, default: &str) -> String {
            default.to_owned()
        }
        fn client_state(&self) -> Option<NativeClientState> {
            let mut st = NativeClientState::default();
            st.focused = true;
            st.screen = NativeScreenKind::Playing;
            Some(st)
        }
    }

    #[test]
    fn registers_five_native_mods_and_handles_minimap_and_zoom() {
        let built = mods();
        assert_eq!(built.len(), MOD_IDS.len());

        let mut host = MockHost {
            playing: true,
            ..Default::default()
        };
        let mut listed = Vec::new();
        crate::mod_api::dispatch_mods(&mut host, &built, &mut listed);
        let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, MOD_IDS);

        // Minimap active when playing & enabled; cycles size via settings_click.
        let mut map = None;
        crate::mod_api::dispatch_minimap(&host, &built, &mut map);
        assert_eq!(map.expect("minimap on").size, 96.0);
        crate::mod_api::dispatch_settings_click(&mut host, &built, "minimap", 1);
        let mut resized = None;
        crate::mod_api::dispatch_minimap(&host, &built, &mut resized);
        assert_eq!(resized.expect("minimap resized").size, 128.0);

        // Zoom activates on hold key and scales with scroll wheel.
        let mut zoom = None;
        crate::mod_api::dispatch_zoom(&host, &built, &mut zoom);
        assert!(zoom.is_none());
        crate::mod_api::dispatch_key(&mut host, &built, "key.keyboard.z", true);
        crate::mod_api::dispatch_zoom(&host, &built, &mut zoom);
        let initial_zoom = zoom.expect("zoom active");
        assert!(initial_zoom.factor >= 3.9);
        let mut claimed = false;
        crate::mod_api::dispatch_wheel(&mut host, &built, 2.0, &mut claimed);
        assert!(claimed);
        let mut scrolled_zoom = None;
        crate::mod_api::dispatch_zoom(&host, &built, &mut scrolled_zoom);
        assert!(scrolled_zoom.expect("zoom scrolled").factor > initial_zoom.factor);
    }
}
