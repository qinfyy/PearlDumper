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

默认执行 `proto,schema,excel`。通过游戏进程的 `CASTORICE_DUMP` 环境变量选择阶段：

```powershell
$env:CASTORICE_DUMP = "schema"
```

必须让游戏进程从设置了该变量的环境启动；修改终端变量不会影响已经运行的游戏。

| 阶段 | 输出 |
| --- | --- |
| `schema`（别名 `parser`） | `DUMP/data.json`、`DUMP/excel_paths.json`、`DUMP/mod.rs` |
| `proto` | `DUMP/StarRail.proto`、`DUMP/packetIds.json`，以及处理器映射 |
| `excel` | `DUMP/Resources/ExcelOutput/*.json` |
| `config` | `DUMP/Resources/` 下的配置，范围沿用 HSR-OWNER |
| `textmap` | `DUMP/Resources/TextMap/TextMapEN.json`，沿用原有单语言入口 |
| `csharp` | C# 类型导出 |
| `script` / `script_v2` | 元数据、脚本映射及结构信息，依赖运行时扫描结果 |

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

原生扫描读取 PE 异常目录中的真实函数范围，不依赖 `.pdata` 节名或相邻 `PUSH`。
开放泛型的 RVA 为零是合法情况，不再对零地址减模块基址。
协议、schema 和 Excel 都会记录阶段进度；未知布局会带具体类型、字段或 RVA 报错，不把缺失字段当作成功结果。

`schema` 的二进制读取器按 `LockTargetParam&` 反序列化签名和唯一 `byte[]` 字段定位，不取方法数组首项。
`excel` 按无参 `FixEnumerator` 签名定位表枚举器，不依赖混淆的方法标志，也不卸载或改写游戏中的表路径。

默认不启用网关联网查询。只有显式构建 `--features gateway-network` 才编译该可选功能。

## 离线验证

工具使用本机 Python 的 `pefile`、`capstone`；可选仿真工具还需要 `unicorn`。
大文件通过内存映射和局部反汇编读取，不需要 IDA 数据库。

```powershell
py -3.14 tools/inspect_client.py D:\Games\StarRail\StarRail_4.6.51 --check profiles/4.6.51.json
py -3.14 tools/resolve_bindings.py D:\Games\StarRail\StarRail_4.6.51 --check
py -3.14 tools/analyze_protocol.py D:\Games\StarRail\StarRail_4.6.51 --check-profile profiles/4.6.51-protocol.json --output target/protocol-analysis.json
py -3.14 tools/analyze_fields.py D:\Games\StarRail\StarRail_4.6.51 --output target/field-analysis.json
cargo check --workspace --all-features --offline --locked
cargo build --release --offline --locked
```

反射绑定使用本地 `dump.cs` 的方法名、参数列表和 RVA，不信任其中的混淆标志位或方法排列顺序。
运行时再次验证绑定对应的方法与参数数量，失败不会试调用相邻索引。
`emulate_api_table.py` 仅用于有时间和指令上限的诊断，未完成的仿真输出不能作为有效 API 表。
项目不保留新增单元测试或伪造 fixture；上述分析直接读取本地真实 DLL、`dump.cs` 和 `DUMP/methods*.json`。
关键 RVA、字段恢复依据与限制记录在 `notes/4.6.51.md`。

**验证边界：** 离线检查与编译成功不等于注入验证。本次尚未在游戏进程中验证实际导出，
不能据此认定闪退已消除。导出仍会调用游戏方法，应在资源就绪后测试，不要在工作线程结束前卸载 DLL。

## 许可

MIT。原作者：Kain。迁入代码的 HSR-OWNER 许可保留在 `crates/LICENSE.HSR-OWNER`。
