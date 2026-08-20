# 0.3.0

- 管理器改为监听、模块主动连接，解决部分 ROM 在 SELinux enforcing 下拒绝应用连接 root socket 的问题；两侧仍互相校验身份

- 未连接时也会保留管理器端的监听、peer 和握手诊断，并随诊断报告一并导出

- daemon 重启留下旧连接时首次请求会自动恢复，工作资料与多用户安装也能正确鉴权

- logd 断流后自动重连，Tombstone 等待写入完成后再解析，避免采集状态长期卡在读取受阻

- 模块与管理器协议升至 5，必须同时更新

- build: release 0.3.0
- manager: clarify paired-version recovery
- manager: include connection diagnostics in reports
- manager: accept authenticated daemon connections
- daemon: authenticate managers from secondary users
- daemon: wait for tombstones to finish writing
- logd: reconnect collectors after log streams close
- build: preserve the MSVC path in the cargo wrapper
- manager: give the runtime log its own page, with a file switcher
- module: read one named log file out of the set, and bump the protocol
- manager: notice a dropped connection instead of waiting to be asked
- daemon: own the log file so it can be rotated by size
- logd: carry the raw bytes of an entry that failed to parse
- bridge: report the release version, not a hardcoded one
- manager: a diagnostics page that works when nothing else does
- daemon: answer why it is not working, not just that it is not
- chore: warn that 0.2.0 needs both halves replaced
- manager: show the protocol version without needing a connection
- daemon: report the release version, not the crate's own
- daemon: refuse to serve a manager that has not agreed on a protocol
- ci: keep tags out of the channel
- chore: update.json and changelog for v0.2.0

# 0.2.0

- 「包含系统应用」现在真的生效:系统应用改为直接问 PackageManager 判定,不再按 APK 所在分区猜——漏掉 /system_ext 正是它此前看起来没反应的原因
- HAL、system_server 这类不是应用的系统进程单独识别、单独筛选,不再按应用的方式显示图标与名称
- 「静音至下次解锁」会在解锁或熄屏后真的恢复,此前只有守护进程重启才会清掉
- 原生崩溃不再一条记成两条,也不会再被标成 Java
- 带参数启动的进程不再被记到它的参数(一个 .so)名下
- 「从未产生日志」与「日志已回收」现在分开显示

⚠ 模块与管理器必须一起更新:本版协议从 1 升到 2,握手要求两侧协议号完全相同,只换一半会连不上并提示「版本不匹配」。

回退旧版本前请先删除 /data/adb/crash.catcher/store/crashes.db:存储结构只升不降,旧版守护进程读到会拒绝启动。

- ci: say what changed, and stop curl reading the caption as a filename
- build: release 0.2.0
- docs: record what the boot order and a schema bump cost
- manager: present platform processes as what they are
- daemon: tell apps, system apps and platform processes apart
- daemon: file a tombstone under its process, not its last argument
- manager: read the status card's text colour after it resolves
- build: resolve miuix from Central instead of nesting it twice
- ci: make the protocol version the signal that ships both halves
- manager: give the healthy status card a visible background
- ci: parallelise the build, and cut the ABIs day-to-day
- ci: build only what changed, and post every build to the channel
- build: make versionCode the commit count
- docs: license under AGPL-3.0

