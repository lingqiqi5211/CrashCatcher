package io.github.lingqiqi5211.crashcatcher.data.daemon

import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.LoadState
import io.github.lingqiqi5211.crashcatcher.domain.model.ReconnectOutcome
import io.github.lingqiqi5211.crashcatcher.domain.model.withError
import io.github.lingqiqi5211.crashcatcher.domain.repository.AppInventoryRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.ConfigRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRecordedEvent
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.DeleteOutcome
import io.github.lingqiqi5211.crashcatcher.domain.repository.DialogTakeoverOutcome
import io.github.lingqiqi5211.crashcatcher.domain.repository.GlobalConfigUpdate
import io.github.lingqiqi5211.crashcatcher.domain.repository.ModuleStatusRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.RuntimeLogSnapshot
import io.github.lingqiqi5211.crashcatcher.domain.repository.PayloadChunkResult
import io.github.lingqiqi5211.crashcatcher.domain.repository.StatsRepository
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorKind
import io.github.lingqiqi5211.crashcatcher.domain.model.valueOrNull
import java.io.FileInputStream
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.stateIn

/**
 * Runs a request and turns any failure into a [DomainError].
 *
 * Every repository funnels through this so error translation happens exactly once
 * and no screen ends up branching on an exception type.
 */
private suspend inline fun <T> DaemonClient.ask(
    request: WireRequest,
    crossinline extract: (WireResponse) -> T,
): Result<T> = try {
    Result.success(extract(request(request).response))
} catch (cause: Exception) {
    Result.failure(DomainErrorException(cause.toDomainError()))
}

/** Carries a [DomainError] through `Result`, which needs a `Throwable`. */
class DomainErrorException(val error: DomainError) : Exception(error.message)

/** The [DomainError] behind a failed repository call. */
val Throwable.domainError: DomainError
    get() = (this as? DomainErrorException)?.error ?: toDomainError()

/** Narrows a reconnect's [Result] to what the message shown afterwards needs. */
internal fun Result<Unit>.toReconnectOutcome(): ReconnectOutcome =
    ReconnectOutcome(exceptionOrNull()?.domainError)

private fun unexpected(response: WireResponse): Nothing =
    throw DaemonException.ProtocolViolation(
        "unexpected response ${response::class.simpleName}",
    )

class DaemonModuleStatusRepository(
    private val client: DaemonClient,
    scope: CoroutineScope,
) : ModuleStatusRepository {

    private val state = MutableStateFlow<LoadState<ModuleStatus>>(LoadState.Loading)

    /**
     * The last status, marked stale the moment the connection goes.
     *
     * The second half is what keeps the overview honest. Nothing polls, so a daemon that dies
     * while the user is looking at the status card used to leave 运行中 on screen until they
     * pulled to refresh — even after another page had already failed a read and knew better.
     */
    override val status: StateFlow<LoadState<ModuleStatus>> =
        combine(state, client.connected) { status, connected ->
            // Only a status that was actually read gets downgraded. Before the first one, and
            // after a failed one, `state` already says what it is — and the app starts
            // disconnected, so anything else would open on an error.
            if (connected || status.valueOrNull == null) status else status.withError(CONNECTION_LOST)
        }.stateIn(scope, SharingStarted.Eagerly, LoadState.Loading)

    override suspend fun refresh() {
        query()
    }

    override suspend fun reconnect(): Result<Unit> {
        client.disconnect()
        return query().map { }
    }

    private suspend fun query(): Result<ModuleStatus> = client
        .ask(WireRequest.ModuleStatusRequest) { response ->
            (response as? WireResponse.ModuleStatusResponse)?.status ?: unexpected(response)
        }
        .onSuccess { state.value = LoadState.Content(it) }
        // Keep the card on screen and mark it stale rather than blanking out: a
        // momentary disconnect should not erase what the user was reading.
        .onFailure { state.value = state.value.withError(it.domainError) }

    private companion object {
        val CONNECTION_LOST = DomainError(
            kind = DomainErrorKind.ConnectionLost,
            message = "daemon connection dropped",
        )
    }
}

class DaemonCrashRepository(
    private val client: DaemonClient,
) : CrashRepository {

    private val recorded = MutableSharedFlow<CrashRecordedEvent>(extraBufferCapacity = 32)
    override val crashRecorded: Flow<CrashRecordedEvent> = recorded.asSharedFlow()

    // Buffered rather than conflated: several deletes in a row must each reach every
    // listener, and a dropped one leaves a screen showing rows that are gone.
    private val changed = MutableSharedFlow<Unit>(extraBufferCapacity = 8)
    override val dataChanged: Flow<Unit> = changed.asSharedFlow()

    /** Called by the subscribe lane when the daemon pushes a new occurrence. */
    suspend fun onCrashRecorded(event: WireEvent.CrashRecorded) {
        recorded.emit(
            CrashRecordedEvent(
                record = event.record,
                group = event.group,
                isNewGroup = event.isNewGroup,
            ),
        )
    }

    override suspend fun listGroups(
        filter: CrashFilter,
        sort: SortKey,
        cursor: Cursor?,
        limit: Int,
    ) = client.ask(
        WireRequest.ListGroups(PageRequest(filter, sort, cursor, limit)),
    ) { response -> (response as? WireResponse.Groups)?.page ?: unexpected(response) }

    override suspend fun getGroup(groupId: String) = client.ask(
        WireRequest.GetGroup(groupId),
    ) { response -> (response as? WireResponse.Group)?.group ?: unexpected(response) }

    override suspend fun listRecords(groupId: String, cursor: Cursor?, limit: Int) = client.ask(
        WireRequest.ListRecords(groupId, PageRequest(cursor = cursor, limit = limit)),
    ) { response -> (response as? WireResponse.Records)?.page ?: unexpected(response) }

    override suspend fun getRecord(id: RecordId) = client.ask(WireRequest.GetRecord(id)) { response ->
        (response as? WireResponse.Record)?.detail ?: unexpected(response)
    }

    /**
     * Streams a payload, preferring the descriptor the daemon passes over the socket.
     *
     * The descriptor path skips framing, JSON escaping and the frame size limit
     * entirely, which is what makes opening a large ANR dump feel instant. The
     * chunked path is only for hosts where descriptor passing is unavailable.
     */
    override fun payloadText(id: RecordId): Flow<PayloadChunkResult> = flow {
        val reply = client.request(WireRequest.OpenPayload(id))
        val opened = (reply.response as? WireResponse.PayloadOpenedResponse)?.payload
            ?: unexpected(reply.response)

        // The arriving descriptor is the condition, not the daemon's `fdAttached` flag.
        // That flag is its intent; delivery can still fail for reasons only this side
        // sees — Android's `LocalSocket` truncates the control message on some ROMs, and
        // SELinux can refuse the app access to the received memfd. Both happen on
        // HyperOS, and trusting the flag turned every crash detail into a blank page.
        // Falling through to the handle costs a few round trips and always works.
        val descriptor = reply.fileDescriptors?.firstOrNull()
        if (descriptor != null) {
            var read = 0L
            var emitted = false
            try {
                FileInputStream(descriptor).use { stream ->
                    val buffer = ByteArray(DESCRIPTOR_CHUNK_BYTES)
                    var carry = ByteArray(0)
                    while (true) {
                        val count = stream.read(buffer)
                        if (count <= 0) break
                        // A read can land mid-character; hold the trailing bytes back
                        // rather than emitting a replacement character.
                        val combined = carry + buffer.copyOf(count)
                        val safeEnd = lastCharBoundary(combined)
                        carry = combined.copyOfRange(safeEnd, combined.size)
                        read += count
                        emitted = true
                        emit(
                            PayloadChunkResult(
                                text = combined.copyOfRange(0, safeEnd).decodeToString(),
                                bytesRead = read,
                                totalBytes = opened.totalBytes,
                                eof = false,
                            ),
                        )
                    }
                    if (carry.isNotEmpty()) {
                        emitted = true
                        emit(
                            PayloadChunkResult(
                                text = carry.decodeToString(),
                                bytesRead = read,
                                totalBytes = opened.totalBytes,
                                eof = false,
                            ),
                        )
                    }
                }
                emit(PayloadChunkResult("", read, opened.totalBytes, eof = true))
                return@flow
            } catch (cause: IOException) {
                // The descriptor arrived but is unusable — SELinux refusing the
                // received memfd is the case seen in the wild. Fall back to chunked
                // reads, but only while nothing has been emitted: re-reading from the
                // start after partial output would duplicate the text the reader can
                // already see, and a stack trace with a repeated middle is worse than
                // an error.
                if (emitted) throw cause
            }
        }

        val handle = opened.handle
            ?: throw DaemonException.ProtocolViolation(
                "payload came with neither a descriptor nor a handle",
            )
        try {
            var offset = 0L
            while (true) {
                val chunkReply = client.request(
                    WireRequest.ReadPayload(handle, offset, CHUNK_REQUEST_BYTES),
                )
                val chunk = (chunkReply.response as? WireResponse.PayloadChunkResponse)?.chunk
                    ?: unexpected(chunkReply.response)
                emit(
                    PayloadChunkResult(
                        text = chunk.text,
                        bytesRead = chunk.nextOffset,
                        totalBytes = opened.totalBytes,
                        eof = chunk.eof,
                    ),
                )
                if (chunk.eof) break
                if (chunk.nextOffset <= offset) {
                    throw DaemonException.ProtocolViolation("payload reader made no progress")
                }
                offset = chunk.nextOffset
            }
        } finally {
            runCatching { client.request(WireRequest.ClosePayload(handle)) }
        }
    }

    override suspend fun delete(target: DeleteTarget) = client.ask(
        WireRequest.DeleteRecords(target),
    ) { response ->
        val deleted = (response as? WireResponse.Deleted) ?: unexpected(response)
        DeleteOutcome(deleted.removedRecords, deleted.removedGroups)
    }.onSuccess {
        // Announced only on success: a failed delete changed nothing, and refreshing
        // every screen over it would make a transient error look like data loss.
        changed.emit(Unit)
    }

    override suspend fun export(
        ids: List<RecordId>,
        format: ExportFormat,
        redaction: ExportRedaction,
    ) = client.ask(WireRequest.ExportRecords(ids, format, redaction)) { response ->
        (response as? WireResponse.Export)?.text ?: unexpected(response)
    }

    override suspend fun dismissNotification(id: RecordId) = client
        .ask(WireRequest.DismissNotification(id)) { response ->
            (response as? WireResponse.NotificationDismissed)?.dismissed ?: unexpected(response)
        }

    private companion object {
        const val DESCRIPTOR_CHUNK_BYTES = 64 * 1024
        const val CHUNK_REQUEST_BYTES = 256 * 1024

        /** Index just past the last complete UTF-8 character in [bytes]. */
        fun lastCharBoundary(bytes: ByteArray): Int {
            var index = bytes.size
            // Continuation bytes are 10xxxxxx; walk back over at most three of them.
            var steps = 0
            while (index > 0 && steps < 4 && (bytes[index - 1].toInt() and 0xC0) == 0x80) {
                index--
                steps++
            }
            if (index == 0) return bytes.size
            val lead = bytes[index - 1].toInt() and 0xFF
            val expected = when {
                lead < 0x80 -> 1
                lead in 0xC0..0xDF -> 2
                lead in 0xE0..0xEF -> 3
                lead in 0xF0..0xF7 -> 4
                else -> 1
            }
            val available = bytes.size - (index - 1)
            return if (available >= expected) bytes.size else index - 1
        }
    }
}

class DaemonConfigRepository(
    private val client: DaemonClient,
) : ConfigRepository {

    private val state = MutableStateFlow<LoadState<GlobalConfig>>(LoadState.Loading)
    override val globalConfig: StateFlow<LoadState<GlobalConfig>> = state.asStateFlow()

    override suspend fun refreshGlobalConfig() {
        client
            .ask(WireRequest.GetGlobalConfig) { response ->
                (response as? WireResponse.GlobalConfigResponse)?.result?.config
                    ?: unexpected(response)
            }
            .onSuccess { state.value = LoadState.Content(it) }
            .onFailure { state.value = state.value.withError(it.domainError) }
    }

    override suspend fun updateGlobalConfig(patch: GlobalConfigPatch) = client
        .ask(WireRequest.SetGlobalConfig(patch)) { response ->
            val result = (response as? WireResponse.GlobalConfigResponse)?.result
                ?: unexpected(response)
            GlobalConfigUpdate(result.config, result.adjusted)
        }
        .onSuccess { state.value = LoadState.Content(it.config) }

    override suspend fun appConfig(packageName: String) = client
        .ask(WireRequest.GetAppConfig(packageName)) { response ->
            (response as? WireResponse.AppConfigResponse)?.result?.config ?: unexpected(response)
        }

    override suspend fun updateAppConfig(packageName: String, patch: AppConfigPatch) = client
        .ask(WireRequest.SetAppConfig(packageName, patch)) { response ->
            (response as? WireResponse.AppConfigResponse)?.result?.config ?: unexpected(response)
        }

    override suspend fun setDialogTakeover(enabled: Boolean) = client
        .ask(WireRequest.SetDialogTakeover(enabled)) { response ->
            val status = (response as? WireResponse.DialogTakeover)?.result?.status
                ?: unexpected(response)
            DialogTakeoverOutcome(
                effective = status.effective,
                anrShowBackgroundConflict = status.anrShowBackgroundConflict,
                unsupportedReason = status.unsupportedReason,
            )
        }

    override suspend fun mute(packageName: String, scope: MuteScope) = client
        .ask(WireRequest.MuteApp(packageName, scope)) { response ->
            if (response !is WireResponse.Muted) unexpected(response)
        }

    override suspend fun runtimeLog(name: String?, maxBytes: Long) = client
        .ask(WireRequest.ReadRuntimeLog(name, maxBytes)) { response ->
            val log = response as? WireResponse.RuntimeLog ?: unexpected(response)
            RuntimeLogSnapshot(
                name = log.name,
                text = log.text,
                truncated = log.truncated,
                totalBytes = log.totalBytes,
                files = log.files,
            )
        }
}

class DaemonAppInventoryRepository(
    private val client: DaemonClient,
) : AppInventoryRepository {

    override suspend fun listApps(
        includeSystemApps: Boolean,
        includeSystemProcesses: Boolean,
        query: String?,
        limit: Int,
    ) = client
        .ask(WireRequest.ListApps(includeSystemApps, includeSystemProcesses, query, limit)) { response ->
            (response as? WireResponse.Apps)?.apps ?: unexpected(response)
        }

    override suspend fun reopen(packageName: String, userId: Int) = client
        .ask(WireRequest.ReopenApp(packageName, userId)) { response ->
            (response as? WireResponse.Reopened)?.launched ?: unexpected(response)
        }
}

class DaemonStatsRepository(
    private val client: DaemonClient,
) : StatsRepository {

    private val state = MutableStateFlow<LoadState<Stats>>(LoadState.Loading)
    override val stats: StateFlow<LoadState<Stats>> = state.asStateFlow()

    override suspend fun refresh(timeFromMs: Long?, timeToMs: Long?) {
        client
            .ask(WireRequest.StatsRequest(timeFromMs, timeToMs)) { response ->
                (response as? WireResponse.StatsResponse)?.stats ?: unexpected(response)
            }
            .onSuccess { state.value = if (it.total == 0L) LoadState.Empty else LoadState.Content(it) }
            .onFailure { state.value = state.value.withError(it.domainError) }
    }
}
