# 🦀 BlockWay Multi-Version Rust Engine (`blockway-rust-engine`)

Nhân **Minecraft Java Edition viết thuần bằng ngôn ngữ hệ thống Rust (`100% Native Rust`, `0% Binary Patch`)** dành riêng cho **BlockWay Launcher**, phát triển dựa trên nền tảng mã nguồn mở MIT `horizon-rust/horizon` (Vulkan / DirectX 12).

## Điểm nâng cấp cốt lõi trong mã nguồn Rust (`.rs`)

1. **Hỗ trợ đa phiên bản Minecraft trực tiếp trong Rust (`crates/horizon-core/src/version_profile.rs`):**
   - Tích hợp bảng định danh chính thức của Mojang cho **29 phiên bản** (`1.16.5` → `1.20.2` → `1.20.4` → `1.21.1` → `1.21.4` → `1.21.11` → `26.1` → `26.2` → `26.3`): `DataVersion`, `ProtocolVersion`, `ResourcePackFormat`, `DataPackFormat`.
   - Cờ khởi chạy gốc `--mc-version <VER>` (hoặc biến môi trường `BLOCKWAY_MC_VERSION`) trong `launch_api.rs` & `launch_options.rs`.
2. **Lọc Kho đồ Sáng tạo (Creative Inventory), Tìm kiếm & Lệnh `/give`, `/setblock`, `/summon`, `/effect` theo đúng phiên bản (`creative.rs`, `game/chat.rs`):**
   - Đối chiếu trực tiếp bảng `registries.json` + `horizon_protocol::adapter::ServerVersion::ids()` của Mojang để chỉ hiển thị đúng các khối, vật phẩm, sinh vật và hiệu ứng thực sự tồn tại ở phiên bản Minecraft đang chọn (ví dụ bản `1.21.1` ẩn vật phẩm `1.21.4`/`26.x`; bản `1.21.11` có Copper Golem/Shelves/Spears nhưng ẩn vật phẩm `26.x`; bản `26.3` mở toàn bộ).
3. **Bảo vệ chống lệch phiên bản bản đồ & Lưu đúng `DataVersion` gốc (`save.rs`, `title.rs`):**
   - `Level::save` ghi trực tiếp `Version: { Name, Id }` chuẩn theo `--mc-version` trong mã Rust.
   - `downgraded_from` cảnh báo đỏ chuẩn Mojang và yêu cầu sao lưu nếu mở nhầm thế giới đời cao hơn (như map `26.3` mở ở `1.21.11`).
4. **Mở khóa sẵn kết nối Đa máy chủ / `playit.gg` (`net.rs`):**
   - Bộ chuyển đổi giao thức mạng đa phiên bản (`1.20.2` – `26.3`) được bật mặc định ngay trong Rust.
5. **Tự động biên dịch & Tự động nâng cấp trên GitHub Actions:**
   - `.github/workflows/build-rust-core.yml`: Tự động biên dịch mã máy `horizon-client.exe`, `horizon-prepare.exe`, `horizon-server.exe`, `horizon-ctl.exe` trên máy chủ GitHub và phát hành gói `horizon-windows-x64.zip` + `blockway-rust-manifest.json` cho BlockWay Launcher tự tải/nâng cấp.
   - `.github/workflows/auto-sync-upstream.yml`: Tự động đồng bộ các cải tiến mới hàng ngày, kiểm tra biên dịch Rust và phát hành bản nâng cấp mới cho Launcher.
