# Zero 后续开发案

> 基线：v1.2.0（2026-11-28，HEAD c343259）。本文档记录差距分析与三个版本的规划，随实现进度更新。
> 进度：1.3 / 1.4 / 1.5 均已完成并合并进 master（1.5 收尾于 2026-11-28 夜）。
> 规划细化：2026-11-28 晚。所有依赖结论均已在本机源码核实（windows-sys 0.52 符号、现有模块接口、仓库状态），不是猜测。

## 一、现状

| 维度 | 数字 |
| --- | --- |
| 源码 | 35 个模块，25,431 行（ui.rs 5,463 / app.rs 3,406 / mcp.rs 1,425 / theme.rs 1,277） |
| 界面 | 17 个工作区 |
| 自检 | 203 项全 PASS |
| MCP | 18 个工具 |
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

## 五、1.5 「更稳」（已完成）

- **变速齿轮（已做）**：从全局挂起改为**按线程选控**（`speed::TargetMode::GameLogic` 默认，按线程起点落在哪个模块判断，48 个系统模块前缀排除渲染/音频/DirectX/Vulkan/音频栈）；时间片按线程数自适应（8 以内 20 ms、32 以内 30 ms、96 以内 40 ms、更多 60 ms），比例不变所以倍率不变；读不到线程起点的不碰（猜错会冻住渲染）；GameLogic 模式计划为空时**明确报错而不回退到全挂起**。仍未做注入式计时器钩子（红线）。
- **更新包校验（已做）**：`update::sha256_file` 走 BCrypt（`Win32_Security_Cryptography`），`published_sha256` 从 release notes 里抓 64 位十六进制；下载后 `check_hash` 三态 Match / Mismatch / NotPublished，Mismatch 直接删文件并报错；界面按三态分别显示「一致（前 12 位…）」「不对：官方 X 实际 Y」「发布说明没公布，无法核对」。
- **内存区域热图（已做）**：新增 `app/src/heat.rs`（采样可写区域前 4 KB，逐字节比差值，上限 512 区域），第 17 个工作区「内存热图」（Tab::Heat，侧边栏 Grid 图标），列表按热度排序、色阶按绝对热度（<2% 灰、<20% 绿、<60% 黄、其余红），可一键跳到内存浏览。只读，不写目标进程。
- **附加失败诊断（已做）**：新增 `app/src/diag.rs`（`Diagnosis { summary, action }` + `line()`），错误码 5/87/299/31 各有结论与下一步，未知码保留原数字；`winmem::open_process`、`debugger` 的 DebugActiveProcess 失败、MCP 的 zero_attach 都走同一张表；最后一次诊断存在全局槽里，界面与 MCP 读到的是同一句话。

### 模块与文件改动（实际）

| 文件 | 改动 |
| --- | --- |
| `app/src/speed.rs` | 重写：TargetMode、plan_threads、thread_start_address（ntdll NtQueryInformationThread 动态取）、period_for 自适应 |
| `app/src/heat.rs`（新增） | 热图采样（WINDOW 4 KB / MAX_REGIONS 512） |
| `app/src/diag.rs`（新增） | 错误码 → 结论 + 下一步，全局记住最后一次 |
| `app/src/update.rs` | BCrypt SHA256、published_sha256、HashCheck 三态、State::Ready { path, hash } |
| `app/src/addr.rs` | 修 `parse_offset` 把负偏移钳成 0 的缺陷 |
| `app/src/ui.rs` | 第 17 页 ui_heat、变速作用范围选择、更新页哈希三态 |
| `app/src/app.rs` | heat 字段 + tick_heat、speed_mode/speed_candidates |
| `app/src/mcp.rs` | zero_attach 失败带下一步 |

### 验收数字（实测）

- 自检 187 → **203 项全 PASS**（+16：热图 9、诊断 6、变速原有 6 已计入）。
- `cargo test` 22 passed / 0 failed（修掉两个原本就坏的测试：addr::offset_parse、aliases 展开测试）。
- 工作区 16 → **17 页**（新增「内存热图」）。
- MCP 工具数不变（18），但 zero_attach 的失败信息带可执行的下一步。

## 五点五、1.6 候选（下一轮）

1.3 / 1.4 / 1.5 已把「找值 → 找基址 → 找出谁在改 → 做成可复用的东西」这条链补完。下一轮不再加新能力面，而是把已有能力做深、做成玩家真能一次上手的东西：

| 方向 | 现状 | 1.6 做什么 | 验收 | 状态 |
| --- | --- | --- | --- | --- |
| 首次使用引导 | 17 页平铺，新用户不知道从哪开始 | 首启向导：选进程 → 输入看到的数值 → 给出前 3 个候选 → 教一次「改一下再搜」 | 向导能跑通靶场并留下地址 | 完成 f3d8ed0 |
| 扫描性能 | 多线程分块扫已就位（按区域均分桶、结果合并去重），但线程数与吞吐对用户不可见 | 把「实际用了几个线程、扫了多少字节、多少 MB/s」变成结果页和完成提示上的数字；小区域集不硬凑八线程 | 靶场扫描后，结果页与完成提示都显示「N 线程 · X MB/s」，且 N 与实际 worker 数一致 | 完成 d04c3ab（实测 8 线程 · 276 MB/s） |
| 命中历史可用性 | 断点命中只列指令与寄存器 | 命中行可一键「在这里打补丁跳过」，并复用 inj 的可还原纪律 | 打补丁→命中消失→撤销→命中回来 | 完成 8379e99 |
| 热图与扫描联动 | 热图只是展示 | 热区一键「只扫这些区域」（scan 增加区域过滤入口） | 热区过滤后候选数明显下降 | 完成 f32793b |
| 训练器体验 | 导出 .ct + .bat | 导出前给一张清单（哪些条目可导出、哪些跳过及原因），并可选择只导出部分 | 清单与实际写入一致 | 完成 d0f6224 |
| 错误可读性 | 只有附加失败有诊断表 | 扫描 / 读内存 / 写内存失败也走同一张诊断表 | 每类失败都有「下一步」 | 完成（本轮） |

排期建议：前两项一轮，后四项一轮；每轮仍然按第八节交付（自检只增不减 + 截图 + 真实 E2E）。

1.6 结果：六项全部落地。自检 187 → **217 项全 PASS**，`cargo test` 25 passed，真实 E2E 24 项 PASS（含实测扫描吞吐与未映射读取的诊断文案）。
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
| 1.5 | 稳定性 | `speed.rs` 选线程重写 + 更新 sha256 + 热图 + 附加诊断 | 无 | 自检 187 → **203**；新增第 17 页「内存热图」 |

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
