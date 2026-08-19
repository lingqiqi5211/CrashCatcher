# CrashCatcher

记录应用崩溃、ANR 与原生崩溃的 root 模块。三块：`crates/` 是 root 守护进程与其库（Rust workspace，edition 2024）、`bridge/` 是 `app_process` 起的特权 Java 桥、`apps/manager` 是 Compose 管理器（AGP 9 内置 Kotlin，UI 走 MeowUI 复合构建）。`apps/crashdemo` 是故意崩溃的测试 app。

本文件是 agent 指南的入口，`CLAUDE.md` 只有一行 `@AGENTS.md`——Claude Code 只自动读 `CLAUDE.md`。

**对照物是 [AppErrorsTracking](https://github.com/KitsunePie/AppErrorsTracking)**：它从 system_server 里 Hook ActivityManager，所以每个 Android 大版本都要适配，且依赖 Zygisk + Xposed。本项目**零注入**——不进任何进程，只读平台已经对 root 公开的那几个口子。看到「要不要 Hook 一下」的念头时先回到这条。

## 工作规程

- **改完先编译再提交**，用户明确要求过。Kotlin 侧 `apps/manager` 跑 `./gradlew :app:assembleDebug`（分钟级，release 走 R8 更久）；Rust 侧 `.\tools\cargo.ps1 clippy --workspace --all-targets` + `test --workspace`。
- **Windows 上 cargo 必须经 `tools\cargo.ps1`**：MSVC Build Tools 不在 PATH，裸 `cargo test` 链接失败（Git Bash 下 `link` 还会解析成 coreutils 的 `link`）。Android 交叉编译走 NDK clang，不受影响。
- **clippy 在本仓禁 `unwrap`/`expect`/`panic`**（见 [clippy.toml](clippy.toml)），CI 是 `-D warnings`。守护进程以 root 跑在别人手机上，一个 panic 就是崩溃记录静默停摆。
- **工具链钉在 [rust-toolchain.toml](rust-toolchain.toml)**：edition 2024 下 rustfmt 的 import 排序会随版本漂移，不钉就会出现「本地格式化过、CI 仍报 fmt 失败」。要升先改这里，再 `cargo fmt --all` + clippy 一起提交。
- submodule 是整个 UI 层（MeowUI，内含 miuix），首次 clone 后 `git submodule update --init --recursive`。
- 保留用户未提交的改动；不用破坏性 reset/checkout；不改也不输出 `local.properties` 与 `keystore.properties`。
- **提交信息不带协作者标识**，作者 `柒柒喵 <lingqiqi233@gmail.com>`。`<scope>: <summary>`，scope 取 `daemon`/`bridge`/`module`/`manager`/`demo`/`build`/`ci`/`docs`；主题行 ≤ 72 字符、无句尾句号；**body 只讲代码里看不出的根因与取舍**，不逐文件复述 diff。
- `docs/` 与 `dist/` 不入库（前者是本地设计稿，后者是产物且带签名 pin）。

## 技术栈

Rust edition 2024（rusqlite bundled、zstd、serde_json、tracing）；Kotlin + Compose + kotlinx.serialization + MeowUI；桥是纯 Java 对着 `android.jar` 编译再 `d8` 成 dex。

**版本与坐标唯一真源**：Rust 侧 [Cargo.toml](Cargo.toml) 的 workspace 表，Kotlin 侧 `apps/manager/gradle/libs.versions.toml`，**发布版本号在 [version.properties](version.properties)**——Gradle 读它当 versionName（versionCode 由 `major*10000+minor*100+patch` 推导），`cch-packager` 读它写 `module.prop`。**文档不复述版本号。**

## 代码地图

```
crates/
├── cch_daemond/   服务本体：core（请求分发）server transport collectors packages bridge_broker
├── cch_logd/      logd 二进制协议：event（am_crash/am_anr）+ crash（FATAL EXCEPTION）
├── cch_dropbox/   DropBox 条目；cch_tombstone/ protobuf tombstone；cch_anrfile/ ANR 转储
├── cch_watcher/   inotify；cch_merge/ 多源同一崩溃合一；cch_model/ 领域模型 + RecordId
├── cch_store/     SQLite 索引 + zstd 正文文件（read/write/retention/payload/schema/sql）
├── cch_wire/      JSON 协议（rpc/dto/event/frame/bridge）+ tests/vectors.rs
├── cch_auth/      按签名证书 pin 鉴权；cch_apk_sig/ APK v2/v3 签名块解析
├── cch_config/    配置文档与 patch；cch_settings/ Android settings（接管系统弹窗）
└── cch_packager/  发布工具：三 ABI 守护进程 + 桥 dex + 模块 zip（子命令 module/manager-apk/bridge）
bridge/src/.../CrashCatcherBridge.java   通知、PackageManager 富化、跨用户启动 Activity
module/            service.sh（选 ABI、退避重启、把状态写进 module.prop 描述）+ customize.sh + service.d/
apps/manager/app/src/main/kotlin/.../crashcatcher/
├── data/daemon/   DaemonClient + WireRpc/WireModel/WireJson + DaemonRepositories
├── data/device/   readDeviceInfo（Build.* 快照）
├── domain/        model（LoadState/DomainError/DeviceInfo）+ repository（接口）
└── ui/            shell（CrashCatcherApp/Chrome/Destination/AppContainer）home crashes apps
                   settings detail components util theme
```

屏幕与 ViewModel 一一对应、`components/` 内各件职责都能从文件名读出，此处不复述。

## 架构

```
logd / DropBox / tombstone / ANR 文件
  → 各 collector → cch_merge 合一 → cch_store（SQLite 索引 + zstd 正文）
       ↑ root 守护进程 catcherd（module/service.sh 拉起，退避重启）
       ├→ 抽象 unix socket @crash_catcher_daemon_manager ──→ 管理器（按签名 pin 鉴权）
       └→ 抽象 unix socket @crash_catcher_daemon_bridge ──→ 特权 Java 桥（app_process）
                                                              通知 / 包名标签 / 启动 Activity
```

四字节大端长度前缀 + JSON 一帧；正文不进帧，走 memfd 经 `SCM_RIGHTS` 传描述符，太大或失败才退回分块读。

## 关键约束

### 采集与存储

**`RecordId` 必须从库里已有的最大 id 续号**（`Store::open` → `resume_ids()` → `RecordIdGenerator::resume_after`）。id 的高 48 位是**崩溃自己的**时间戳而非当前时间，所以重启后从零开始的生成器会对同一条 tombstone 再发一次同样的 id，插入撞主键，**崩溃被静默丢掉**，唯一痕迹是日志里一行 `UNIQUE constraint failed: crash_record.id`。

**`packages.xml` 从 Android 12 起是 ABX（二进制 XML）**，`read_to_string` 直接 "stream did not contain valid UTF-8"，守护进程会起不来并被 service.sh 反复重启。包信息一律走 `cmd package list packages -f`。

**包索引要能在鉴权失败时重载一次**（`authenticate_uid`，`PACKAGE_RELOAD_INTERVAL_MS` 限频）。索引在启动时建好，而管理器重装/更新会把 APK 挪到 `/data/app` 下新的随机目录，长跑的守护进程会一直拒绝真正的管理器、界面上表现为「未连接」——每次更新管理器都会遇到，不能指望重启。限频是防任何 app 靠反复连接让守护进程枚举全部包。

**保留策略只删正文与明细行，不动 `occurrence`**：聚合值在写入时维护、读取时不算。分组页因此要说明「另有 N 次已超出保留上限」，否则两个数字看起来自相矛盾。

### 协议与鉴权

**kotlinx 的 `@SerialName` 不能在 Request 与 Response 之间重名**。描述符相等性只看**名字、元数数量和元素类型**，不看属性名：两个都叫 `handshake` 的 `(Int, String)` 类会判等并共用一份字段名缓存，Response 于是去找 Request 的 `client_version`，报错指向一个根本不存在的字段。新增变体先扫一遍两个 enum 的 serial name。

**`cch_wire/tests/vectors.rs` 是两侧不漂移的唯一保证**：Rust 与 Kotlin 各自实现同一份协议，改 DTO 必须同时改 `WireRpc.kt`/`WireModel.kt` 并让 vectors 通过。

**memfd 必须以只读重开再交出去**（`payload.rs::reopen_read_only`，经 `/proc/self/fd/{fd}`）。带写权限的 fd 传给 `untrusted_app` 会被 SELinux 拒（`avc: denied { write } … tmpfs`），而如果同时把响应里的 `handle` 写成 `None`，分块回退路径也够不到，详情页就是一片空白且没有任何报错。

### 特权边界

**桥不能启动 Activity**。它是裸 `app_process`：有 `Context`（够用 `NotificationManager`），但没有注册的 application record，ActivityManager 会以 `Unable to find app for caller … (pid=-1)` 拒掉。要弹的东西由守护进程用 `am` 启动——它是 root，`am` 乐意为它拉起，还能 `--user` 跨用户。

**这台 ROM 的 `am -n` 只认相对组件名**（`包名/.类名`）。写全限定类名会得到 "Activity class … does not exist"，尽管 PackageManager 明明列着它。见 `MANAGER_DETAIL_COMPONENT`。

**`am start -a MAIN -c LAUNCHER -p <包名>` 起不了应用**：`-p` 是加在隐式 intent 上的**过滤器**，不是目标，这台 ROM 回 "unable to resolve Intent"。`start_launcher_activity` 因此先 `cmd package resolve-activity --brief` 拿组件再 `-n` 启动——这正是「重新打开按下去没反应」的根因。

**通知归发它的进程所有**。桥用系统 context 发的通知，管理器 `cancel` 不掉；`setAutoCancel` 也只管点正文。所以按钮动作要经 `Request::DismissNotification` → 守护进程 → `BridgeCommand::CancelNotification` 让桥去撤——否则动作执行了、通知还挂在那，看起来就像按钮没反应。

**`pkill -f catcherd` 会连自己的 `su -c` shell 一起杀**（模式匹配到了那条命令行本身），退出码 143。要用 `pkill -x`。

### 管理器 UI

**不要跨风格读 token**。MeowUI 在 Material Expressive 与 Miuix 之间切换，`MeowTheme.colors`/`MeowIcons` 已经按风格解析好；直接读 `MaterialTheme.colorScheme` 在 Miuix 皮肤下拿到的是 Material 的容器色（比周围扁平灰的容器明显更艳），图标同理会出现「Material 垃圾桶配 miuix 圆角卡片」。`MiuixMaterialBridge` 只是给 `Text`/`Icon` 这类无 MeowUI 对应件兜 `LocalContentColor` 的，**不是取色来源**。

**手写的滚动容器必须挂 `Modifier.meowScaffoldScroll()`**，否则大标题永不折叠、内容滚上去直接撞在标题上。库自己的容器（`MeowPreferenceScreen`、`MeowPullToRefresh`）已经接好了，所以只有手搓 `LazyColumn` 的页面会中招。**例外是崩溃详情页**：那里大标题**不折叠**（它写着正在读的异常名，是一屏栈帧唯一的上下文），所以顶部 inset 当**布局** padding 用，让正文在标题下方滚动而不是穿过去。

**一个 `ScrollState` 不能给多个 `horizontalScroll` 共用**：每个节点都按**自己**内容的宽度写 `maxValue`，最后测量的那行（往往是短行，报 0）会把整体夹死——这就是「不换行模式左右滑不动」。正确做法是整个列表套一个滚动容器，宽度按最宽行实测（等宽字体先按列数排序取前几名，再用 `TextMeasurer` 精确量，兼顾 CJK 双宽）。

**关闭页面的信号必须一次性**。删除一条记录会同时满足两个关闭条件（删除自身成功、以及 `dataChanged` 后重读发现记录已没了——而删除成功正是 `dataChanged` 的来源），两次 `Deleted` 就 pop 掉两页，把用户所在的分组页也带走；第三次到达甚至能把 `Page.Shell` 弹掉，`MeowNavHost` 于是抛 `IllegalArgumentException: backStack must not be empty`（这条崩溃本项目记录过自己的）。两侧都用 `closed` 标志守一次，外加 `pop` 本身拒绝弹根页。

### 构建与发布

**模块 zip 与管理器 APK 是成对的**：zip 里 `config/manager_signing_cert.sha256` pin 住管理器的签名证书，守护进程据此在 socket 上鉴权。所以 `cch-packager module` 要 `--manager-apk` 从那个 APK 推 pin，**不要手写 pin**；也因此 debug 构建同样用 release key 签（`keystore.properties` 在就两个 variant 都用），否则装上去连不通。

**miuix 在 app 侧要再 `includeBuild` 一次**。嵌套 included build 的坐标替换不传到外层：MeowUI 自己那份能解析，但当 MeowUI 也是 included build 时，`top.yukonga.miuix.kmp:*` 在外层解析——0.9.4-rc01 起它们在 Maven Central 上有真产物，于是**解析成功但类不对**，报一屏 `Unresolved reference 'preference'` 且指向 MeowUI 自己的源码。

**`gradlew` 的可执行位要显式加**（`git update-index --chmod=+x`）：Windows 上提交丢这一位，CI 报 `./gradlew: Permission denied`（exit 126）。

**release 走 R8**，规则在 `apps/manager/app/proguard-rules.pro`。里面只有两条真需要的：kotlinx serializer 的 keep（full mode 会剪掉，失败表现是第一次请求抛 `SerializationException` 而不是构建报错），以及 `CrashDetailActivity`（守护进程按名字启动它，APK 内部没有任何引用）。

**CI 与发布**：[ci.yml](.github/workflows/ci.yml) 跑两侧检查；[release.yml](.github/workflows/release.yml) 由 `v*` tag 触发，先校验 tag 与 `version.properties` 一致，再 APK → 模块 zip → GitHub release → Telegram 频道（两个文件一条消息发出去，避免有人只下到半套）。签名密钥经 base64 存在仓库 secret 里，落到 checkout 之外。
