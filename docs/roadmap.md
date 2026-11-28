# Zero 后续开发案

> 基线：v1.2.0（2026-11-28，HEAD c343259）。本文档记录差距分析与三个版本的规划，随实现进度更新。
> 规划细化：2026-11-28 晚。所有依赖结论均已在本机源码核实（windows-sys 0.52 符号、现有模块接口、仓库状态），不是猜测。

## 一、现状

| 维度 | 数字 |
| --- | --- |
| 源码 | 30 个模块，20,212 行（ui.rs 4,872 / app.rs 3,115 / theme.rs 1,256 / mcp.rs 1,169） |
| 界面 | 15 个工作区 |
| 自检 | 136 项全 PASS |
| MCP | 14 个工具 |
| 扫描 | 9 种数值方式 + 指针扫描 + AOB |
| 修改 | 代码补丁 / 代码洞（可还原）、自动汇编、变速、Lua、CT 表格 |
| 扩展 | 8 套主题 + 主题包 + 插件清单（目前只接 theme 能力） |
| 分发 | GitHub Release 三资产 + 自动更新 + 免管理员安装包 |

## 二、差距：按玩家真正会用的顺序排

Cheat Engine 的实战路径是：**找值 → 找基址 → 找出谁在改它 → 做成下次还能用的东西**。

| 环节 | CE 有什么 | Zero 现在 | 缺口 |
| --- | --- | --- | --- |
| 找值 | 9 种扫描 | 9 种 | 齐 |
| 找基址 | 指针扫描 + 指针地图（存盘、换地址重扫） | 一次性扫描，结果不存盘 | 中 |
| **找出谁在改** | **硬件断点 / 内存断点，命中记录指令与寄存器** | **完全没有调试器** | **高（最大缺口）** |
| 做成可复用 | CT 表格 + 训练器导出 + 完整 Lua API | CT 读写 + 6 个 Lua 全局函数 | 高 |
| 反作弊对抗 | 内核驱动 | 明确不做 | 不做 |

结论：**1.3 补调试器，1.4 补可携带性，1.5 补稳定性。**

## 三、1.3 「谁在改它」

### 目标

回答一个具体问题：这个地址是被哪条指令改的？那条指令长什么样？当时的寄存器是什么？

### 已核实的依赖前提（2026-11-28 实测）

- windows-sys 0.52 已有 feature 已覆盖：`DebugActiveProcess` / `WaitForDebugEvent` / `ContinueDebugEvent` / `DebugActiveProcessStop` / `SetThreadContext` / `Wow64GetThreadContext` / `Wow64SetThreadContext` 都在 `Win32_System_Diagnostics_Debug`。
- 唯一缺口：`GetThreadContext` 需要 `Win32_System_Kernel` feature —— **Cargo.toml 补一个 feature 即可，不引新依赖、不升级 windows-sys。**
- `CONTEXT`（x64）与 `WOW64_CONTEXT`（x86）都含 Dr0–Dr3 / Dr6 / Dr7 字段与 `CONTEXT_DEBUG_REGISTERS` 标志，可对 WOW64 目标走 Wow64 分支。
- 断点命中后取 RIP 用 disasm.rs 反汇编、标识模块归属用 winmem::ModuleInfo::offset_of（既有）。

### 功能

1. **硬件断点（DR0–DR3）**
   - 对某个地址设写入 / 读写断点，每线程最多 4 个（槽位管理，无空槽明确报错）
   - 命中时记录：指令地址、反汇编文本、寄存器快照（RAX–R15、RIP、RFLAGS）、写入值
   - x86（WOW64）目标走 WOW64_CONTEXT 分支，x64 走 CONTEXT
2. **找出是什么改写了这个地址**
   - 地址列表右键直接发起，结果进一张命中历史表
   - 历史按时间倒序，同一条指令重复命中合并计数
3. **命中 → 反汇编联动**
   - 点一条命中跳到反汇编页并定位到该指令
   - 可选：在该指令处建议「打补丁跳过它」（复用 inj.rs，可还原）
4. **安全边界**
   - 只对自己 attach 的进程设断点
   - 断点在会话结束或进程退出时全部清除（Dr0–Dr3 归零 + 恢复原 CONTEXT）
   - 目标线程上下文恢复原值；已被调试 / 受保护进程拒绝时给具体原因（ERROR_ACCESS_DENIED=5 / 已被调试=299 等），不静默失败

### 模块与文件改动（预期工作量）

| 文件 | 改动 |
| --- | --- |
| 新增 `app/src/debugger.rs` | 核心区（约 600 行）：\`DebugSession\` 状态机（Idle→Attaching→Active→Detached）、断点表、事件线程、命中历史、上下文快读 |
| `app/src/main.rs` | mod debugger；`--tab debugger`；新 CLI `--dbg-selftest` 可单测调试器链路 |
| `app/src/app.rs` | Tab::Debugger（第 16 页）、ui_debugger、从地址列表右键发起断点 |
| `app/src/ui.rs` / `theme.rs` | 断点页布局、命中历史表、寄存器面板（沿用 23 键调色板，无需新配色） |
| `app/src/session.rs` | 命中历史可存盘（会话级），断点本身不跨会话持久化（安全边界） |
| `app/src/selftest.rs` | 新增约 10 项自检 |
| `app/src/mcp.rs` | 新增会话式工具（start/status/stop 三段，沿用 zero_watch 的会话纪律） |
| `app/Cargo.toml` | 加 `Win32_System_Kernel` feature；版本号 1.3.0 |

### 自检 / 验收（写进断言与 E2E）

- 对**自己进程**分配一个已知地址并周期自增（多线程），设写入断点 → 命中次数与自增次数一致（自检断言）
- 命中记录里的 RIP 落在模块内，且反汇编长度有效
- 四个断点同时设置不互相干扰（4 个不同地址各命中其对应计数）
- 关闭调试器后目标进程无残留：Dr0–Dr3 恢复为 0，线程上下文与原值一致，可直接再次 attach
- E2E：e2e_target.py 增加一个高频自增变量（如每轮 +1 的 Python 线程），--e2e 对其设写断点，断言命中计数 > 0 且 RIP 在 python exe 模块内

### 风险

- 事件循环与 UI 打架 → 后台线程 + mpsc channel 回传（与 update.rs / mcp.rs 同款模式）
- 目标已带调试器 / 受保护 → attach 失败给 5/299 具体码，不重试不静默
- WOW64 目标：x86 命中地址在任务线程里是 32 位地址，读内存 / 反汇编走 32 位路径
- 断点命中时目标线程会停一下：只对用户主动选择的小地址集合设断，命中即继续（ContinueDebugEvent）

## 四、1.4 「能带走」

### 目标

把一次调试的成果变成下次还能用的东西。

### 功能

1. **Lua API 补到 CE 常用集**（lua.rs 已有：print / readInteger / writeInteger / readBytes / writeBytes / getAddress，共 6 个；补齐到 14 个）
   - 新增：`autoAssemble`（调 assembler.rs + inj.rs 可还原补丁）、`AOBScan`（调 aob.rs，返回结果表）、`registerSymbol`（新符号表，模块内先于 addr.rs 解析）、`readPointer` / `writePointer`（按目标位宽走 handle.read_pointer）、`allocateMemory`（调 winmem::allocate，脚本结束时释放）、`createTimer`（mlua 每帧回调，注意与 UI 帧 sync）、`getAddress`（已有，补 module 名+指针链语义）
   - 目标：能直接跑网上常见的 CE 表脚本；3 个真实样本放 examples/
2. **训练器导出**（新增 `trainer.rs`，约 300 行）
   - 路线选择：**不产独立 exe**（会引入第二个工程与 ABI 承诺），导出为「`.ct` + `启动.bat`」：bat 调 `zero-scanner.exe --apply-table 训练器.ct`，新增 `--apply-table` CLI（无窗口读表、应用补丁、挂热键，Ctrl+Shift+Q 退出）
   - 导出前校验：所有地址可解析（addr::resolve 非 None）、所有补丁可还原（inj::verify）
   - 干净目录里能启动、能读写、能还原（验收）
3. **指针地图存盘**（pointerscan.rs + session.rs）
   - PtrDone / PointerPath 序列化为 workspace 里的 `pointer_maps` 段
   - 重新加载后按新基址重映射，解析结果与内存中一致（验收）
4. **插件能力扩展**（plugins.rs 契约扩展）
   - Manifest 新增字段：`struct_templates`（JSON 描述字段布局，structs.rs 注册，按模块签名匹配）、`game_profiles`（每游戏一套扫描参数 + 热键，落地到 smart::Preset 同构的配置）
   - 保持纪律：坏清单跳过并报原因，不阻塞启动

### 模块与文件改动

| 文件 | 改动 |
| --- | --- |
| `app/src/lua.rs` | +8 个 API（约 +250 行） |
| 新增 `app/src/trainer.rs` | 导出器 + `--apply-table` 模式（约 300 行） |
| `app/src/session.rs` | pointer_maps 段（约 +80 行） |
| `app/src/plugins.rs` | manifest 两字段 + 注册（约 +100 行） |
| `app/src/structs.rs` | 模板注册与匹配 |
| `app/src/main.rs` | --apply-table；版本号 1.4.0 |
| `examples/` | 3 个 CE 样本 + 1 个 struct 模板 + 1 个游戏配置包示例 |
| `app/src/mcp.rs` | +2 工具：zero_symbols、zero_breakpoint（若 1.3 未及） |

### 自检 / 验收

- 用 3 个真实 CE 脚本样本跑通（examples/）
- 指针地图存盘后重新加载，解析结果与内存中一致
- autoAssemble 的补丁可单独撤销；allocateMemory 在脚本结束后可释放（无泄漏断言）

## 五、1.5 「更稳」

- **变速齿轮**：现状是全局线程挂起/恢复（speed.rs 144 行），问题 = 目标里计时与音频一起停。1.5 改为**按线程选控**：只挂起游戏逻辑线程（按模块/优先级过滤），排除渲染与音频线程；并把时间片从固定 40ms 改为自适应（按目标线程数校准），保持 suspend_count 精确恢复的既有纪律。**不做注入式计时器钩子**（明确不做 DLL/代码注入是本项目红线，见第六节）
- **更新包校验补 sha256**：BCrypt（Win32_Security 已在）加 `BCryptHash` SHA256；发布时在 release notes 里公布安装包 sha256，应用下载后先核对再进替换脚本（validate_exe 保留为第一道闸）
- **内存区域热图**：winmem::regions 采样写操作统计（VirtualQueryEx + 时间窗），UI 一屏热力网格显示哪些页在频繁变化（复用 Regions 接口，不做新依赖）
- **附加失败诊断**：open_process 失败按系统错误码归类（5=权限/需管理员、87=进程已退出、299=已被调试或受保护），UI 与 MCP 用同一诊断表

### 模块与文件改动

| 文件 | 改动 |
| --- | --- |
| `app/src/speed.rs` | 选线程 + 自适应时间片（约 +120 行） |
| `app/src/update.rs` | 下载后 sha256 比对（+80 行）；发布脚本记录哈希 |
| `app/src/winmem.rs` | 附加诊断函数（错误码映射） |
| 新增 `app/src/heatmap.rs` 或并入 ui_memory | 热图采样与绘制（约 200 行） |
| `app/src/mcp.rs` | zero_attach 错误信息带诊断码 |
| `installer/build.sh` | 发布产物 sha256 输出 |

## 六、明确不做

- 内核驱动、绕过反作弊
- DLL 注入（训练器导出因此走 .ct + --apply-table，而非注入式加载器）
- 云端大模型
- 联机游戏用途

## 七、排期建议

| 版本 | 主题 | 主要工作量 | 依赖 | 验收数字目标 |
| --- | --- | --- | --- | --- |
| 1.3 | 调试器 | 新增 `debugger.rs`（约 600 行）+ 断点页（16 页）+ 自检 +2 MCP 工具 | 无（仅加 1 个 Cargo feature） | 自检 136 → ~146；MCP 14 → 16 |
| 1.4 | 可携带 | `lua.rs` +8 / 新增 `trainer.rs` + `--apply-table` / 指针地图存盘 / 插件扩展 | 1.3 的命中数据（训练器导出可用断点建议做补丁） | 自检 ~146 → ~156；Lua 6 → 14；examples +5 |
| 1.5 | 稳定性 | `speed.rs` 选线程重写 + 更新 sha256 + 热图 + 附加诊断 | 无 | 自检 ~156 → ~164；新增 1 张热图（可并入内存页） |

## 八、验证策略（沿用现有纪律）

每个版本交付时必须同时给出：

1. `--selftest` 新增断言全 PASS（数量只增不减）
2. 至少一张新页面截图（`--demo` 实渲）
3. 一条真实 E2E（对练习靶场或 `e2e_target.py`）
4. README 与 docs 同步更新
5. 安装包重新构建并真装一遍
6. 版本号 / README 徽章 / Cargo.toml 同步 bump；发布时 README 注明 sha256

## 九、开发顺序建议（按依赖排）

1. 1.3 先落 `debugger.rs` 核心与自检（不动 UI，先有断言）→ 再补断点页与 MCP
2. 1.4 依赖 1.3 的命中历史 → 先 Lua（无 UI 依赖）→ 再指针地图 / 训练器 / 插件
3. 1.5 全部无前置依赖，可在任意间隙并行做 winmem 诊断与 update sha256
