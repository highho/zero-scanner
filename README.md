# Zero

一个现代、智能的内存扫描工作台。用 Rust + egui 重写的 Cheat Engine 思路实现，**本机推理、不联网、不上传**。

## 快速开始

双击 `zero-scanner.exe` 启动。

**给 AI 用（MCP 服务）**：命令行加端口参数启动，

```
zero-scanner.exe --mcp 3780
```

启动后打印一行地址，形如：

```
MCP listening on http://127.0.0.1:3780/mcp?token=xxxx
```

把这个 URL 填进 MCP 客户端即可。服务只监听本机，**不带 token 的请求一律 401**，关闭时端口完全不监听。

## 主要功能

| 功能 | 说明 |
|---|---|
| 数值扫描 | 精确值 / 模糊值 / 字符串，按可写区收窄，结果分诊排序 |
| 指针扫描 | 从已知地址反推稳定指针链 |
| AOB | 按字节模式搜索 |
| 模块枚举 / 符号解析 | 模块表、导入表、导出表 |
| 反汇编 | 反汇编目标进程代码 |
| 代码补丁 / 代码洞 | 可逆写入，支持逐条撤销 |
| 自动汇编 | 汇编 x86-64 指令并做往返校验 |
| CT 表格 | 读写 Cheat Engine 的 .ct 文件 |
| 变速齿轮 | 挂起/恢复目标线程，0.1x-10x |
| Lua 脚本 | 直接读写目标进程的 CE 风格脚本 |
| 观察队列 | 后台采样，看哪个地址真的在动 |
| 反馈学习 | 标记「不是它」，学特征、重排序 |
| 工作区持久化 | 扫描结果、补丁、地址列表随配置保存 |

## MCP 工具（14 个）

**连接**：`zero_attach` · `zero_status`
**扫描**：`zero_scan` · `zero_next_scan` · `zero_candidates`
**分析**：`zero_pointer_scan` · `zero_disassemble`
**内存**：`zero_read_memory` · `zero_write_memory` · `zero_undo`
**观察**：`zero_watch`（会话式：`action=start/status/stop`）
**表格与脚本**：`zero_ct_table` · `zero_lua`
**地址**：`zero_add_address`

## 命令行

```
zero-scanner.exe                      图形界面
zero-scanner.exe --mcp <port>          启动 MCP 服务
zero-scanner.exe --selftest            跑自检（结果写 selftest.log，退出码即结论）
zero-scanner.exe --screenshot <png> --tab <名称>   渲染某个页面截图
zero-scanner.exe --verify <pid>        对目标进程做 8 项验证（结果写 verify.log）
zero-scanner.exe --e2e <pid> <info>    端到端验证
```

## 构建

需要 Rust 工具链和 Windows 上的 MSVC 链接器（mlua 使用 vendored feature，Lua 直接编进二进制，无需另装 Lua）：

```
cd app
cargo build --release
```

产物为 `app/target/release/zero-scanner.exe`。

## 端到端测试

仓库根目录的 `e2e_target.py` 是一个真实的测试目标进程：它公布自己的金币 / 血量地址、指针链和 AOB 标记，供 `--e2e` 使用。

```
python e2e_target.py      # 在后台起一个带已知内存的目标进程
zero-scanner.exe --e2e <pid> e2e_target.info
```

## 安全原则

- **写入必须可还原**：每次补丁都保存原字节，可逐条撤销。
- **不杀不属于你的进程**：变速只挂起目标线程，且拒绝对自己进程操作。
- **不假设可以覆盖**：写文件（如 .ct）前必须显式确认。
- **出错不编结果**：地址解析失败就报错，绝不返回一个假地址。
