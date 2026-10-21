# Jpixel 原生遮罩 Spike（Stage 1 测试包）

这是一个**技术探针**，不是产品。它只验证一件事：

> 在 Windows 11、双屏混合 DPI 下，原生遮罩能不能做到「按下热键后快速出现、选区像素准确、内存可控」。

设置、贴图、画笔、托盘、安装包都不在这个包里。

## 包里有什么

| 文件 | 用途 |
|---|---|
| `spike.exe` | 测试程序，默认热键 **Alt+X**（速度优先构建，已内置抓屏预热） |
| `run-AltX.bat` | 显式用 **Alt+X** 启动（推荐入口） |
| `run-F1.bat` | 只注册 F1 的干净测试（先退出 Jpixel，否则 F1 被占用） |
| `run-F8.bat` | 用 F8 作为热键启动（F1 被占用时用） |
| `run-CtrlAltJ.bat` | 用 Ctrl+Alt+J 作为热键启动（备选） |
| `run-CtrlF1.bat` | 用 Ctrl+F1 启动，用来判断“F1 键本身是否被拦截” |
| `key-test.bat` | 按键诊断：打印 Windows 实际收到的 VK 码 |
| `key-test.ps1` | `key-test.bat` 调用的 PowerShell 脚本 |
| `test-overlay.bat` | 不按热键，直接在 1 号屏显示遮罩 |
| `test-pattern-1920x1200.png` | 125% 屏的测试图 |
| `test-pattern-1920x1080.png` | 100% 屏的测试图 |
| `README.md` | 本文件 |
| `spike-results.md` | 测试结果填写模板 |
| `spike-log.csv` | 运行后自动生成（当前目录） |

> **2026-10-21 更新**：
> - 蓝框闪动已修复（只在拖动时重绘 + 双缓冲）；
> - 热键改为**窗口级注册**（`RegisterHotKey(overlay hwnd)`），并移除了 `AttachThreadInput`；
> - 当前测试机已确认：**裸 F 键（F1/F8）会被系统/键盘软件层拦截；Ctrl+F1 和 Ctrl+Alt+J 均正常**；默认热键为 **Alt+X**；
> - 修复副屏黑边 / C1 `Area size is invalid`：`screenshots::capture_area` 的 `x/y` 是**相对该屏的本地坐标**，且会再乘一次 `scale_factor`；已改用 `capture_area_ignore_area_check(0, 0, 物理宽, 物理高)`；
> - 拖动选区时用 `ClipCursor` 把鼠标限制在当前显示器内，松手/取消后自动释放（已确认可用）；
> - 遮罩已打开时再按 Alt+X：先关闭旧遮罩，再按当前鼠标/焦点所在屏重新抓取（不会叠加两个遮罩）；
> - 当前 exe 已默认 Alt+X，可直接双击 `spike.exe`，也可以双击 `run-AltX.bat`；
> - 运行前请关闭旧的 spike 控制台，避免旧进程占用热键或锁住 exe。

完整测试步骤、C1–C4 排列配置和判定标准在：
[../../docs/spike-test-plan.md](../../docs/spike-test-plan.md)

开发机上的参考基线在：
[../../docs/spike-baseline.md](../../docs/spike-baseline.md)

## 运行前

1. 关闭 Snipaste、OBS、Teams 共享、录屏等会占用 DXGI 的软件；
2. 关闭旧的 spike 控制台（如果还开着），避免它占用热键或锁住 exe；
3. 如果要用 F1 测试，先从系统托盘退出 Jpixel；默认的 Alt+X 不受 Jpixel 影响；
4. 双击 `spike.exe` 会打开一个控制台窗口，**把它最小化**，再去点目标窗口；
5. 如果 Windows SmartScreen 提示“未知发布者”，选择“仍要运行”（这是未签名测试程序）。

## 手动模式

1. 双击 `spike.exe`（默认 Alt+X），或双击 `run-AltX.bat`；
2. 在你想截的那块屏上点一下目标软件，让它获得焦点；
3. 按 **Alt+X**：应该只在这块屏出现遮罩；
4. 如果遮罩已经打开，想换一块屏：把鼠标移到目标屏（或点一下目标屏的窗口），再按 **Alt+X**，旧遮罩会关闭并在目标屏重新抓取；
4. 按住左键拖一个框；拖动期间鼠标会被限制在当前屏，不会跑到另一块屏；
5. 按 `Enter`：复制选区到剪贴板；按 `Esc`：取消并回到原窗口；
6. 打开画图，`Ctrl+V`，检查粘贴出的图像是否与选区一致。

控制台会打印每次的：

```
capture=xx ms  first_frame=xx ms  clipboard=xx ms  pixel_match=true/false  max_diff=0  mem=xx MB
```

同时写入 `spike-log.csv`。

> spike 启动时会对每块屏做一次预热（控制台打印 `capture warm-up: ...`），所以第一次按热键不会再有冷启动尖峰。
> 如果你按热键后 Enter/Esc 没反应，把控制台里的 `focus: ...` 行发我。

## 为什么 F 键不行？怎么测

先双击 **`key-test.bat`**，在 20 秒内依次按：

1. `F1`
2. `F8`
3. `Ctrl+Alt+J`
4. `Fn+F1`（如果键盘有 Fn）

> 注意：`key-test.bat` 只读取按键状态，**不会拦截** F1，所以测试时浏览器仍然可能打开；这是正常的，它证明键盘确实发出了 F1。

它会把 Windows 实际收到的虚拟键码打印出来，例如：

```
0.52s  VK=0x70  F1
1.10s  VK=0x77  F8
2.40s  VK=0x11  Ctrl
2.42s  VK=0x12  Alt
2.45s  VK=0x4A  J
```

把这几行发我，就能判断：

| 看到什么 | 说明 |
|---|---|
| 按 F1 出现 `VK=0x70 F1` | Windows 确实收到了 F1；问题在软件钩子/热键通知，不在键盘 |
| 按 F1 出现 `VK=0x2F HELP` 或浏览器/媒体键码 | 键盘把 F1 映射成了帮助/媒体键；用 Fn 或改用 Ctrl+Alt+J |
| 按 F1 什么都没有 | 按键在到达 Windows 之前就被键盘固件/驱动吃掉了；试 `Fn+F1` 或换键盘 |
| `Ctrl+Alt+J` 出现 Ctrl/Alt/J | 普通组合键正常，问题只在 F 键 |

还可以做几个快速交叉验证：

1. **干净的 F1 测试**：关闭所有 spike 控制台 → 从托盘退出 Jpixel → 双击 `run-F1.bat` → 确认控制台显示 `registered global hotkey: F1` → 按 F1。  
   注意：`run-AltX.bat` 只注册 Alt+X；如果当前跑的是它，F1 不会被接管，浏览器打开是正常的。
2. **Fn+F1 / Fn+F8**：如果带 Fn 能用，就是键盘的 Fn-Lock/媒体模式（常见切换键是 `Fn+Esc`）。
3. **屏幕键盘**：运行 `osk.exe`，用鼠标点 F1。如果屏幕键盘的 F1 能触发遮罩，说明物理键盘发的不是 F1。
4. **Ctrl+F1**：双击 `run-CtrlF1.bat`，按 `Ctrl+F1`。如果它能用、单独 F1 不能用，说明 F1 这个键被系统/软件单独拦截了。

另外，如果装了 PowerToys Keyboard Manager、AutoHotkey、Logitech/Razer 驱动、Steam/NVIDIA 覆盖层，先退出它们再测；这些软件的底层键盘钩子会抢在全局热键之前吃掉 F1。

## 热键没反应？

**最终结论：裸 F 键被系统/键盘软件层拦截，Ctrl+F1 和 Ctrl+Alt+J 正常，默认改用 Alt+X。** 这是环境限制，不是热键代码问题；正式版不再默认裸 F 键。副屏黑边和 C1 `Area size is invalid` 已通过修正抓屏坐标修复，请重新跑 C1/C3/C4。

如果 F1 仍然不行，按这个顺序查：

1. 看控制台有没有这两行：

   ```
   registered global hotkey: Alt+X (overlay hwnd=0x...)
   manual mode ready: Alt+X = capture, Enter = copy, Esc = cancel
   ```

2. 按热键后有没有 `[hotkey] received`：
   - 没有 → 按键没送到 spike；先点一下控制台按 `Esc` 退出文字选择（QuickEdit 会暂停进程），再试。
   - 有 → 抓屏/窗口问题，把控制台最后几行发我。

3. 还是不行，双击 `test-overlay.bat`（不按热键直接显示遮罩）：
   - 遮罩出现 → 抓屏/窗口没问题，问题只在热键；
   - 遮罩也不出现 → 抓屏/窗口问题。

4. 想换回 F 键：`run-F8.bat`，或运行 `spike.exe --hotkey F1`。如果注册了 F1 但仍然被系统拦截，它们不会触发。

5. 遮罩出现在“当前焦点窗口”所在屏。如果你刚启动控制台、焦点还在控制台，遮罩会出现在控制台那块屏；先点一下目标软件，再按热键。

## 免热键测试

如果你只是想确认原生遮罩能不能正常显示：

```
.\spike.exe --test-overlay --monitor 1
```

它会直接在 1 号屏显示遮罩，不需要按 F1。拖框后 Enter 复制，Esc 取消。

## 查看显示器编号

```
spike.exe --list-monitors
```

会打印类似：

```
1: 1920x1200 @125% rect=(0,0,1920,1200) primary
2: 1920x1080 @100% rect=(0,-1080,1920,0)
```

编号以这次打印为准；改变排列后编号可能变化。

## 性能模式

先关掉其他抓屏软件，然后在 `spike.exe` 所在目录打开 PowerShell：

```
.\spike.exe --bench --monitor 1 --iterations 10
.\spike.exe --bench --monitor 2 --iterations 10
```

它会自动抓屏、显示遮罩、复制一个 800×600 的固定选区，然后写日志。

- `--monitor` 用上一步打印的编号；
- `--iterations 10` 表示跑 10 次；
- 启动时会先做一次预热（控制台打印 `capture warm-up: ...`），预热不计入 P1；
- 每次运行都会在 `spike-log.csv` 追加行，不会覆盖。

## 日志字段

| 字段 | 含义 |
|---|---|
| `mode` | `idle` / `manual` / `bench` / `test` / `error` |
| `monitor` | 显示器描述（分辨率、缩放、物理矩形） |
| `rect` | 目标屏的物理矩形 |
| `scale` | 缩放比例（125% = 1.250） |
| `capture_ms` | 抓屏耗时 |
| `first_frame_ms` | 热键/触发 → 遮罩第一帧 |
| `clipboard_ms` | Enter → 剪贴板写入完成 |
| `pixel_match` | 回读剪贴板与选区是否逐像素一致 |
| `max_diff` | 最大通道差（0 最好，≤1 可接受） |
| `working_set_mb` | 当前进程工作集内存 |
| `note` | `copy` / `cancel` / `iteration=N` 等 |

## 测试流程

不要一次跑完。建议：

1. 先在当前排列下跑手动 F1/F2/F3/F4/F5；
2. 再跑 `--bench`，确认 P1–P5；
3. 再按测试计划的 C1–C4 切换排列/主屏，做 D1–D10；
4. 最后做 S1–S3 和可选用例。

## 需要回传的材料

- `spike-log.csv`
- 填好的 `spike-results.md`
- 每个配置的 Windows 显示设置截图（排列 + 缩放）
- 每个配置的遮罩截图/手机照片（至少 C1、C3、C4）
- 任务管理器内存截图（空闲、截图后、20 次后）
- 控制台报错（如果有）

## 已知限制

- 这是未签名的测试程序，可能触发 SmartScreen；
- 没有安装包、没有托盘、没有设置页；
- F1 是硬编码的；
- 只支持一个选区、没有画笔/橡皮；
- 没有 RDP/HDR/真实 4K 的正式验证。
