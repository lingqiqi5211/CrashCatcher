# CrashCatcher CrashDemo

这是一个独立的 Kotlin + Jetpack Compose 测试应用，用于触发 Java 崩溃、自处理异常、ANR、
native signal 和 WTF 记录。只应安装在测试设备上。

可复用管理器项目的 Gradle Wrapper 构建：

```text
../manager/gradlew -p . :app:assembleDebug
```
