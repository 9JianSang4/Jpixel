# Spike 开发机基线（参考）

> 这是开发机上的 **bench 基线 + 用户验收结论**。
> 测试日期：2026-10-21（最终版：双缓冲 + 窗口级热键 + 抓屏坐标修复 + ClipCursor + 重按热键重抓）；Windows 11；两块屏：1920×1200@125% + 1920×1080@100%。
> C1–C4 排列、副屏黑边、C1 `Area size is invalid`、光标限制、H03 重按热键均已由用户复测通过。

## 构建

- 源码：`src-tauri/src/bin/spike/`
- 构建配置：`[profile.spike]`（`opt-level = 3`，继承 release 的 LTO/strip/panic=abort）
- 命令：
  ```
  cargo build --bin spike --profile spike --offline --locked
  ```
- 产物：`src-tauri/target/spike/spike.exe`

## 基线结果（10 次/屏）

| 显示器 | 首帧 p50 | 首帧 p95 | 首帧最大 | 剪贴板 p50 | 剪贴板 p95 | 峰值内存 |
|---|---:|---:|---:|---:|---:|---:|
| 1920×1200 @125% | 80.2ms | 88.5ms | 88.5ms | 3.0ms | 4.9ms | 64MB |
| 1920×1080 @100% | 69.2ms | 73.5ms | 73.5ms | 6.1ms | 11.0ms | 61MB |

- 空闲内存：24MB（预热后）
- 像素校验：20/20 次 `pixel_match=true`，`max_diff=0`
- 启动预热：85–109ms（首次进程更慢）
- 预热后首帧不再出现 120ms 级别的冷启动尖峰

### 20 次稳定性（1920×1200 @125%）

- 首帧 p95：91.5ms
- 内存：64MB → 56MB，无持续增长（最大值 64MB）
- 像素校验：20/20 `pixel_match=true`，`max_diff=0`
- 焦点：每次 `set_foreground=true`、`set_focus=true`，`foreground == target`

## 解读

- **性能方向成立**：预热后 p95 在 73.5–88.5ms，低于 100ms 目标；剪贴板 p95 在 11ms 以内。
- **内存方向成立**：空闲 24MB，单次峰值 61–64MB，远低于 150MB 目标（双缓冲相比上一版多约 9MB）。
- **像素方向成立**：20/20 次回读一致，`max_diff=0`。
- **热键消息循环已修复**：`RegisterHotKey(None, ...)` 的 `WM_HOTKEY` 是线程消息，现在在消息循环里直接处理；通过线程消息注入已验证 `[hotkey] received` 和 `[capture]` 都会出现。
- **副屏黑边 / C1 `Area size is invalid` 已修复并复测通过**：`screenshots::capture_area(x, y, w, h)` 的 `x/y` 是相对该显示器的本地坐标，并且内部会再乘一次 `scale_factor`；之前传了全局坐标，导致副屏偏移、黑边和无效区域。现已改为 `capture_area_ignore_area_check(0, 0, 物理宽, 物理高)`。
- **拖动时鼠标限制**：`ClipCursor` 已确认可用，选区不会拖出当前显示器；松手/Esc/失去捕获时释放。
- **遮罩打开时重按热键**：已实现“关闭旧遮罩 → 按当前鼠标/焦点所在屏重新抓取”，不会叠加两个遮罩；已确认可用。
- **裸 F 键被当前机器拦截**：F1 会打开 Windows 帮助，F8 无反应；屏幕键盘 F1 同样失败，但 `Ctrl+F1` 和 `Ctrl+Alt+J` 都正常。结论是系统/键盘软件层拦截裸 F 键，不是热键代码或键盘硬件问题。正式版默认热键改为 **Alt+X**（普通组合键，已验证注册成功），并在设置中允许修改。
- **仍需人工验证（产品级 T2/T3）**：Win10、150%/200%、三屏、旋转屏、RDP、HDR、全屏游戏、睡眠/唤醒、显示器热插拔。
- **注意**：release 默认 `opt-level = "z"` 会把首帧拖到 140–150ms；spike 使用单独的 `spike` profile（`opt-level = 3`）。正式实现必须使用速度优先配置，而不是直接用现有 release 配置。

## 结论

**Go**：Stage 1 通过。原生遮罩、混合 DPI 多屏、抓屏坐标、剪贴板、热键、光标限制、重抓切换均已在真实环境验证；性能 p95 73.5–88.5ms、空闲 24MB、峰值 61–64MB。进入 Phase 2 正式架构与实现。
