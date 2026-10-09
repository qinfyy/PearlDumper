# PearlDumper

支持版本：`CNBETAWin4.6.51`

## 输出

文件写入游戏工作目录的 `DUMP/`，导出阶段由 `Config.toml` 的 `[output].stages` 配置。

| 阶段 | 主要文件 | 内容 |
| --- | --- | --- |
| `csharp` | `dump.cs` | 程序集索引、类型、字段及偏移、构造函数、方法签名和 RVA/VA |
| `proto` | `StarRail.proto`、`packetIds.json` | 协议定义、消息名称和命令 ID |
| `proto` 名称恢复 | `nt.txt`、`nt.scoped.txt`、`method_nt.txt`、`nt-report.json` | NT 名称映射及恢复报告 |
| `schema` | `data.json`、`excel_paths.json`、`mod.rs` | 数据结构、资源路径和 Rust 类型定义 |

另有 `dump.log`（日志）、`report.json`（阶段状态）、`methods.json` 和 `methods2.json`（方法地址映射）。

## Morax 输出

```text
morax GAME_DIR [OUTPUT_DIR] [--inspect]
```

`GAME_DIR` 为游戏目录；`OUTPUT_DIR` 可选，默认 `GAME_DIR/DUMP/Morax/`；`--inspect` 仅查看元数据信息，不导出文件。

| 文件 | 内容 |
| --- | --- |
| `dump.cs` | 类型、成员、偏移和方法地址 |
| `script.json` | 方法、元数据和字符串地址映射 |
| `il2cpp.h` | 原生类型结构声明，布局需另行验证 |
| `stringLiterals.json` | 字符串字面量及地址 |
| `DummyDll/` | 供反编译浏览的程序集，方法体为占位实现 |
| `morax-report.json` | 导出状态、统计和限制说明 |
