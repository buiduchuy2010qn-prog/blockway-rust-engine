//! Multi-version Minecraft Java Edition profile registry for BlockWay Rust Engine.
//!
//! Allows the Rust engine to target any supported Minecraft release (`1.20.2` through `26.3`)
//! natively via `--mc-version <VER>` (or `BLOCKWAY_MC_VERSION`), supplying authentic Mojang
//! `DataVersion`, protocol version, pack formats, and protocol-registry gating metadata
//! without binary patching.

use std::sync::OnceLock;

/// Exact Mojang metadata for a supported Minecraft Java Edition release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionProfile {
    /// Official version string (`"1.21.1"`, `"1.21.11"`, `"26.3"`, ...).
    pub id: &'static str,
    /// `SharedConstants.getProtocolVersion()`.
    pub protocol_version: i32,
    /// Protocol number used for built-in registry gating (`764..=777`).
    pub registry_protocol: i32,
    /// `WorldVersion.dataVersion()` (`level.dat` `Data.Version.Id`).
    pub data_version: i32,
    /// `PackType.CLIENT_RESOURCES` `(major, minor)`.
    pub resource_pack_format: (i64, i64),
    /// `PackType.SERVER_DATA` `(major, minor)`.
    pub data_pack_format: (i64, i64),
}

impl VersionProfile {
    const fn new(
        id: &'static str,
        protocol_version: i32,
        registry_protocol: i32,
        data_version: i32,
        resource_pack_format: (i64, i64),
        data_pack_format: (i64, i64),
    ) -> Self {
        Self {
            id,
            protocol_version,
            registry_protocol,
            data_version,
            resource_pack_format,
            data_pack_format,
        }
    }

    /// Format `(major, minor)` as Mojang `/version` prints pack versions (`97.1`, `75.0`, ...).
    pub fn format_pack(pack: (i64, i64)) -> String {
        format!("{}.{}", pack.0, pack.1)
    }
}

/// Default profile matching the pinned Horizon 26.3 simulation tables.
pub const DEFAULT_PROFILE: VersionProfile = VersionProfile::new(
    crate::version::VANILLA_VERSION,
    crate::version::PROTOCOL_VERSION,
    crate::version::PROTOCOL_VERSION,
    crate::version::DATA_VERSION,
    crate::version::RESOURCE_PACK_FORMAT,
    crate::version::DATA_PACK_FORMAT,
);

/// Official Minecraft Java Edition release profiles supported natively by BlockWay Rust Engine.
pub const PROFILES: &[VersionProfile] = &[
    DEFAULT_PROFILE,
    VersionProfile::new("26.2", 776, 776, 4890, (90, 0), (114, 0)),
    VersionProfile::new("26.1.2", 775, 775, 4790, (84, 0), (101, 1)),
    VersionProfile::new("26.1.1", 775, 775, 4788, (84, 0), (101, 1)),
    VersionProfile::new("26.1", 775, 775, 4786, (84, 0), (101, 1)),
    VersionProfile::new("1.21.11", 774, 774, 4671, (75, 0), (94, 1)),
    VersionProfile::new("1.21.10", 773, 773, 4556, (69, 0), (88, 0)),
    VersionProfile::new("1.21.9", 773, 773, 4554, (69, 0), (88, 0)),
    VersionProfile::new("1.21.8", 772, 772, 4440, (64, 0), (81, 0)),
    VersionProfile::new("1.21.7", 772, 772, 4438, (64, 0), (81, 0)),
    VersionProfile::new("1.21.6", 771, 771, 4435, (63, 0), (80, 0)),
    VersionProfile::new("1.21.5", 770, 770, 4325, (55, 0), (71, 0)),
    VersionProfile::new("1.21.4", 769, 769, 4189, (46, 0), (61, 0)),
    VersionProfile::new("1.21.3", 768, 768, 4082, (42, 0), (57, 0)),
    VersionProfile::new("1.21.2", 768, 768, 4080, (42, 0), (57, 0)),
    VersionProfile::new("1.21.1", 767, 767, 3955, (34, 0), (48, 0)),
    VersionProfile::new("1.21", 767, 767, 3953, (34, 0), (48, 0)),
    VersionProfile::new("1.20.6", 766, 766, 3839, (32, 0), (41, 0)),
    VersionProfile::new("1.20.5", 766, 766, 3837, (32, 0), (41, 0)),
    VersionProfile::new("1.20.4", 765, 765, 3700, (22, 0), (26, 0)),
    VersionProfile::new("1.20.3", 765, 765, 3698, (22, 0), (26, 0)),
    VersionProfile::new("1.20.2", 764, 764, 3578, (18, 0), (18, 0)),
    VersionProfile::new("1.20.1", 763, 764, 3465, (15, 0), (15, 0)),
    VersionProfile::new("1.20", 763, 764, 3463, (15, 0), (15, 0)),
    VersionProfile::new("1.19.4", 762, 764, 3337, (13, 0), (12, 0)),
    VersionProfile::new("1.19.2", 760, 764, 3120, (9, 0), (10, 0)),
    VersionProfile::new("1.18.2", 758, 764, 2975, (8, 0), (9, 0)),
    VersionProfile::new("1.17.1", 756, 764, 2730, (7, 0), (7, 0)),
    VersionProfile::new("1.16.5", 754, 764, 2586, (6, 0), (6, 0)),
];

static ACTIVE_PROFILE: OnceLock<VersionProfile> = OnceLock::new();

/// Look up an exact `VersionProfile` by Minecraft version string (e.g. `"1.21.11"`).
pub fn lookup(version_id: &str) -> Option<VersionProfile> {
    let clean = version_id.trim();
    PROFILES.iter().copied().find(|p| p.id == clean)
}

/// All officially registered Minecraft version IDs (`26.3` down to `1.16.5`).
pub fn supported_ids() -> Vec<&'static str> {
    PROFILES.iter().map(|p| p.id).collect()
}

/// Set the active runtime Minecraft version profile once at process startup.
pub fn set_active(version_id: &str) -> &'static VersionProfile {
    let trimmed = version_id.trim();
    ACTIVE_PROFILE.get_or_init(|| {
        if trimmed.is_empty() {
            return DEFAULT_PROFILE;
        }
        if let Some(found) = lookup(trimmed) {
            return found;
        }
        // Prefix fallback for custom sub-releases (e.g. "1.21.11-rc1" -> 1.21.11 metadata).
        let base = PROFILES
            .iter()
            .copied()
            .find(|p| trimmed.starts_with(p.id))
            .unwrap_or(DEFAULT_PROFILE);
        let leaked_id: &'static str = Box::leak(trimmed.to_owned().into_boxed_str());
        VersionProfile {
            id: leaked_id,
            ..base
        }
    })
}

/// Return the active runtime Minecraft version profile (defaults to `26.3` when unset).
pub fn active() -> &'static VersionProfile {
    ACTIVE_PROFILE.get_or_init(|| {
        if let Ok(env_ver) = std::env::var("BLOCKWAY_MC_VERSION") {
            let trimmed = env_ver.trim();
            if !trimmed.is_empty() {
                if let Some(found) = lookup(trimmed) {
                    return found;
                }
                let base = PROFILES
                    .iter()
                    .copied()
                    .find(|p| trimmed.starts_with(p.id))
                    .unwrap_or(DEFAULT_PROFILE);
                let leaked_id: &'static str = Box::leak(trimmed.to_owned().into_boxed_str());
                return VersionProfile {
                    id: leaked_id,
                    ..base
                };
            }
        }
        DEFAULT_PROFILE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_resolves_all_key_releases() {
        let p263 = lookup("26.3").unwrap();
        assert_eq!(p263.data_version, 5023);
        assert_eq!(p263.protocol_version, 777);

        let p12111 = lookup("1.21.11").unwrap();
        assert_eq!(p12111.data_version, 4671);
        assert_eq!(p12111.protocol_version, 774);

        let p1211 = lookup("1.21.1").unwrap();
        assert_eq!(p1211.data_version, 3955);
        assert_eq!(p1211.protocol_version, 767);

        let p1202 = lookup("1.20.2").unwrap();
        assert_eq!(p1202.data_version, 3578);
        assert_eq!(p1202.protocol_version, 764);
    }
}
