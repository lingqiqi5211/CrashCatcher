# 崩溃捕手 · CrashCatcher

记录应用崩溃、ANR 与原生崩溃的 root 模块。守护进程以 root 常驻，**不向任何进程注入代码**。

[![CI](https://github.com/lingqiqi5211/CrashCatcher/actions/workflows/ci.yml/badge.svg)](https://github.com/lingqiqi5211/CrashCatcher/actions/workflows/ci.yml)

## 为什么又写一个

对照物是 [AppErrorsTracking](https://github.com/KitsunePie/AppErrorsTracking)——它好用，但从 system_server 内部 Hook ActivityManager，于是每个 Android 大版本都要跟着适配，还得先备好 Zygisk 与 Xposed。

这个换了条路：崩溃信息平台本来就对 root 公开——事件与崩溃日志缓冲、DropBox、tombstone、ANR 转储。守护进程只是把这几个口子读全并合成一条记录。Android 升级改变的是**要解析什么**，而不是**代码住在哪里**，所以不需要 Zygisk、不需要 Xposed，也不用等适配。

顺带能捕到一类别的工具看不到的崩溃：**应用自己装了 `UncaughtExceptionHandler` 并吞掉的异常**——崩溃日志缓冲里有 `FATAL EXCEPTION`，而事件缓冲里没有对应的 `am_crash`，两者一对照就认出来了。

## 它做什么

- Java 异常、ANR、原生崩溃（tombstone）、WTF，以及上面说的「应用自行处理」的异常
- 按指纹分组，同一个 bug 的多次发生归一条，多个采集源看到的同一次崩溃合并成一条记录
- 堆栈按框架栈帧折叠（几十帧常常收成几行）、可缩放、可选中复制，导出成带环境信息的文本
- 崩溃提醒可选通知或替换系统「已停止运行」弹窗；可按应用静音
- SQLite 索引 + zstd 压缩正文分离存储，列表页只读索引，所以历史一万条和十条打开一样快
- Material Expressive / Miuix 双风格界面（UI 库 [MeowUI](https://github.com/lingqiqi5211/MeowUI)）

## 安装

需要 Magisk / KernelSU / APatch 任一。

1. 从 [Releases](https://github.com/lingqiqi5211/CrashCatcher/releases) 下最新的 `CrashCatcher-module-*.zip` 与 `CrashCatcher-*.apk`
2. 在 root 管理器里刷入 zip，重启
3. 安装同一次发布的 APK

**两个文件必须同批**：模块里 pin 了管理器的签名证书，守护进程据此在 socket 上鉴权，混用会连不上（界面显示「未连接」）。模块是否正常运行直接看 root 管理器里的模块描述，它会写成 `[ ✅ 运行中 ]`。

## 从源码构建

```bash
git clone --recurse-submodules https://github.com/lingqiqi5211/CrashCatcher.git
cd CrashCatcher
```

需要 JDK 21+、Android SDK 与 NDK、Rust（工具链由 `rust-toolchain.toml` 钉住，rustup 会自动装）。

```bash
# 管理器 APK（release 需要签名配置，见下）
cargo run --release -p cch_packager -- manager-apk

# 模块 zip：三个 ABI 的守护进程 + 桥 dex + 从该 APK 推出的签名 pin
cargo run --release -p cch_packager -- module --manager-apk dist/crashcatcher.apk
```

签名配置放在 `apps/manager/keystore.properties`（不入库）：

```properties
storeFile=/path/to/your.jks
storePassword=…
keyAlias=…
keyPassword=…
```

**debug 构建也用这把 release key**：守护进程只认 pin 住的那张证书，用调试签名签出来的管理器连不上 socket。

发布版本号在 [version.properties](version.properties) 一处，Gradle 与打包器都读它，`versionCode` 由它推导。

## 结构

| 目录 | 内容 |
| --- | --- |
| `crates/` | Rust：守护进程、各采集器、存储、协议、签名鉴权、打包工具 |
| `bridge/` | `app_process` 起的特权 Java 桥：通知、包名标签、跨用户启动 Activity |
| `module/` | 模块脚手架：`service.sh`、`customize.sh`、状态写回 `module.prop` |
| `apps/manager/` | Compose 管理器 |
| `apps/crashdemo/` | 故意崩溃的测试 app，一个按钮对应一条采集路径 |

设计取舍与踩过的坑记在 [AGENTS.md](AGENTS.md)。

## 许可

暂未确定，发布前会补上。

## 致谢

[AppErrorsTracking](https://github.com/KitsunePie/AppErrorsTracking) 是本项目的对照与灵感来源；界面用 [MeowUI](https://github.com/lingqiqi5211/MeowUI) 与 [miuix](https://github.com/compose-miuix-ui/miuix)；存储用 [rusqlite](https://github.com/rusqlite/rusqlite) 与 [zstd](https://github.com/facebook/zstd)。
