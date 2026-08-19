package io.github.lingqiqi5211.crashcatcher.ui.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorHealth
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorSource
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.domain.model.valueOrNull
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTag
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTagTone
import io.github.lingqiqi5211.crashcatcher.ui.components.TonalCard
import io.github.lingqiqi5211.crashcatcher.ui.components.crashCatcherContentScaffoldPadding
import io.github.lingqiqi5211.meowui.component.MeowPreferenceScreen
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * The overview screen.
 *
 * Three cards: a state-toned status hero, one card per collector, and the device
 * facts. [MeowPreferenceScreen] owns the scroll container, page padding and section
 * spacing — and the top-bar collapse behaviour — so there is one dashboard rather
 * than a Material and a Miuix copy of the same card list.
 *
 * The collector card is the reason this screen leads the app. A module that reports
 * itself "active" while quietly recording nothing is the failure this whole tool
 * exists to avoid, so the screen says which sources have actually produced data
 * rather than showing a single green badge.
 *
 * It doubles as the about page: the device card is useful before the daemon is
 * reachable, which is exactly when a user goes looking for it.
 */
@Composable
internal fun HomeScreen(
    state: HomeUiState,
    onRefresh: () -> Unit,
    onReconnect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    LaunchedEffect(Unit) { onRefresh() }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { onRefresh() }

    MeowPreferenceScreen(
        modifier = modifier.testTag("crashcatcher.home.scroll"),
        scaffoldPadding = crashCatcherContentScaffoldPadding,
    ) {
        StatusCard(state = state, onReconnect = onReconnect)
        StatsCard(state)
        CollectorCard(state.collectors)
        StorageCard(state)
        DeviceCard(state.deviceInfo)
    }
}

/**
 * The facts about the device this is running on.
 *
 * Lives here rather than only on the about page: these are what a crash has to be read
 * against — an ABI, an API level, a ROM build — so they belong with the module's current
 * state. The about page keeps the *project's* identity instead: versions, source, and
 * what it is built on.
 */
@Composable
private fun DeviceCard(deviceInfo: DeviceInfo) {
    HomeSectionTitle(stringResource(R.string.home_section_device))
    HomeInfoCard(modifier = Modifier.testTag("crashcatcher.home.device")) {
        HomeInfoEntry(
            label = stringResource(R.string.home_android_version),
            value = "${deviceInfo.androidRelease} (API ${deviceInfo.androidApiLevel})",
        )
        HomeInfoEntry(
            label = stringResource(R.string.home_device_model),
            value = "${deviceInfo.manufacturer} ${deviceInfo.model}",
        )
        HomeInfoEntry(
            label = stringResource(R.string.home_abi),
            value = deviceInfo.supportedAbis.joinToString(", ").ifEmpty { "—" },
        )
        HomeInfoEntry(
            label = stringResource(R.string.home_fingerprint),
            value = deviceInfo.fingerprint,
        )
    }
}

/**
 * The state of the module, as the first and largest thing on the page.
 *
 * Every state gets a real container, not just the failing one. Drawing the healthy
 * case on `surfaceVariant` left it indistinguishable from the page behind it, so the
 * page opened with an icon and two lines of text floating in space while every other
 * surface in the app is a card. The tone carries the meaning — error, warning, or a
 * calm tint when there is nothing wrong — and the badge repeats it in a shape, so the
 * state survives a glance and does not depend on colour alone.
 */
@Composable
private fun StatusCard(state: HomeUiState, onReconnect: () -> Unit) {
    val visuals = state.runtimeStatus.visuals()

    // MeowUI's roles, not Material's: this card is the largest block of tinted colour on
    // the page, so painting it from Material's containers under a Miuix skin was the most
    // visible place the two palettes disagreed.
    //
    // Only a problem gets a tinted container. Healthy used to take `secondaryContainer`,
    // which in Miuix is a mid grey meant for a switch track — as a full-width card it came
    // out lighter than every other card on the page, and its `onSecondaryContainer` text is
    // a dim grey designed to sit on that swatch, not to be read as a heading. Nothing being
    // wrong is the ordinary state, so it now looks like an ordinary card and the accent
    // lives in the badge alone.
    val colors = MeowTheme.colors
    val container = when (state.runtimeStatus) {
        RuntimeStatus.Unreachable -> colors.errorContainer
        // Degraded is a warning, not information: something is collecting less than it
        // should, which is not the same as the daemon being gone.
        RuntimeStatus.Degraded -> colors.warningContainer
        else -> colors.surfaceVariant
    }
    val content = when (state.runtimeStatus) {
        RuntimeStatus.Unreachable -> colors.onErrorContainer
        RuntimeStatus.Degraded -> colors.onWarningContainer
        else -> colors.onSurface
    }
    // The badge carries the state's colour when the card itself does not.
    val accent = when (state.runtimeStatus) {
        RuntimeStatus.Unreachable, RuntimeStatus.Degraded -> content
        else -> colors.primary
    }

    TonalCard(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("crashcatcher.home.status"),
        color = container,
        contentColor = content,
        contentPadding = PaddingValues(horizontal = 18.dp, vertical = 18.dp),
        // Tapping only does something when there is something to retry.
        onClick = onReconnect.takeIf { state.runtimeStatus == RuntimeStatus.Unreachable },
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .background(accent.copy(alpha = BADGE_ALPHA), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = visuals.icon,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp),
                    tint = accent,
                )
            }
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(
                    text = stringResource(visuals.headlineRes),
                    style = MeowTheme.typography.sectionTitle,
                    fontWeight = FontWeight.SemiBold,
                    color = content,
                )
                Text(
                    text = state.supportingText(),
                    style = MeowTheme.typography.summary,
                    color = content.copy(alpha = SUPPORTING_ALPHA),
                )
                val bridgeConnected = state.moduleStatus.valueOrNull?.bridgeConnected
                if (bridgeConnected == false) {
                    // Records still land; only the notification is delayed. Saying so
                    // stops it reading as "nothing works".
                    Text(
                        text = stringResource(R.string.status_bridge_missing),
                        style = MeowTheme.typography.summary,
                        color = content.copy(alpha = SUPPORTING_ALPHA),
                    )
                }
            }
        }
    }
}

/**
 * The three numbers an overview exists to answer, side by side.
 *
 * Previously the only count on the page was a clause inside the status card's
 * supporting sentence, and the storage totals were four label/value rows further down
 * — so "how much has this thing actually recorded" took reading, not looking.
 *
 * Hidden entirely until the daemon answers: three dashes tell the reader nothing that
 * the status card above has not already said more clearly.
 */
@Composable
private fun StatsCard(state: HomeUiState) {
    val storage = state.moduleStatus.valueOrNull?.storage ?: return

    TonalCard(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("crashcatcher.home.stats"),
        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 16.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceEvenly,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            StatColumn(
                value = storage.groupCount.toString(),
                label = stringResource(R.string.storage_groups),
            )
            StatColumn(
                value = storage.recordCount.toString(),
                label = stringResource(R.string.storage_records),
            )
            StatColumn(
                // The two halves added together, because "how much room is this taking"
                // is one question. The split lives in the storage section below, which
                // exists to answer the follow-up.
                value = formatBytes(storage.payloadBytes + storage.databaseBytes),
                label = stringResource(R.string.storage_total_bytes),
            )
        }
    }
}

@Composable
private fun StatColumn(value: String, label: String) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(
            text = value,
            // The page-title role rather than a Material headline: these three numbers are
            // the largest thing on the overview, and the role is the only one either design
            // system defines for that.
            style = MeowTheme.typography.pageTitle,
            fontWeight = FontWeight.SemiBold,
            color = MeowTheme.colors.onSurface,
            maxLines = 1,
        )
        Text(
            text = label,
            style = MeowTheme.typography.summary,
            color = MeowTheme.colors.onSurfaceVariant,
            maxLines = 1,
        )
    }
}

@Composable
private fun HomeSectionTitle(text: String) {
    Text(
        text = text,
        style = MeowTheme.typography.sectionTitle,
        color = MeowTheme.colors.onSurfaceVariant,
        modifier = Modifier.padding(start = 4.dp, top = 8.dp, bottom = 4.dp),
    )
}

/**
 * Per-source collection health, as compact rows inside one card.
 *
 * One card, not five. Each source needs a name and a state and nothing else, and five
 * separate cards with a gap between them spent most of a phone screen restating the
 * same shape — the block is one answer ("is anything silent?") and now reads as one.
 *
 * This is still the reason the overview leads the app: a module that reports itself
 * active while quietly recording nothing is the exact failure this tool exists to
 * avoid, so the page names the sources that have never produced data instead of showing
 * one green badge for the lot.
 */
@Composable
private fun CollectorCard(collectors: List<CollectorHealth>) {
    if (collectors.isEmpty()) return

    HomeSectionTitle(stringResource(R.string.home_section_collectors))
    TonalCard(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("crashcatcher.home.collectors"),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 6.dp),
    ) {
        collectors.forEach { collector ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 10.dp)
                    .testTag("crashcatcher.home.collector.${collector.source.name}"),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(1.dp),
                ) {
                    Text(
                        text = stringResource(collector.source.labelRes),
                        style = MeowTheme.typography.title,
                        color = MeowTheme.colors.onSurface,
                        maxLines = 1,
                    )
                    collector.detail?.let { detail ->
                        Text(
                            text = detail,
                            style = MeowTheme.typography.summary,
                            color = MeowTheme.colors.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                // "Not triggered yet" is neutral, not a warning: for most sources it is
                // what a device with no crashes of that kind looks like. Only being
                // switched off, or an error the daemon recorded, earns a warning.
                StatusTag(
                    text = stringResource(
                        when {
                            !collector.enabled -> R.string.collector_disabled
                            collector.detail != null -> R.string.collector_error
                            collector.everReceived -> R.string.collector_receiving
                            else -> R.string.collector_idle
                        },
                    ),
                    tone = when {
                        collector.isImpaired -> StatusTagTone.Warning
                        collector.everReceived -> StatusTagTone.Success
                        else -> StatusTagTone.Neutral
                    },
                )
            }
        }
    }
}

@Composable
private fun StorageCard(state: HomeUiState) {
    val storage = state.moduleStatus.valueOrNull?.storage ?: return

    HomeSectionTitle(stringResource(R.string.home_section_storage))
    // The breakdown behind the single total in the stats row. Split this way because the
    // two halves behave differently under the retention policy: payloads are what gets
    // evicted first, while the index only shrinks when whole records go.
    HomeInfoCard(modifier = Modifier.testTag("crashcatcher.home.storage")) {
        HomeInfoEntry(
            stringResource(R.string.storage_payload_bytes),
            formatBytes(storage.payloadBytes),
        )
        HomeInfoEntry(
            stringResource(R.string.storage_database_bytes),
            formatBytes(storage.databaseBytes),
        )
        if (storage.evictedPayloadCount > 0) {
            // These records still exist; only their full stack was reclaimed.
            HomeInfoEntry(
                stringResource(R.string.storage_evicted),
                storage.evictedPayloadCount.toString(),
            )
        }
    }
}

private data class StatusVisuals(val icon: ImageVector, val headlineRes: Int)

/** Composable because the glyph follows the interface style; see [MeowIcons]. */
@Composable
@ReadOnlyComposable
private fun RuntimeStatus.visuals(): StatusVisuals = when (this) {
    RuntimeStatus.Running -> StatusVisuals(
        MeowIcons.Healthy,
        R.string.status_headline_running,
    )

    RuntimeStatus.Degraded -> StatusVisuals(
        MeowIcons.Warning,
        R.string.status_headline_degraded,
    )

    RuntimeStatus.Unreachable -> StatusVisuals(
        MeowIcons.Error,
        R.string.status_headline_unreachable,
    )

    RuntimeStatus.Checking -> StatusVisuals(
        MeowIcons.Pending,
        R.string.status_headline_checking,
    )
}

@Composable
private fun HomeUiState.supportingText(): String = when (runtimeStatus) {
    // "0/0 collectors" says nothing when nothing answered at all.
    RuntimeStatus.Unreachable -> stringResource(R.string.status_supporting_unreachable)
    RuntimeStatus.Checking -> stringResource(R.string.loading)
    RuntimeStatus.Degraded -> stringResource(
        R.string.status_supporting_degraded,
        impairedCollectors.size,
    )

    RuntimeStatus.Running -> stringResource(
        R.string.status_supporting_running,
        stats.valueOrNull?.total ?: 0,
    )
}

private val CollectorSource.labelRes: Int
    get() = when (this) {
        CollectorSource.Events -> R.string.collector_events
        CollectorSource.CrashBuffer -> R.string.collector_crash_buffer
        CollectorSource.Dropbox -> R.string.collector_dropbox
        CollectorSource.Tombstone -> R.string.collector_tombstone
        CollectorSource.AnrFile -> R.string.collector_anr_file
    }

/** Byte counts in the largest unit that keeps the number readable. */
internal fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val units = listOf("KiB", "MiB", "GiB")
    var value = bytes.toDouble() / 1024
    var unit = 0
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit++
    }
    return if (value >= 100) {
        "${value.toLong()} ${units[unit]}"
    } else {
        String.format("%.1f %s", value, units[unit])
    }
}

private const val SUPPORTING_ALPHA = 0.85f

/** Tint strength of the status badge, derived from the card's own content colour. */
private const val BADGE_ALPHA = 0.16f
