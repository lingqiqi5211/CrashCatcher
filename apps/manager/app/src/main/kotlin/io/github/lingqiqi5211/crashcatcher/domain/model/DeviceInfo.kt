package io.github.lingqiqi5211.crashcatcher.domain.model

/**
 * Static facts about the device and this build.
 *
 * Read once at construction: none of it can change while the process lives, so
 * there is nothing to refresh and no reason for it to travel through the daemon.
 * It is also what makes the overview screen useful before the daemon is reachable —
 * the screen doubles as the about page.
 */
data class DeviceInfo(
    val managerVersionName: String,
    val managerVersionCode: Long,
    val androidRelease: String,
    val androidApiLevel: Int,
    val manufacturer: String,
    /**
     * Kept alongside [manufacturer], which is usually but not always the same string —
     * a rebranded device reports one vendor as its maker and another as its brand, and an
     * exported report is read by someone trying to identify the exact device.
     */
    val brand: String,
    val model: String,
    /** `Build.DISPLAY` — the ROM build a reader needs to tell two firmwares apart. */
    val buildDisplayId: String,
    val fingerprint: String,
    val supportedAbis: List<String>,
)
