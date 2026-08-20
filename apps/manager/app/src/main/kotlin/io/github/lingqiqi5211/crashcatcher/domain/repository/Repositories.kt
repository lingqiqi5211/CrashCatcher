package io.github.lingqiqi5211.crashcatcher.domain.repository

import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppEntry
import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashFilter
import io.github.lingqiqi5211.crashcatcher.data.daemon.Cursor
import io.github.lingqiqi5211.crashcatcher.data.daemon.DeleteTarget
import io.github.lingqiqi5211.crashcatcher.data.daemon.ExportFormat
import io.github.lingqiqi5211.crashcatcher.data.daemon.ExportRedaction
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.ModuleStatus
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.Page
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordDetail
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.SortKey
import io.github.lingqiqi5211.crashcatcher.data.daemon.Stats
import io.github.lingqiqi5211.crashcatcher.domain.model.LoadState
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/**
 * Repository contracts.
 *
 * Each exposes long-lived state as `StateFlow<LoadState<T>>` plus explicit suspend
 * refreshers, so a ViewModel can `combine` several without owning any loading
 * bookkeeping of its own. One-shot operations return a result instead of pushing
 * into the flow, because the caller needs to know whether *its* action succeeded.
 */

interface ModuleStatusRepository {
    val status: StateFlow<LoadState<ModuleStatus>>

    suspend fun refresh()

    /**
     * Drops and re-establishes the connection, for the settings screen's action.
     *
     * Returns whether the daemon answered afterwards. [status] carries the same outcome,
     * but a user who pressed a button needs to be told what *their* press did, and a
     * failed reconnect leaves the status flow looking exactly as it did before.
     */
    suspend fun reconnect(): Result<Unit>
}

/**
 * Paged access to crash history.
 *
 * Pages are requested explicitly rather than exposed as a hot flow: the cursor is
 * only meaningful relative to the page that produced it, and a flow that silently
 * restarted the query would hand the UI a cursor from a different result set.
 */
interface CrashRepository {
    suspend fun listGroups(
        filter: CrashFilter,
        sort: SortKey,
        cursor: Cursor?,
        limit: Int,
    ): Result<Page<GroupSummary>>

    suspend fun getGroup(groupId: String): Result<GroupSummary>

    suspend fun listRecords(
        groupId: String,
        cursor: Cursor?,
        limit: Int,
    ): Result<Page<RecordSummary>>

    suspend fun getRecord(id: RecordId): Result<RecordDetail>

    /**
     * The full stack text for a record.
     *
     * Streams the descriptor the daemon hands over when it can, and falls back to
     * chunked reads otherwise. Emits progressively so the detail screen can paint
     * the first lines of a multi-megabyte dump immediately.
     */
    fun payloadText(id: RecordId): Flow<PayloadChunkResult>

    suspend fun delete(target: DeleteTarget): Result<DeleteOutcome>

    /**
     * Fires after anything is deleted, so every screen holding crash data can drop it.
     *
     * A deletion invalidates far more than the screen that triggered it: the list still
     * shows rows that are gone, the per-app counts are wrong, the overview's storage
     * figures are stale, and an open detail page is looking at a record that no longer
     * exists. Having each caller remember to notify the others is how one of them ends
     * up forgotten, so the repository that performed the delete announces it instead.
     *
     * Carries no payload deliberately. "Something was deleted, re-read what you show" is
     * all a listener needs, and a diff would have to describe cascades — deleting a
     * group's last record removes the group too — that every listener would then have to
     * interpret identically.
     */
    val dataChanged: Flow<Unit>

    suspend fun export(
        ids: List<RecordId>,
        format: ExportFormat,
        redaction: ExportRedaction,
    ): Result<String>

    /**
     * Takes down the notification posted for [id].
     *
     * Has to go through the daemon: the notification belongs to the privileged bridge that
     * posted it, and a process can only cancel notifications of its own. Without this,
     * pressing one of its buttons left it on screen looking like nothing had happened.
     */
    suspend fun dismissNotification(id: RecordId): Result<Boolean>

    /** Emitted whenever a new occurrence lands, so an open list can refresh itself. */
    val crashRecorded: Flow<CrashRecordedEvent>
}

data class DeleteOutcome(val removedRecords: Long, val removedGroups: Long)

data class CrashRecordedEvent(
    val record: RecordSummary,
    val group: GroupSummary,
    val isNewGroup: Boolean,
)

/** One slice of payload text, with enough context for the UI to show progress. */
data class PayloadChunkResult(
    val text: String,
    val bytesRead: Long,
    val totalBytes: Long,
    val eof: Boolean,
)

interface ConfigRepository {
    val globalConfig: StateFlow<LoadState<GlobalConfig>>

    suspend fun refreshGlobalConfig()

    /**
     * Applies a partial update and returns what was actually stored.
     *
     * The stored value, not the requested one: the daemon clamps retention values,
     * so a slider that assumed its own number would drift out of step with reality.
     */
    suspend fun updateGlobalConfig(patch: GlobalConfigPatch): Result<GlobalConfigUpdate>

    suspend fun appConfig(packageName: String): Result<AppConfig>

    suspend fun updateAppConfig(packageName: String, patch: AppConfigPatch): Result<AppConfig>

    suspend fun setDialogTakeover(enabled: Boolean): Result<DialogTakeoverOutcome>

    suspend fun mute(packageName: String, scope: MuteScope): Result<Unit>

    /**
     * The tail of the daemon's own logs.
     *
     * On the config repository because it is read for the same reason the settings are — someone
     * is working out why the module is not behaving — and it is the one answer that stays useful
     * when the rest of the app is showing errors.
     */
    suspend fun runtimeLog(maxBytes: Long = 0): Result<RuntimeLogSnapshot>
}

data class RuntimeLogSnapshot(
    val text: String,
    /** Something was cut from the front; what is shown is the end. */
    val truncated: Boolean,
    val totalBytes: Long,
)

data class GlobalConfigUpdate(val config: GlobalConfig, val adjusted: Boolean)

data class DialogTakeoverOutcome(
    val effective: Boolean,
    /** `anr_show_background` is on and overrides the suppression. */
    val anrShowBackgroundConflict: Boolean,
    val unsupportedReason: String?,
)

interface AppInventoryRepository {
    suspend fun listApps(
        includeSystemApps: Boolean,
        includeSystemProcesses: Boolean,
        query: String?,
        limit: Int,
    ): Result<List<AppEntry>>

    suspend fun reopen(packageName: String, userId: Int): Result<Boolean>
}

interface StatsRepository {
    val stats: StateFlow<LoadState<Stats>>

    suspend fun refresh(timeFromMs: Long?, timeToMs: Long?)
}
