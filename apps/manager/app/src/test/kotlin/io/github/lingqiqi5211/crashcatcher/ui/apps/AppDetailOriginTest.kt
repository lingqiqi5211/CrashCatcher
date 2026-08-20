package io.github.lingqiqi5211.crashcatcher.ui.apps

import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashKind
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Whether the per-app page is about an app or a platform process.
 *
 * The route carries identity but no classification, so this is answered twice: from the name's
 * shape while the first row is still loading, and from the daemon's own verdict once one has arrived. Getting the
 * first answer wrong means the page opens offering to launch a HAL and rearranges itself a moment
 * later.
 */
class AppDetailOriginTest {

    private fun group(packageName: String, packageInstalled: Boolean) = GroupSummary(
        groupId = "g1",
        packageName = packageName,
        processName = packageName,
        userId = 0,
        kind = CrashKind.NativeCrash,
        isSystemApp = !packageInstalled,
        isMainProcess = true,
        selfHandled = false,
        occurrence = 27,
        firstSeenMs = 10,
        lastSeenMs = 20,
        payloadBytes = 0,
        packageInstalled = packageInstalled,
    )

    @Test
    fun `the daemon's verdict decides once a row has loaded`() {
        val process = AppDetailUiState(
            packageName = "/vendor/bin/hw/some.hal",
            groups = listOf(group("/vendor/bin/hw/some.hal", packageInstalled = false)),
        )
        assertTrue(process.isPlatformProcess)

        val app = AppDetailUiState(
            packageName = "com.example.app",
            groups = listOf(group("com.example.app", packageInstalled = true)),
        )
        assertFalse(app.isPlatformProcess)
    }

    /**
     * A name that could not be a package is treated as a process straight away, so the heading and
     * the launch row are right on the first frame rather than after the query returns.
     */
    @Test
    fun `before any row loads the name's shape decides`() {
        assertTrue(AppDetailUiState(packageName = "/vendor/bin/hw/some.hal").isPlatformProcess)
        assertTrue(AppDetailUiState(packageName = "system_server").isPlatformProcess)
        assertFalse(AppDetailUiState(packageName = "com.example.app").isPlatformProcess)
    }

    /**
     * The verdict wins over the shape: `com.android.server.telecom` looks like a package and on
     * some builds is not an installed one.
     */
    @Test
    fun `a process whose name looks like a package still follows the verdict`() {
        val state = AppDetailUiState(
            packageName = "com.android.server.telecom",
            groups = listOf(group("com.android.server.telecom", packageInstalled = false)),
        )
        assertTrue(state.isPlatformProcess)
    }
}
