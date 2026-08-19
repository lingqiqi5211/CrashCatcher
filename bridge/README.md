# ct-bridge

由 `app_process` 启动的常驻特权 Java 桥。它只负责 Android Java API 才能可靠完成的三件事：
通知、PackageManager 富化和跨用户启动 Activity；业务状态仍只保存在 daemon。

入口类：`io.github.lingqiqi5211.crashcatcher.bridge.CrashCatcherBridge`。
`cch_packager` 使用目标 Android SDK 的 `android.jar` 编译后交给 `d8` 生成
`dex/cch_bridge.dex`。
