# module/

Magisk / KernelSU / APatch 模块模板。打包时由 `cch_packager` 原样复制，并改写
`module.prop` 的 `version` 为 `<packager 版本>-<git7>`。

## 命名（已定，勿随意改）

| 项 | 值 |
| --- | --- |
| 模块 id | `crash.catcher` |
| 模块名 | `崩溃捕手 (CrashCatcher)` |
| 作者 | `lingqiqi5211` |
| 管理器包名 | `io.github.lingqiqi5211.crashcatcher` |
| Rust crate 前缀 | `cch_` |
| daemon 二进制 | `catcherd` |
| 持久化目录 | `/data/adb/crash.catcher/` |
| 模块目录 | `/data/adb/modules/crash.catcher/` |

`id` 一旦发布就不能改 —— 它同时是模块目录名和持久化目录名。

## `description` 的运行状态前缀（契约）

`module.prop` 里的 `description` **必须**以 `[ 状态 ] ` 开头。root 管理器（Magisk /
KernelSU / APatch 的模块列表）直接显示这个字段，所以它是用户不打开管理器应用就能看到
运行状态的唯一位置。

`service.sh` 在每次改变状态时重写这一行（读原文件、替换首个 `[...]`、原子写回）。
**状态带图标**，让人在模块列表里不用读字就能分辨：

| 状态 | 含义 |
| --- | --- |
| `[ ⏳ 未启动 ] ` | 打包时的初始值。重启后仍是它，说明 `service.sh` 没跑起来 |
| `[ ✅ 运行中 ] ` | daemon 已启动且已监听 socket，各采集源都有数据 |
| `[ ⚠️ 采集受限 ] ` | daemon 在跑，但有采集源拿不到数据（例如 `dropbox:<tag>` 被关） |
| `[ ❌ 已停止 ] ` | daemon 退出且未能重启 |
| `[ 🚫 已禁用 ] ` | boot guard 判定上次启动未完成，已自禁用 |

改写只动方括号内的部分，后面的正文保持不变。正文要短 —— 模块列表里是单行显示，
长了会被截断。

改写实现注意两点：图标是多字节 UTF-8，替换时按「第一个 `]` 之前的全部内容」定位，
不要按字符数偏移；写回要走临时文件 + `mv` 原子替换，避免 root 管理器读到半截文件。

> 这部分脚本属于采集/打包工作流的范围；本文件只固定命名与 `description` 契约。
