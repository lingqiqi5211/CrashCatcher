package io.github.lingqiqi5211.crashcatcher

import android.app.Application
import io.github.lingqiqi5211.crashcatcher.ui.shell.AppContainer

/** Owns the process-wide daemon connection and repositories. */
class CrashCatcherApplication : Application() {
    internal val container by lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
        AppContainer(this)
    }
}
