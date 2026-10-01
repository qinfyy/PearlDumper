# CastoriceDumper

基于 HSR-OWNER 的 IL2CPP 运行时导出器，构建产物为 Windows x64 DLL `astrolabe.dll`。
当前适配目标是本地 **StarRail 4.6.51**，运行时按 `profiles/4.6.51.json` 检查已加载模块的 PE 头。

## 构建

```powershell
cargo build --release --locked
```

产物：`target/release/astrolabe.dll`。这是 DLL，不支持 `cargo run`。

已迁入 `crates/il2cpp`、`crates/reflection`、`crates/derive`、`crates/utils` 和 `crates/dumper`。
旧的 `src/core`、`src/il2cpp`、`src/output` 仅保留供对照，不参与构建。

## 导出

DLL 加载后创建工作线程，等待模块与 IL2CPP 就绪，检查已加载的 PE 头，随后执行导出。
启动时不再读取整个游戏 DLL 或计算 SHA-256；完整文件指纹检查保留在离线工具中。
PE 头只能排除明显不匹配的客户端，不等同于完整文件指纹检查。
日志与导出结果位于**游戏进程工作目录**的 `DUMP/`，控制台不显示进度条。

启动日志包含实际加载的导出器路径、Debug/Release 构建类型及工作目录。
API 就绪检查会记录槽位错误、domain 和程序集数量；等待和长循环约每 2 秒输出一次 INFO 状态，
类型扫描、方法映射写入、反射绑定及自检均有开始与完成信息。INFO 同时显示在控制台和日志文件中，
DEBUG 细节仅写入文件；文件日志包含毫秒时间戳和线程 ID。
若单次原生调用阻塞，循环无法继续输出进度，可用该调用前的日志定位；这些日志不保证中断原生调用。
`methods.json` / `methods2.json` 流式写入 `.partial` 文件，完成后替换正式文件。

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

默认不启用网关联网查询。只有显式构建 `--features gateway-network` 才编译该可选功能。

## 离线验证

工具使用本机 Python 的 `pefile`、`capstone`；可选仿真工具还需要 `unicorn`。
大文件通过内存映射和局部反汇编读取，不需要 IDA 数据库。

```powershell
py -3.14 tools/inspect_client.py D:\Games\StarRail\StarRail_4.6.51 --check profiles/4.6.51.json
py -3.14 tools/resolve_bindings.py D:\Games\StarRail\StarRail_4.6.51 --check
py -3.14 -m unittest discover -s tools -p "test_*.py" -v
cargo test --workspace --offline --locked
```

反射绑定使用本地 `dump.cs` 的方法名、参数列表和 RVA，不信任其中的混淆标志位或方法排列顺序。
运行时再次验证绑定对应的方法与参数数量，失败不会试调用相邻索引。
`emulate_api_table.py` 仅用于有时间和指令上限的诊断，未完成的仿真输出不能作为有效 API 表。

**验证边界：** 离线检查与编译成功不等于注入验证。本次尚未在游戏进程中验证实际导出，
不能据此认定闪退已消除。导出仍会调用游戏方法，应在资源就绪后测试，不要在工作线程结束前卸载 DLL。

## 许可

MIT。原作者：Kain。迁入代码的 HSR-OWNER 许可保留在 `crates/LICENSE.HSR-OWNER`。
