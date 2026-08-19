package io.github.lingqiqi5211.crashcatcher.data.device

import android.os.Build
import io.github.lingqiqi5211.crashcatcher.BuildConfig
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo

/**
 * Reads the device and build facts the overview screen shows.
 *
 * A plain function rather than a repository: there is no I/O, no caching decision
 * and nothing that can fail, so wrapping it in a `LoadState` would add a loading
 * branch the UI would never take.
 */
internal fun readDeviceInfo(): DeviceInfo = DeviceInfo(
    managerVersionName = BuildConfig.VERSION_NAME,
    managerVersionCode = BuildConfig.VERSION_CODE.toLong(),
    androidRelease = Build.VERSION.RELEASE.orEmpty(),
    androidApiLevel = Build.VERSION.SDK_INT,
    manufacturer = Build.MANUFACTURER.orEmpty(),
    brand = Build.BRAND.orEmpty(),
    model = Build.MODEL.orEmpty(),
    buildDisplayId = Build.DISPLAY.orEmpty(),
    fingerprint = Build.FINGERPRINT.orEmpty(),
    supportedAbis = Build.SUPPORTED_ABIS?.toList().orEmpty(),
)
