package io.github.lingqiqi5211.crashcatcher.ui.shell

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import io.github.lingqiqi5211.crashcatcher.BuildConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonAppInventoryRepository
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonClient
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonConfigRepository
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonCrashRepository
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonModuleStatusRepository
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonStatsRepository
import io.github.lingqiqi5211.crashcatcher.data.daemon.LogcatDaemonTrace
import io.github.lingqiqi5211.crashcatcher.data.daemon.LocalSocketTransport
import io.github.lingqiqi5211.crashcatcher.data.daemon.ManagerTraceStore
import io.github.lingqiqi5211.crashcatcher.data.preferences.AppearancePreferencesRepository
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppDetailViewModel
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppsViewModel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashesViewModel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.GroupDetailViewModel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailViewModel
import io.github.lingqiqi5211.crashcatcher.ui.home.HomeViewModel
import io.github.lingqiqi5211.crashcatcher.ui.settings.DiagnosticsViewModel
import io.github.lingqiqi5211.crashcatcher.ui.settings.SettingsViewModel
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.withContext

/**
 * Hand-built dependency container.
 *
 * No DI framework: the graph is a dozen objects with no cycles and no scoping
 * beyond "process" and "screen", which a framework would only obscure. Everything
 * is constructed here, so the whole graph is visible in one screen of code.
 */
internal class AppContainer(context: Context) {

    /** Outlives any screen; owns the flows repositories keep hot. */
    private val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    // Kept in every build: when the daemon never connects, its logs cannot be requested, so this
    // local listener/peer/handshake trace is the only evidence a user can put in a report.
    private val managerTrace = ManagerTraceStore(
        File(context.applicationContext.filesDir, "diagnostics"),
    )

    private val daemonTrace = if (BuildConfig.DEBUG) {
        LogcatDaemonTrace(managerTrace)
    } else {
        managerTrace
    }

    private val client = DaemonClient(
        transport = LocalSocketTransport(trace = daemonTrace),
        clientVersion = BuildConfig.VERSION_NAME,
        trace = daemonTrace,
    )

    val appearance = AppearancePreferencesRepository(context.applicationContext, applicationScope)

    val moduleStatus = DaemonModuleStatusRepository(client, applicationScope)
    val crashes = DaemonCrashRepository(client)
    val config = DaemonConfigRepository(client)
    val apps = DaemonAppInventoryRepository(client)
    val stats = DaemonStatsRepository(client)

    suspend fun readManagerLogs(): Map<String, String> = withContext(Dispatchers.IO) {
        managerTrace.readAll()
    }
}

/**
 * Maps a ViewModel class to its constructor.
 *
 * A `when` rather than reflection: an unmapped ViewModel then fails to compile in
 * the branch that added it, instead of at the moment a user opens that screen.
 */
internal class AppViewModelFactory(
    private val container: AppContainer,
) : ViewModelProvider.Factory {

    @Suppress("UNCHECKED_CAST")
    override fun <T : ViewModel> create(modelClass: Class<T>): T = when (modelClass) {
        HomeViewModel::class.java -> HomeViewModel(
            moduleStatus = container.moduleStatus,
            stats = container.stats,
            crashes = container.crashes,
        )

        CrashesViewModel::class.java -> CrashesViewModel(
            crashes = container.crashes,
        )

        GroupDetailViewModel::class.java -> GroupDetailViewModel(
            crashes = container.crashes,
        )

        RecordDetailViewModel::class.java -> RecordDetailViewModel(
            crashes = container.crashes,
        )

        AppsViewModel::class.java -> AppsViewModel(
            apps = container.apps,
            crashes = container.crashes,
        )

        AppDetailViewModel::class.java -> AppDetailViewModel(
            config = container.config,
            crashes = container.crashes,
            apps = container.apps,
        )

        SettingsViewModel::class.java -> SettingsViewModel(
            config = container.config,
            moduleStatus = container.moduleStatus,
            crashes = container.crashes,
        )

        DiagnosticsViewModel::class.java -> DiagnosticsViewModel(
            config = container.config,
            managerLogs = container::readManagerLogs,
        )

        else -> error("no factory for ${modelClass.name}")
    } as T
}
