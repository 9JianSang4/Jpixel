# Jpixel Phase 2 架构与实施计划（B+）

> 状态：待确认。Stage 1 spike 已 **Go**。
> 目标：在保持“极致轻快、内存小、安装包 ≤10MB”的前提下，把 spike 的能力产品化，并保持代码可维护、可测试。

## 1. 目标与非目标

### v1 目标

- 区域截图：热键唤出遮罩，拖框，复制/保存/贴图。
- 全屏截图：当前焦点窗口所在显示器。
- 多显示器 + 混合 DPI：物理像素坐标，PerMonitorV2。
- 标注：画笔、橡皮、撤销/重做。
- 贴图：置顶、拖动、滚轮缩放、Ctrl+滚轮透明度、右键关闭。
- 托盘、单实例、开机自启、可配置热键。
- 设置页按需打开，关闭后释放 WebView。
- 安装包 ≤10MB，空闲内存目标 ≤40MB，首帧 p95 ≤100ms，复制 p95 ≤60ms。

### v1 非目标

- 跨屏框选（明确不做）。
- 文字/箭头/矩形/马赛克/ OCR。
- 云同步、账号、上传、网络请求。
- 自动更新（可后置）。
- 裸 F 键的键盘钩子兼容模式（后置为高级选项）。

## 2. 方案对比

| 方案 | 做法 | 优点 | 缺点 | 结论 |
|---|---|---|---|---|
| A. 全 Tauri/WebView 遮罩 | 截图/遮罩/工具栏都用 Vue | 开发快、UI 灵活 | 首帧慢、内存高、透明/置顶/焦点问题多 | 不采用 |
| **B+. Rust 原生热路径 + Tauri 设置页** | 截图/遮罩/标注/贴图原生；设置页按需创建 | 快、内存低、可控 | 原生代码量较大 | **采用** |
| C. 纯 Win32 | 全部原生，无 WebView | 最小最快 | 设置页/安装/维护成本高 | 不采用 |

## 3. 总体架构

```
┌─────────────────────────────────────────────────────────┐
│ Tauri 主线程（事件循环 / 托盘 / 单实例 / 设置窗口）        │
│  app::tray  app::singleton  app::commands  settings       │
└───────────────┬───────────────────────────────┬─────────┘
                │ OverlayCommand (mpsc)          │ PinCommand
                ▼                                ▼
┌──────────────────────────────┐   ┌──────────────────────────┐
│ Overlay 线程（Win32 消息循环）│   │ Pin 线程（Win32 消息循环）│
│  hotkey / overlay window     │   │  layered windows          │
│  capture / selection / draw  │   │  drag / zoom / opacity    │
│  toolbar / annotation        │   │  close / cleanup          │
└───────────────┬──────────────┘   └──────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────────────────────┐
│ core（无 UI 依赖，可单测）                                │
│  geometry  monitor  capture  clipboard  config  hotkey   │
│  image_ops  paths  logging                               │
└─────────────────────────────────────────────────────────┘
```

关键原则：

- **热路径不经过 WebView**：热键、抓屏、遮罩、标注、贴图全在 Rust/Win32。
- **设置页按需创建**：打开时创建 Tauri 窗口，关闭时销毁，不常驻 WebView。
- **core 不依赖 Win32 窗口**：几何、配置、裁剪、路径、热键解析等可单测。
- **线程隔离**：遮罩与贴图各自有消息循环，避免阻塞 Tauri 事件循环。

## 4. 目录结构（建议）

```
src-tauri/src/
  main.rs                 # 入口，极薄
  lib.rs                  # Tauri builder / 模块装配
  app/
    mod.rs                # AppContext、启动流程
    commands.rs           # Tauri invoke 命令（只做参数校验 + 转发）
    tray.rs               # 托盘菜单
    singleton.rs          # 单实例与二次启动转发
    autostart.rs          # HKCU Run 键
  core/
    mod.rs
    geometry.rs           # Point/Rect/Size，clamp、normalize、DPI 换算
    monitor.rs            # 枚举、目标屏、显示器 ID
    capture.rs            # CaptureService trait + screenshots 实现
    clipboard.rs          # ClipboardService trait + arboard 实现
    config.rs             # 版本化配置、校验、迁移、原子写
    hotkey.rs             # 热键解析、注册/冲突/回滚
    image_ops.rs          # 裁剪、合成、PNG 编码
    paths.rs              # app data / temp / 文件名校验
    logging.rs            # 日志与脱敏
  overlay/
    mod.rs                # OverlayController（线程入口 / 命令队列）
    window.rs             # 窗口创建、定位、焦点、显示/隐藏
    render.rs             # GDI 双缓冲、暗区、边框、工具栏
    input.rs              # 鼠标/键盘状态机
    selection.rs          # 选区归一化、最小尺寸、命中测试
    toolbar.rs            # 按钮布局、命中、边缘翻转
    annotation.rs         # 矢量笔迹、橡皮、撤销/重做
    capture_flow.rs       # 抓屏 -> 遮罩 -> 输出 的编排
  pin/
    mod.rs                # PinManager（线程入口 / 命令队列）
    window.rs             # 分层窗口、拖动、缩放、透明度
  settings/
    mod.rs                # 设置窗口生命周期
    commands.rs           # 配置读写、热键测试、恢复默认
  bin/spike/              # Stage 1 探针，产品化后删除或移到 examples
```

前端（`src/`）只保留设置页：

```
src/
  main.ts
  App.vue                 # 设置页壳
  views/Settings.vue
  api/ipc.ts              # invoke 封装 + 类型
  utils/hotkey.ts         # 录制/展示
```

## 5. 核心模块职责

### core::geometry

- 全部内部坐标使用**物理像素**。
- `Rect::normalize()`、`clamp_to(bounds)`、`min_size()`、`contains()`。
- 只做纯计算，无 Win32 依赖，单测覆盖率目标 >95%。

### core::monitor

- `enumerate() -> Vec<MonitorInfo>`，包含物理 rect、work area、DPI、primary、稳定 ID。
- `target_for(foreground, cursor) -> MonitorInfo`：
  - 前台窗口属于本进程/桌面/任务栏 → 用鼠标所在屏；
  - 否则用前台窗口所在屏；
  - 最后回退主屏。
- `MonitorInfo` 不持有 HWND 以外的最小状态。

### core::capture

```rust
pub trait CaptureService: Send + Sync {
    fn capture_monitor(&self, monitor: &MonitorInfo) -> Result<RgbaImage>;
}
```

- 实现：`ScreenshotsCapture`，内部固定用 `capture_area_ignore_area_check(0, 0, 物理宽, 物理高)`。
- 禁止再调用 `capture_area(global_x, global_y, ...)`；这是 Stage 1 的实际 bug。
- 可替换为 DXGI 直连实现，不影响上层。

### core::clipboard

```rust
pub trait ClipboardService: Send + Sync {
    fn copy_image(&self, image: &RgbaImage) -> Result<()>;
}
```

- `ArboardClipboard`，失败时有限重试（例如 3 次、20/50/100ms）。
- 失败返回语义化错误，不 panic。

### core::config

- 文件：`%APPDATA%\Jpixel\config.json`（或 Tauri app_config_dir）。
- `Config { version, hotkey, default_action, autostart, annotation, pin, ... }`。
- 读取：缺失字段用默认值；版本迁移；损坏时备份并回退默认。
- 写入：临时文件 + `rename` 原子替换；失败不破坏旧配置。
- 所有外部输入校验：热键格式、颜色、宽度范围、路径。

### core::hotkey

- `Hotkey { vk, mods }`，解析 `Alt+X` / `Ctrl+F1` / `Ctrl+Alt+J`。
- 注册在 overlay 线程的窗口句柄上：`RegisterHotKey(hwnd, id, mods | MOD_NOREPEAT, vk)`。
- 失败：返回错误，设置页回滚 UI，旧热键继续有效。
- 默认 `Alt+X`；裸 F 键不默认，后续可用 `HotkeyBackend` trait 扩展低级钩子。

### overlay

- 一个时刻只有一个 overlay，一个线程、一个窗口。
- 状态机：`Idle -> Capturing -> Selecting -> Selected -> Drawing/Dragging -> Output`。
- 选择/绘制全在物理像素；GDI 双缓冲一次 blit。
- 工具栏作为同一窗口内的绘制区域（v1），按钮命中用矩形；靠近边缘自动翻转。
- 标注为矢量 `Stroke { tool, color, width, points }`；橡皮按“从原图恢复”实现；撤销/重做操作栈。
- 输出：base + strokes 合成 -> 裁剪 -> 复制/保存/贴图；绝不再抓一次屏。

### pin

- `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`。
- `UpdateLayeredWindow` 提交 32bpp premultiplied ARGB DIB；`BLENDFUNCTION` 控制整体透明度。
- 拖动、滚轮缩放（0.5–3.0x）、Ctrl+滚轮透明度（0.1–1.0）、右键关闭。
- 每个 pin 一个窗口，PinManager 统一清理；显示器变化时只做位置修正，不崩溃。

### app / tray / settings

- 托盘：截图、设置、退出；图标与菜单轻量。
- 单实例：第二个实例把 `capture` / `settings` 转发给第一个实例后退出。
- 开机自启：HKCU `Run`，无需管理员；关闭时删除。
- 设置窗口：打开时 `WebviewWindowBuilder` 创建，关闭时 `destroy()`；不 hide。
- Tauri 命令只做：参数校验 -> 调用 core/controller -> 返回结构化错误；不写业务逻辑。

## 6. 数据流

### 区域截图

```
热键消息
  -> overlay 线程解析目标显示器
  -> CaptureService 抓整屏物理像素
  -> 创建/定位 overlay 窗口（该显示器）
  -> 显示冻结画面 + 暗区
  -> 鼠标框选（ClipCursor 限制当前屏）
  -> 工具栏/标注
  -> 输出（复制/保存/贴图）
  -> 关闭 overlay，恢复焦点
```

### 设置变更

```
Vue 设置页
  -> invoke("update_config", patch)
  -> settings::commands 校验
  -> 若热键变化：overlay 线程试注册
      成功 -> 写 config（原子）
      失败 -> 回滚旧热键，返回错误
  -> 前端显示成功/失败
```

## 7. 线程与消息循环

| 线程 | 职责 | 生命周期 |
|---|---|---|
| Tauri 主线程 | 事件循环、托盘、设置窗口、单实例 | 全程 |
| Overlay 线程 | 热键、遮罩窗口、抓屏、标注、输出 | 全程（空闲时阻塞在 GetMessage） |
| Pin 线程 | 所有贴图窗口 | 全程（空闲时阻塞） |
| 设置 WebView | 设置 UI | 按需创建/销毁 |

- 线程间用 `std::sync::mpsc` 发送命令；窗口句柄只在所属线程使用。
- overlay 线程注册热键，`WM_HOTKEY` 直接进窗口过程；不使用 `AttachThreadInput`。
- 抓屏在 overlay 线程同步执行（约 40–70ms），此时遮罩尚未显示；可接受。若后续需要，再拆 capture worker。

## 8. 关键设计决策

| 决策 | 推荐 | 备选 | 理由 |
|---|---|---|---|
| 遮罩渲染 | **GDI 双缓冲** | Direct2D | spike 已验证；依赖少、首帧稳；工具栏/文字需求 v1 足够 |
| 标注模型 | **矢量笔迹** | 位图叠加 | 撤销/重做质量高；缩放/裁剪不糊；内存可控 |
| 贴图窗口 | **分层窗口 UpdateLayeredWindow** | DirectComposition | 实现简单、兼容性好；v1 足够 |
| 保存对话框 | **原生 Win32 IFileSaveDialog** | Tauri dialog 插件 | 少一个依赖、少一份体积；原生体验 |
| 热键后端 | **RegisterHotKey** | 低级键盘钩子 | 无 AV 风险；默认 Alt+X；裸 F 键后置 |
| 设置页 | **现有 Vue 3 + Tauri 按需窗口** | 原生设置 | 复用现有 UI；关闭即释放 |
| 安装包 | **NSIS** | MSI | 体积小、Tauri 默认支持；自定义页足够 |
| 自动更新 | **v1 不做** | Tauri updater | 先保证体积/稳定；后续加签名更新 |

## 9. 配置、安全与隐私

- 配置路径固定，拒绝任意路径写入；保存图片只允许用户通过系统对话框选择。
- 不做任何网络请求；不内置遥测。
- 日志不记录图像内容、剪贴板内容、完整用户路径；错误信息脱敏。
- 所有 Tauri 命令参数做类型/范围校验；错误用 `thiserror` 语义化返回。
- 临时文件如产生，退出时清理；文件权限仅当前用户。
- 未来若做 `read_image`/文件读取，必须白名单目录 + 大小/像素上限。

## 10. 性能预算

| 指标 | 目标 | 验收方式 |
|---|---|---|
| 热键→首帧 p95 | ≤100ms | bench 20 次/屏 |
| 复制 p95 | ≤60ms | bench 20 次 |
| 空闲内存 | ≤40MB | 托盘空闲 5 分钟 |
| 单屏峰值 | ≤150MB | 1920×1200 |
| 三屏峰值 | ≤250MB | 3×1920×1080 |
| 安装包 | ≤10MB | NSIS 产物 |
| 冷启动到托盘 | ≤500ms | 冷启动 10 次 |

## 11. 测试策略

### 单元测试（core）

- geometry：normalize/clamp/最小尺寸/反向拖拽/边界。
- monitor：目标屏选择、负坐标、primary、fallback。
- config：默认值、缺字段、损坏、版本迁移、原子写。
- hotkey：解析、非法输入、冲突回滚。
- image_ops：裁剪、合成、PNG 编码、尺寸校验。
- annotation：撤销/重做、橡皮、多点线。
- paths：文件名/路径校验、Unicode/长路径。

目标：core 行覆盖 >90%，纯函数 >95%。

### 集成测试

- 抓屏：每块显示器抓一张，尺寸/像素断言（golden）。
- 剪贴板：写入→回读→像素比较。
- overlay：消息注入触发 capture/recapture；不依赖真实鼠标。
- pin：创建/移动/缩放/关闭，窗口句柄与内存释放。
- 配置：设置变更 → 重启 → 读回。

### 性能门禁

- `cargo bench` 或专用 `--bench`：首帧/复制/内存阈值，超阈值失败。
- CI 产物记录基线，回归 >15% 报警。

### 手动矩阵

- `docs/test-cases.md` 的 T1/T2；每次 release candidate 填写。
- T0 spike 已通过，不再重复。

## 12. 实施阶段

### P2.1 骨架与核心截图（先跑通）

- 建模块目录、AppContext、错误类型、日志。
- core::monitor / geometry / capture / clipboard / config / hotkey。
- overlay 线程、窗口、抓屏、选区、复制/取消。
- 托盘、单实例、Alt+X。
- 验收：区域截图复制可用；首帧/内存接近 spike。

### P2.2 工具栏、保存、标注

- 工具栏（复制/保存/贴图/画笔/橡皮/撤销/关闭）。
- 原生保存对话框、PNG 编码、文件名校验。
- 矢量标注、橡皮、撤销/重做；高 DPI 对齐。
- 验收：完整截图工作流；保存/复制像素一致。

### P2.3 贴图

- Pin 线程、分层窗口、拖动/缩放/透明度/关闭/多贴图。
- 跨屏/混合 DPI、显示器变化。
- 验收：贴图全生命周期；内存可控。

### P2.4 设置与配置

- 设置窗口按需创建/销毁；热键录制/冲突回滚；默认操作、自启。
- 配置迁移/损坏回退；单实例转发设置。
- 验收：设置全链路；关闭设置后内存回落。

### P2.5 打包、性能与回归

- NSIS 安装包、签名（如有证书）、卸载清理。
- 性能门禁、T1/T2 回归、长稳/热插拔。
- 验收：安装包 ≤10MB；全部 P0 通过；性能达标。

## 13. 主要风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| Tauri 与原生线程集成复杂 | 崩溃/死锁 | 线程只通过 mpsc；句柄不跨线程；先做最小骨架 |
| 分层窗口混合 DPI 模糊/偏移 | 贴图体验 | 物理像素 + PerMonitorV2；真机矩阵 |
| GDI 标注在 4K 卡顿 | 体验 | 双缓冲、增量绘制、限制撤销步数；必要时 Direct2D |
| 安装包超 10MB | 目标失败 | 移除不必要插件/依赖；native dialog；测量每阶段体积 |
| 裸 F 键环境差异 | 用户预期 | 默认 Alt+X；后置 Hook 高级选项 |
| 热键冲突 | 无法触发 | 设置页冲突检测 + 回滚 + 明确错误 |
| 现有代码迁移回归 | 功能丢失 | 保留 spike 对照；分阶段替换；T1 冒烟 |

## 14. 待确认

1. **v1 范围**是否就是：区域/全屏、复制/保存、画笔/橡皮/撤销、贴图、托盘、单实例、自启、设置；不做跨屏/文字/箭头/马赛克/OCR/云。
2. **默认热键 Alt+X**，设置里可改；v1 不做裸 F 键钩子（后置）。
3. **遮罩渲染用 GDI**、**标注用矢量**、**贴图用分层窗口**、**保存用原生对话框**——是否按推荐方案。
4. **v1 不做自动更新**，先保证安装包 ≤10MB 和稳定；是否可以。
5. 是否保留现有 Vue 设置页风格，只做必要精简。

确认后我按 P2.1 → P2.5 开始实现；每个阶段先给测试用例列表，再写代码。
