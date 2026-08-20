package io.github.lingqiqi5211.crashcatcher.ui.settings

import io.github.lingqiqi5211.crashcatcher.data.daemon.BridgeFacts
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorHealth
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorSource
import io.github.lingqiqi5211.crashcatcher.data.daemon.DialogTakeoverStatus
import io.github.lingqiqi5211.crashcatcher.data.daemon.ModuleStatus
import io.github.lingqiqi5211.crashcatcher.data.daemon.PackageIndexFacts
import io.github.lingqiqi5211.crashcatcher.data.daemon.RuntimeFacts
import io.github.lingqiqi5211.crashcatcher.data.daemon.StorageStatus
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The report is the thing that gets pasted into an issue, so what it must not do is omit the
 * half that is broken — an unreachable daemon is the most common reason to open the page.
 */
class DiagnosticsReportTest {

    private val device = DeviceInfo(
        managerVersionName = "0.2.0",
        managerVersionCode = 29,
        androidRelease = "16",
        androidApiLevel = 37,
        manufacturer = "Xiaomi",
        brand = "Redmi",
        model = "23049RAD8C",
        buildDisplayId = "OS4.0.0.9.XPCCNXM",
        fingerprint = "Redmi/marble/marble:17/AQ3A.250226.002/OS4.0.0.9.XPCCNXM:user/release-keys",
        supportedAbis = listOf("arm64-v8a", "armeabi-v7a"),
    )

    private fun status() = ModuleStatus(
        daemonVersion = "0.2.0",
        protocolVersion = 3,
        uptimeMs = 1_234,
        collectors = listOf(
            CollectorHealth(
                source = CollectorSource.Tombstone,
                enabled = true,
                everReceived = false,
                detail = "dropbox:data_app_crash is disabled",
            ),
        ),
        bridgeConnected = true,
        dialogTakeover = DialogTakeoverStatus(
            requested = false,
            effective = false,
            anrShowBackgroundConflict = false,
        ),
        storage = StorageStatus(groupCount = 35, recordCount = 209),
        runtime = RuntimeFacts(
            pid = 2726,
            abi = "aarch64",
            androidSdk = 37,
            selinux = "enforcing",
            storeSchemaVersion = 2,
            debugLogging = false,
            packageIndex = PackageIndexFacts(packageCount = 656, systemFlagsKnown = true),
            bridge = BridgeFacts(connected = true, version = "1", androidSdk = 37),
            activeMutes = 1,
        ),
    )

    @Test
    fun `a full report carries every link of the chain`() {
        val report = buildDiagnosticsReport(status(), device, connected = true)

        // The daemon side.
        assertTrue(report.contains("pid: 2726"))
        assertTrue(report.contains("abi: aarch64"))
        assertTrue(report.contains("selinux: enforcing"))
        assertTrue(report.contains("store_schema: 2"))
        assertTrue(report.contains("active_mutes: 1"))
        // The bridge and the index, which explain notifications and app classification.
        assertTrue(report.contains("system_flags_known: true"))
        assertTrue(report.contains("packages: 656"))
        // A collector's own error text, which is usually the actual answer.
        assertTrue(report.contains("tombstone: enabled=true received=false"))
        assertTrue(report.contains("dropbox:data_app_crash is disabled"))
        // And the device, so a report identifies the ROM it came from.
        assertTrue(report.contains("build: OS4.0.0.9.XPCCNXM"))
        assertTrue(report.contains("fingerprint: Redmi/marble"))
        // Next to the daemon's own abi, this is what makes a wrong-architecture install visible.
        assertTrue(report.contains("supported_abis: arm64-v8a, armeabi-v7a"))
    }

    /**
     * The manager's own protocol number has to survive an unreachable daemon: it is half of a
     * version mismatch, and the other half is in the refusal the user just saw.
     */
    @Test
    fun `an unreachable daemon still produces a usable report`() {
        val report = buildDiagnosticsReport(status = null, device = device, connected = false)

        assertTrue(report.contains("unreachable"))
        assertTrue(report.contains("protocol: 3"))
        assertTrue(report.contains("version: 0.2.0 (29)"))
        assertTrue(report.contains("connected: false"))
        // Device facts are local, so they are known either way.
        assertTrue(report.contains("model: 23049RAD8C"))
        assertTrue(report.contains("android: 16 (API 37)"))
    }
}
