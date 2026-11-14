# 🦀 BlockWay Multi-Version Ultra-Light Rust Engine (`blockway-rust-engine` v2.1.0)

Nhân **Minecraft: Java Edition viết thuần bằng ngôn ngữ hệ thống Rust (`100% Native Rust`, `0% Binary Patch`)** dành cho **BlockWay Launcher (`v42.4.0 PRO`)**, phát triển mở rộng từ dự án mã nguồn mở **[horizon-rust/horizon](https://github.com/horizon-rust/horizon)** (`Copyright (c) 2026 Ludovic Marie and the Horizon contributors`, [MIT License](LICENSE)).

---

## 🚀 Điểm nâng cấp cốt lõi trong nhánh `blockway-rust-engine` (`v2.1.0`)

1. **⚡ Ultra-Light Memory & Windows Hybrid CPU Tuning (`crates/horizon-threads` & `Cargo.toml`):**
   - **Windows Native `HeapCompact`:** Tự động thu hồi phân mảnh bộ nhớ đệm Win32 / driver Vulkan & DirectX 12 (`GetProcessHeaps` + `HeapCompact`) ngay khi xả bộ nhớ.
   - **`mimalloc` v3 Zero-Retain Purge:** Cấu hình trả trang bộ nhớ vật lý tức thì về hệ điều hành (`PURGE_DECOMMITS=1`, `PAGE_FULL_RETAIN=0`, `PURGE_DELAY=0`, `ARENA_PURGE_MULT=1`).
   - **Siêu nhẹ Đa luồng & Nhị phân:** Rút gọn stack luồng worker xuống **`1 MiB`**, giải phóng bộ đệm nhàn rỗi sau **`3 giây`**, biên dịch release với `lto = "thin"`, `codegen-units = 1`, `strip = "debuginfo"`.
   - **Windows Hybrid `P-Core` & `Anti-EcoQoS`:** Tự động ưu tiên nhân hiệu năng cao (Performance Cores) và tắt cơ chế bóp xung `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` của Windows 11.
2. **🌍 Hỗ trợ Đa phiên bản Minecraft trực tiếp trong Rust (`crates/horizon-core/src/version_profile.rs`):**
   - Tích hợp bảng định danh Mojang cho **29 phiên bản** (`1.16.5` → `1.20.2` → `1.20.4` → `1.21.1` → `1.21.4` → `1.21.11` → `26.1` → `26.2` → `26.3`) qua cờ `--mc-version <VER>` / `BLOCKWAY_MC_VERSION`.
   - Lọc Kho đồ Sáng tạo (Creative Inventory) và lệnh `/give`, `/setblock`, `/summon`, `/effect` chuẩn theo từng phiên bản; bảo vệ chống hạ cấp nhầm `DataVersion` bản đồ (`save.rs`).
3. **📦 Trình chuyển đổi Mod/Datapack 5 kênh & Bảo vệ Worldgen 26.3 (`crates/horizon-port-pack`):**
   - Tiện ích gốc `horizon-port-jar.exe` chuyển đổi gói nội dung `.jar` / `.mrpack` sang `.hmod`, tích hợp **5 tiện ích HUD Rust gốc** (*Minimap, Zoom, Tọa độ, Keystrokes, FPS*) và bộ bảo vệ cấu trúc `density_function` chuẩn Minecraft `26.3` (*Terralith 26.3*).
4. **📜 Minh bạch Bản quyền & Giấy phép ([LICENSE](LICENSE) · [NOTICE.md](NOTICE.md) · [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)):**
   - Phát hành theo giấy phép **MIT License**; giữ nguyên bản quyền tác giả gốc **Ludovic Marie & Horizon contributors** cùng đầy đủ thông báo thư viện bên thứ ba.
