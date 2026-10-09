# CastoriceDumper

基于 HSR-OWNER 的 IL2CPP 运行时导出器，构建产物为 Windows x64 DLL `astrolabe.dll`。
当前适配目标是本地 **StarRail 4.6.51**。Dumper 从已加载模块和实时 IL2CPP 元数据定位运行时入口，
不读取或编译期嵌入 `profiles/`；Morax 是独立的静态工具，两者没有 crate 依赖。

## 构建

```powershell
cargo build --release --locked
```

产物：`target/release/astrolabe.dll`。这是 DLL，不支持 `cargo run`。

根目录是虚拟 workspace，默认构建 `crates/loader`；DLL 入口和日志也已迁入该 crate。
运行时与导出代码全部使用 `crates/il2cpp`、`crates/reflection`、`crates/derive`、`crates/utils`、`crates/dumper`。
旧 `src` 已删除，迁入的实现沿用 HSR-OWNER 的结构。源码只使用普通说明注释，不使用文档注释。

## 导出

DLL 加载后创建工作线程，等待模块与 IL2CPP 就绪，检查已加载的 PE 头，随后执行导出。
启动时不读取磁盘上的整个游戏 DLL 或计算 SHA-256；动态扫描使用已加载模块的有界内存快照。
PE 头检查体系结构和镜像边界，之后按指令、数据流和元数据签名验证，不使用版本白名单。
日志与导出结果位于**游戏进程工作目录**的 `DUMP/`，控制台不显示进度条。

启动日志包含实际加载的导出器路径和基址、Debug/Release 构建类型及工作目录。
API 就绪检查会记录槽位错误、domain 和程序集数量；等待和长循环约每 2 秒输出一次 INFO 状态，
类型扫描、方法映射写入、反射绑定及自检均有开始与完成信息。
日志采用与 `robinsr` 相同的 `tracing`、`ansi_term`、`env_logger` 组合，使用 `env_logger` 默认的 UTC 时间、
级别和模块名称格式。控制台默认 INFO，支持 ANSI 颜色；`DUMP/dump.log` 默认 DEBUG、追加写入且不带颜色控制码。
已有的 `log` 调用也进入同一后端，不会丢失迁入模块的日志。

`RUST_LOG` 同时控制控制台和文件过滤，例如 `debug` 或 `info,il2cpp=debug,reflection=debug`；
`RUST_LOG_STYLE=never` 可关闭控制台颜色。环境变量需要设置在游戏进程启动前。
控制台不可用或 ANSI 初始化失败时回退为纯文本，文件日志仍保留。
若单次原生调用阻塞，循环无法继续输出进度，可用该调用前的日志定位；这些日志不保证中断原生调用。
`methods.json` / `methods2.json` 流式写入 `.partial` 文件，完成后替换正式文件。
已验证布局的原生 MethodInfo 直接读取方法地址，由阶段级 SEH 保护；不再为每一个地址调用
`VirtualQuery`。这是本次日志中两份方法映射耗时约 51 分钟的主要排查点，新的实际耗时仍需运行确认。

默认读取 `crates/config/Config.toml` 中的 `[output].stages`。运行时工作目录中的 `Config.toml` 会覆盖内置默认配置，
因此可以在游戏目录旁直接调整要输出的阶段；也可以用 `CASTORICE_CONFIG` 指定配置文件路径。
`CASTORICE_DUMP` 仍可作为一次性环境变量覆盖配置：

```powershell
$env:CASTORICE_DUMP = "schema"
```

配置示例：

```toml
[output]
stages = ["proto", "schema", "csharp"]
```

可选阶段为 `proto`、`schema` 和 `csharp`。
旧配置的 `metadata` 会在初始化前明确报错；请单独运行 `morax.exe`。

必须让游戏进程从设置了该变量的环境启动；修改终端变量不会影响已经运行的游戏。

| 阶段 | 输出 |
| --- | --- |
| `schema`（别名 `parser`） | `DUMP/data.json`、`DUMP/excel_paths.json`、`DUMP/mod.rs` |
| `proto` | `DUMP/StarRail.proto`、`DUMP/packetIds.json`、`DUMP/packetIds.obfuscated.json`，以及处理器映射 |
| `csharp` | `DUMP/dump.cs`：原生类型、继承、字段及偏移、构造函数、方法签名和 RVA/实际 VA |

`csharp` 按 honkai-dumper 的精简范围导出，遍历已建立的完整类型缓存，使用原生 IL2CPP API，
不逐类型创建 RuntimeType，不逐成员创建反射对象，不展开属性、事件、特性、泛型参数或常量值。
属性和事件的访问器仍作为方法保留；类型名称保留原生元数据的泛型名称格式。完整静态信息由 Morax 导出。
元数据中为空的类名或签名类型名写为 `""` 并记录日志，继续保留该类型的成员；空句柄仍明确报错。
方法 flags 偏移从当前原生 instance API 指令解析，不复制参考项目的固定结构偏移；地址检查使用局部内存区域缓存。
字段偏移对值类型实例字段扣除 16 字节对象头，静态和常量字段保持原始偏移。
每约 2 秒记录类型速度、预计剩余时间及下一个类句柄，超过 1 秒的类型记录慢项，完成后记录字段和方法数量及分类耗时。
输出先写入 `dump.cs.partial`，成功后替换 `dump.cs`；实际性能需以新构建的游戏内日志为准。

`schema` **保持 HSR-OWNER 的格式**，不是旧版 `ExcelSchema.json`：

- `data.json`：以类型名为键，保留 `Class`、`Struct`、`Typeindex`、`Enum` 类型标签及 `fields`、`interfaces`、`descendants` 等原有结构。
- `excel_paths.json`：类型名到 `BakedConfig/ExcelOutput/*.bytes` 等配置路径的映射。
- `mod.rs`：原有 SRTOOLS 筛选范围内的 Rust 类型定义。

任一阶段报错即停止后续阶段，详情写入 `DUMP/dump.log` 和 `DUMP/report.json`。
`schema`、协议与表数据先生成 `.partial` 文件，成功后再替换正式文件；失败时旧文件可能仍在，判断本次结果请以报告为准。

协议结构和 CmdId 校验通过后，会先发布 `StarRail.core.proto` 与 `packetIds.core.json`，
再执行 HSR-OWNER 的辅助名称恢复。核心文件不是完整名称恢复成功的标志；后续异常仍将 `proto`
标记为失败并停止，但已校验的核心结果会保留。正式的 `StarRail.proto` 仅在整个阶段成功后替换。

`proto` 不再依赖额外的全量方法元数据扫描：

- 从真实注册函数恢复 2690 个 CmdId，其中响应/通知 1531 个、请求 1159 个；发送调用只补名称和处理器 RVA，不决定覆盖范围。
- 解析 1068 个 XLua 委托绑定；需要引用槽时只调用已验证的 metadata usage 初始化器，不执行发送函数或协议注册类的静态构造函数，不修改 once 标志。
- 默认使用 `NativeGetters`。4.6.51 的协议属性表被剥离，字段由常量声明顺序、原生 getter 的实例偏移、oneof 判别值和 `WriteTo` tag 交叉恢复。
- 离线核对覆盖 4931 个消息、14072 个字段、175 个含 oneof 的消息和 653 个枚举；识别 9 个 `sint32` 字段，不能仅按 CLR 的 `int` 输出。
- 字段名缺少属性来源时保留实例字段名，oneof 使用判别枚举成员名。嵌套类型保留父路径，名称冲突保留原名；输出前检查字段名、类型引用和 CmdId 覆盖。
- 原生方法列表包含构造函数，不能直接把所有条目当作 `MonoMethod` 调用返回类型槽。战斗回调先按名称筛选，再读取原生返回类型；反射封装通过原生签名处理构造函数，并拒绝空返回类型。
- Avatar 等回调名称从已有原生方法缓存统一筛选，不再为每个名称遍历全部类型；每组名称恢复记录开始、完成和耗时，原生异常包含所属模块及 RVA。
- 核心 wire model 只构建一次，所有 NT 来源提交候选，由统一决策应用到正式输出。分类顺序、参数数量和未经控制流验证的旧线性追踪只作提示，不能单独补名；冲突保留原名。
- `packetIds.json` 的值对应最终 proto 定义名，`packetIds.obfuscated.json` 保留原始混淆名；处理器映射也同步最终名称。

原生扫描读取 PE 异常目录中的真实函数范围，不依赖 `.pdata` 节名或相邻 `PUSH`。
开放泛型的 RVA 为零是合法情况，不再对零地址减模块基址。
协议、schema 和 C# 都会记录阶段进度；未知布局会带具体类型、字段或 RVA 报错，不把缺失字段当作成功结果。

`schema` 的二进制读取器按 `LockTargetParam&` 反序列化签名和唯一 `byte[]` 字段定位，不取方法数组首项。
默认不启用网关联网查询。只有显式构建 `--features gateway-network` 才编译该可选功能。

## NT 名称恢复

NT 恢复随 `proto` 阶段执行，不需要额外配置，也不会在运行时读取外部协议参考目录。

GateServer 支持 LunarCore 所用非 Unk 字段和 `use_tcp` 的专用原生恢复规则，使用当前元数据、IPv4 分支和密钥消费链定位，不固定字段编号或偏移。两份客户端样本的静态回放与适用边界见 [GateServer NT 笔记](notes/gate-server-nt.md)。
2026-10-07 的 [NT 冲突修复记录](notes/nt-conflicts.md) 覆盖响应回调 CFG、GridFight 参数变更及 GateServer bool 别名。用户列出的 20 条冲突已在静态回放中解决；其中 11 个响应名称是明确标记的声明类型作用域导出别名，不是官方原名。
随后 11:46 的真实运行已确认这些冲突消失；[最新运行复查](notes/nt-latest-run.md) 又修复三条参数锚点及四条 IdleLive 诊断漏项。更新静态回放新增 7 个消息选择，未决组 4033 -> 4029。
只导出协议时沿用现有阶段配置：

```toml
[output]
stages = ["proto"]
```

LunarCore 和历史 DUMP 仅用于开发时对照，不是导出器的运行时依赖。
名称来自客户端证据和经校验的作用域规则；同类型重复字段不按顺序配对，
共享请求的全部业务别名保留为候选，`Receive` 等通用回调不能独立命名。
内置 `protox` 仅校验生成的 proto，不需要游戏运行时启动 Python 或 protoc。

- `nt-report.json`：输入 SHA-256、候选方法/RVA、证据等级、接受/拒绝原因和消息/枚举/字段确认及未决数量。最终发布成功才标记 `completed`。
- `nt.scoped.txt`：类型使用原始完整路径，字段使用 `原始完整消息名#字段编号`，包括 map 和 oneof 成员。
- `nt.txt`：保留两列格式，仅输出在所有出现位置都可安全使用的映射。`method_nt.txt` 只输出最终接受的类型映射。

正式 proto、CmdId 和 handler 映射使用同一决策。中间阶段不重复打印名称碰撞；完整候选和拒绝原因
仍在报告中，不能把控制台警告变少当作识别成功。不同作用域的同名字段不会互相传播。

只读描述符审计工具需要本机 Python `protobuf` 和 `protoc`，仅审计已有输出，不恢复或改写名称：

```powershell
py -3.14 tools/audit_nt.py --core DUMP/StarRail.core.proto --proto DUMP/StarRail.proto --ids DUMP/packetIds.json --core-ids DUMP/packetIds.core.json
```

此前真实文件回放、27 组歧义和 14 个碰撞类型的保留依据见 `notes/nt-recovery.md`。
其中参考库实验的增量是历史结果，不代表当前不加载参考库的版本。
静态回放不是新游戏运行。历史多个 DUMP 核心哈希相同，不构成多版本样本；后续 GateServer 和响应规则另以两份不同完整 DLL 做了定向跨版本观测。
NT 记录只反映名称恢复层；后续运行时定位改造见下节，也不承诺任意未知 IL2CPP 布局自动适配。

## 动态定位

- Unity API 表从根域初始化调用点的 RIP-relative MOV 定位；LEA 只是域名字符串参数。校验完整表位于可读写、不可执行区，字符串位于只读区，拒绝多候选，不再读固定 RVA。
- 85 个反射绑定从实时类名、方法名、参数数量及必要的完整参数类型唯一匹配，不信任混淆 flags 或方法序号。
- 协议注册类按字典类型、`.cctor` 注册数据流识别；XLua 缓存要求有 `Send...Req` 字段，并核对全部缓存绑定。
- 请求入口按 `NetworkManager` 签名、返回 `NetPacket` 的构建方法及调用关系恢复，不固定混淆方法名。
- 元数据初始化器按解码算法特征定位，从代码推导全局指针、header 和 usage 表成员偏移；数组边界来自实际 usage lists。
- C# 直接遍历已初始化的完整类型缓存，不读取 Morax 输出、profiles 或 Script usage 快照。

两份完整 DLL 的元数据解码、API 表定位及反射签名已离线核对；
4.6.51 的协议注册数据流另经生产扫描器回放。算法族变化、API 槽语义或底层 ABI 变化仍可能需要适配。
具体计数、复查命令与验证边界见 [动态扫描记录](notes/dynamic-discovery.md)。

## Morax 元数据导出

Morax 已回退本轮动态计划改造，保留原来的静态解析实现。它读取指定目录的原始 DLL、
`global-metadata.dat` 和 `startup-metadata.dat`，不调用 usage 初始化器、不依赖 Dumper 或 profiles。
无需加载游戏，直接使用命令行：

```powershell
cargo build -p morax --release --offline --locked
./target/release/morax.exe D:/Games/StarRail/StarRail_4.6.51 D:/Project/CastoriceDumper/target/morax-current-verified
./target/release/morax.exe D:/Games/StarRail/StarRail_4.6.51 --inspect
```

产物是 `dump.cs`、`script.json`、`il2cpp.h`、`stringLiterals.json`、`DummyDll/` 和
`morax-report.json`。报告逐阶段落盘，记录 usage 覆盖率、参数常量和未恢复字段初始化数据。
文件使用 `.partial` 发布；DummyDll 整个目录成功生成后才发布，已有目录保留为
`DummyDll.previous-<run_id>`。失败后的旧文件不代表本次成功，请核对报告。

2026-10-07 回退后重新完整离线导出，六阶段均 completed；报告在 `target/morax-current-verified/morax-report.json`。
覆盖 143 个输入程序集、84,933 个类型、1,118,058 个脚本方法地址，
生成 144 个 DummyDll（含辅助属性程序集）。独立 .NET 元数据审计确认 687,739 行参数、
13,649 个参数常量；被剥离的参数名不伪造恢复，C# 中使用 `pN` 占位。
仍有 716 项字段初始化数据未恢复，方法体只是占位，原始属性、泛型约束、marshal 描述和
原生结构 ABI 尚未完整恢复。报告的 completed 表示离线导出流程成功，不表示完整运行时 Dump。

## 离线验证

工具使用本机 Python 的 `pefile`、`capstone`；可选仿真工具还需要 `unicorn`。
大文件通过内存映射和局部反汇编读取，不需要 IDA 数据库。

```powershell
cargo build -p dumper --examples --release --offline --locked
./target/release/examples/inspect_runtime.exe D:/Games/StarRail/StarRail_4.6.51 target/runtime-discovery/new 0x3EAE330
./target/release/examples/inspect_runtime.exe 'D:/Games/StarRail/Star Rail Game' target/runtime-discovery/old
py -3.14 tools/replay_dynamic_protocol.py D:/Games/StarRail/StarRail_4.6.51 target/runtime-discovery/new --allocation-wrapper-rva 0x3EAE220
py -3.14 tools/analyze_fields.py D:\Games\StarRail\StarRail_4.6.51 --output target/field-analysis.json
cargo check --workspace --all-features --offline --locked
cargo build --release --offline --locked
./tools/inspect_dummy_dll.ps1 D:/Project/CastoriceDumper/target/morax-current-verified/DummyDll
py -3.14 tools/inspect_client.py D:/Games/StarRail/StarRail_4.6.51 --rva 0x3F23DD0 --function --compare D:/Games/StarRail/StarRail_4.6.51/GameAssembly-mini2.dll
py -3.14 tools/inspect_client.py D:/Games/StarRail/StarRail_4.6.51 --module UnityPlayer.dll --rva 0xF5F907 --function
```

上述命令中的 wrapper RVA 仅供对已观测 DLL 的离线回放；Dumper 运行时从 live API 表取得入口，不使用这些地址。
反射绑定在运行时读取实时元数据，不读取本地 `dump.cs` 或旧 binding JSON；历史分析工具和 `profiles/` 仅保留作开发对照。
匹配失败不会试调用相邻索引。
`emulate_api_table.py` 仅用于有时间和指令上限的诊断，未完成的仿真输出不能作为有效 API 表。
`inspect_client.py --function` 从原始 PE 异常目录取函数边界，不使用 mini 的分析结果。
`inspect_dummy_dll.ps1` 需要 PowerShell 7，使用 `System.Reflection.Metadata` 审计生成物，
不加载或执行生成的程序集；它不是完整 CLR 可加载性或原生 ABI 验证。
项目不保留新增单元测试或伪造 fixture；上述分析直接读取本地真实 DLL 和运行时发现结果。
关键 RVA、字段恢复依据与限制记录在 `notes/4.6.51.md`。

**验证边界：** 上述修复的离线检查与编译成功不等于新的完整导出已经成功。
替换 DLL 后需重启游戏进程加载新版，再核对本次的两个报告、日志与 proto；旧进程不会自动使用新版。
导出仍会调用游戏方法，应在资源就绪后验证，不要在工作线程结束前卸载 DLL。

## 许可

MIT。原作者：Kain。迁入代码的 HSR-OWNER 许可保留在 `crates/LICENSE.HSR-OWNER`。
