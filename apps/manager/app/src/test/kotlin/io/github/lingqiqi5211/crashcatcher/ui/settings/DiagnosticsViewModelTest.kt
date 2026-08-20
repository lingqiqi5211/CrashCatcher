package io.github.lingqiqi5211.crashcatcher.ui.settings

import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.RuntimeLogFile
import io.github.lingqiqi5211.crashcatcher.domain.model.LoadState
import io.github.lingqiqi5211.crashcatcher.domain.repository.ConfigRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.DialogTakeoverOutcome
import io.github.lingqiqi5211.crashcatcher.domain.repository.GlobalConfigUpdate
import io.github.lingqiqi5211.crashcatcher.domain.repository.RuntimeLogSnapshot
import java.io.IOException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticsViewModelTest {

    @Test
    fun `an export includes a connection failure discovered while reading daemon logs`() = runTest {
        val managerTrace = StringBuilder("manager socket listener ready\n")
        val config = FailingRuntimeLogRepository {
            managerTrace.append("request retry failed request=ReadRuntimeLog\n")
        }
        val viewModel = DiagnosticsViewModel(
            config = config,
            managerLogs = { mapOf("manager.log" to managerTrace.toString()) },
        )

        val logs = viewModel.readAll()

        assertTrue(logs.getValue("manager.log").contains("request retry failed"))
        assertFalse(logs.containsKey("daemon.log"))
    }
}

private class FailingRuntimeLogRepository(
    private val onNamedReadFailure: () -> Unit,
) : ConfigRepository {
    override val globalConfig = MutableStateFlow<LoadState<GlobalConfig>>(LoadState.Loading)

    override suspend fun refreshGlobalConfig() = Unit

    override suspend fun updateGlobalConfig(patch: GlobalConfigPatch): Result<GlobalConfigUpdate> =
        error("not used")

    override suspend fun appConfig(packageName: String): Result<AppConfig> = error("not used")

    override suspend fun updateAppConfig(
        packageName: String,
        patch: AppConfigPatch,
    ): Result<AppConfig> = error("not used")

    override suspend fun setDialogTakeover(enabled: Boolean): Result<DialogTakeoverOutcome> =
        error("not used")

    override suspend fun mute(packageName: String, scope: MuteScope): Result<Unit> =
        error("not used")

    override suspend fun runtimeLog(name: String?, maxBytes: Long): Result<RuntimeLogSnapshot> {
        if (name == null) {
            return Result.success(
                RuntimeLogSnapshot(
                    name = "daemon.log",
                    text = "",
                    truncated = false,
                    totalBytes = 1,
                    files = listOf(RuntimeLogFile("daemon.log", bytes = 1, modifiedMs = 0)),
                ),
            )
        }
        onNamedReadFailure()
        return Result.failure(IOException("connection closed"))
    }
}
