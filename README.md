# CastoriceDumper

基于 HSR-OWNER 的 IL2CPP 运行时导出器，构建产物为 Windows x64 DLL `astrolabe.dll`。
当前适配目标是本地 **StarRail 4.6.51**，运行时按 `profiles/4.6.51.json` 检查已加载模块的 PE 头。

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
启动时不再读取整个游戏 DLL 或计算 SHA-256；完整文件指纹检查保留在离线工具中。
PE 头只能排除明显不匹配的客户端，不等同于完整文件指纹检查。
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
stages = ["proto", "schema", "excel", "config"]
```

可选阶段为 `proto`、`schema`、`excel`、`config`、`textmap`、`csharp`、`script`、`script_v2` 和 `metadata`。

必须让游戏进程从设置了该变量的环境启动；修改终端变量不会影响已经运行的游戏。

| 阶段 | 输出 |
| --- | --- |
| `schema`（别名 `parser`） | `DUMP/data.json`、`DUMP/excel_paths.json`、`DUMP/mod.rs` |
| `proto` | `DUMP/StarRail.proto`、`DUMP/packetIds.json`、`DUMP/packetIds.obfuscated.json`，以及处理器映射 |
| `excel` | `DUMP/Resources/ExcelOutput/*.json`、`DUMP/excel_report.json` |
| `config` | `DUMP/Resources/` 下的配置，范围沿用 HSR-OWNER |
| `textmap` | `DUMP/Resources/TextMap/TextMapEN.json`，沿用原有单语言入口 |
| `csharp` | C# 类型导出 |
| `script` / `script_v2` | 元数据、脚本映射及结构信息，依赖运行时扫描结果 |
| `metadata` | 使用迁入的 morax，从原始文件离线导出到 `DUMP/Morax/` |

`schema` **保持 HSR-OWNER 的格式**，不是旧版 `ExcelSchema.json`：

- `data.json`：以类型名为键，保留 `Class`、`Struct`、`Typeindex`、`Enum` 类型标签及 `fields`、`interfaces`、`descendants` 等原有结构。
- `excel_paths.json`：类型名到 `BakedConfig/ExcelOutput/*.bytes` 等配置路径的映射。
- `mod.rs`：原有 SRTOOLS 筛选范围内的 Rust 类型定义。

任一阶段报错即停止后续阶段，详情写入 `DUMP/dump.log` 和 `DUMP/report.json`。
`schema`、协议与表数据先生成 `.partial` 文件，成功后再替换正式文件；失败时旧文件可能仍在，判断本次结果请以报告为准。

协议结构和 CmdId 校验通过后，会先发布 `StarRail.core.proto` 与 `packetIds.core.json`，
再执行 HSR-OWNER 的辅助名称恢复。核心文件不是完整名称恢复成功的标志；后续异常仍将 `proto`
标记为失败并停止，但已校验的核心结果会保留。正式的 `StarRail.proto` 仅在整个阶段成功后替换。

`proto` 不再依赖 `script` 阶段的 `TYPE_INFOS` 或额外的全量方法元数据扫描：

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
协议、schema 和 Excel 都会记录阶段进度；未知布局会带具体类型、字段或 RVA 报错，不把缺失字段当作成功结果。

`schema` 的二进制读取器按 `LockTargetParam&` 反序列化签名和唯一 `byte[]` 字段定位，不取方法数组首项。
`excel` 按无参 `FixEnumerator`、`AdapterEnumerator` 或 `AdapterEnumerator64` 签名定位表枚举器，
不依赖混淆的方法标志。首次导出前等待 `ExistsDesignData` 确认资源可读，最多 120 秒。
单个逻辑文件使用工厂自身的 lazy loader；同名 client/GameCore 数据层保留路径顺序。
多个不同逻辑文件临时切换来源路径，调用工厂原生 CALL 对应的 loader，结束后恢复原路径和数据。
loader 自身清理旧数据，不强制要求表存在 `UnloadData`。value type 枚举器使用 IL2CPP unbox，
实际导出行数必须与枚举器 `Count` 一致。

`excel_report.json` 记录期望文件名、成功文件及行数、未配置路径的表和逐表失败原因。
只统计成功发布的文件，任何失败或覆盖不完整都会使阶段失败；不能用目录中残留的旧文件判断完成。
动态 Textmap 的空路径不生成 Excel 文件，Textmap 仍通过独立的 `textmap` 阶段导出。

默认不启用网关联网查询。只有显式构建 `--features gateway-network` 才编译该可选功能。

## NT 名称恢复

NT 恢复随 `proto` 阶段执行，不需要额外配置，也不会在运行时读取外部协议参考目录。
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
静态回放不是新游戏运行，当前多个 DUMP 核心哈希相同，不构成多版本样本。
本次只改善名称恢复层，不承诺未知版本的 IL2CPP 布局或注册地址自动适配。

## Morax 元数据导出

4.6.51 的旧 `script` 阶段仍存在扫描失败，旧 `csharp` 还依赖它的缓存。
当前建议使用 `metadata` 获取元数据产物，不必先运行这三个旧阶段：

```toml
[output]
stages = ["metadata"]
```

此阶段从实际加载的 `GameAssembly.dll` 所在目录读取原始 DLL、`global-metadata.dat` 和
`startup-metadata.dat`，不调用 usage 初始化器。单独选择它时不执行 IL2CPP 反射初始化；
DLL 加载器仍会等待两个游戏模块。需要完全离线运行时，直接使用命令行：

```powershell
cargo build -p morax --release --offline --locked
./target/release/morax.exe D:/Games/StarRail/StarRail_4.6.51 D:/Games/StarRail/StarRail_4.6.51/DUMP/Morax
./target/release/morax.exe D:/Games/StarRail/StarRail_4.6.51 --inspect
```

产物是 `dump.cs`、`script.json`、`il2cpp.h`、`stringLiterals.json`、`DummyDll/` 和
`morax-report.json`。报告逐阶段落盘，记录 usage 覆盖率、参数常量和未恢复字段初始化数据。
文件使用 `.partial` 发布；DummyDll 整个目录成功生成后才发布，已有目录保留为
`DummyDll.previous-<run_id>`。失败后的旧文件不代表本次成功，请核对报告。

2026-10-05 实际离线导出覆盖 143 个输入程序集、84,933 个类型、1,118,058 个脚本方法地址，
生成 144 个 DummyDll（含辅助属性程序集）。独立 .NET 元数据审计确认 687,739 行参数、
13,649 个参数常量；被剥离的参数名不伪造恢复，C# 中使用 `pN` 占位。
仍有 716 项字段初始化数据未恢复，方法体只是占位，原始属性、泛型约束、marshal 描述和
原生结构 ABI 尚未完整恢复。报告的 completed 表示离线导出流程成功，不表示完整运行时 Dump。

## 离线验证

工具使用本机 Python 的 `pefile`、`capstone`；可选仿真工具还需要 `unicorn`。
大文件通过内存映射和局部反汇编读取，不需要 IDA 数据库。

```powershell
py -3.14 tools/inspect_client.py D:\Games\StarRail\StarRail_4.6.51 --check profiles/4.6.51.json
py -3.14 tools/resolve_bindings.py D:\Games\StarRail\StarRail_4.6.51 --check
py -3.14 tools/analyze_protocol.py D:\Games\StarRail\StarRail_4.6.51 --check-profile profiles/4.6.51-protocol.json --output target/protocol-analysis.json
py -3.14 tools/analyze_fields.py D:\Games\StarRail\StarRail_4.6.51 --output target/field-analysis.json
py -3.14 tools/analyze_excel.py --game-dir D:\Games\StarRail\StarRail_4.6.51
cargo check --workspace --all-features --offline --locked
cargo build --release --offline --locked
./tools/inspect_dummy_dll.ps1 D:/Games/StarRail/StarRail_4.6.51/DUMP/Morax/DummyDll
py -3.14 tools/inspect_client.py D:/Games/StarRail/StarRail_4.6.51 --rva 0x3F23DD0 --function --compare D:/Games/StarRail/StarRail_4.6.51/GameAssembly-mini2.dll
py -3.14 tools/inspect_client.py D:/Games/StarRail/StarRail_4.6.51 --module UnityPlayer.dll --rva 0xF5F907 --function
```

反射绑定使用本地 `dump.cs` 的方法名、参数列表和 RVA，不信任其中的混淆标志位或方法排列顺序。
运行时再次验证绑定对应的方法与参数数量，失败不会试调用相邻索引。
`emulate_api_table.py` 仅用于有时间和指令上限的诊断，未完成的仿真输出不能作为有效 API 表。
`inspect_client.py --function` 从原始 PE 异常目录取函数边界，不使用 mini 的分析结果。
`inspect_dummy_dll.ps1` 需要 PowerShell 7，使用 `System.Reflection.Metadata` 审计生成物，
不加载或执行生成的程序集；它不是完整 CLR 可加载性或原生 ABI 验证。
项目不保留新增单元测试或伪造 fixture；上述分析直接读取本地真实 DLL、`dump.cs` 和 `DUMP/methods*.json`。
关键 RVA、字段恢复依据与限制记录在 `notes/4.6.51.md`。

**验证边界：** 2026-10-04 的旧版本真实日志完成 proto 和 schema，但 Excel 仅生成 643 个文件，
1426 张表失败，旧报告仍误写完成。上述修复的离线检查与编译成功不等于新的完整导出已经成功。
替换 DLL 后需重启游戏进程加载新版，再核对本次的两个报告、日志与 proto；旧进程不会自动使用新版。
导出仍会调用游戏方法，应在资源就绪后验证，不要在工作线程结束前卸载 DLL。

## 许可

MIT。原作者：Kain。迁入代码的 HSR-OWNER 许可保留在 `crates/LICENSE.HSR-OWNER`。
