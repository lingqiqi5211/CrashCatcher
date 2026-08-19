package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.descriptors.element
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Requests, responses and events, mirroring `cch_wire::rpc` and `cch_wire::event`.
 */

/** Partial update to the global config. Absent fields are left alone. */
@Serializable
data class RetentionPatch(
    val retentionDays: Int? = null,
    val maxRecordsPerGroup: Int? = null,
    val maxRecordsTotal: Int? = null,
    val maxPayloadBytesTotal: Long? = null,
    val maxPayloadBytesPerRecord: Long? = null,
)

@Serializable
data class GlobalConfigPatch(
    val enabled: Boolean? = null,
    val captureJava: Boolean? = null,
    val captureAnr: Boolean? = null,
    val captureNative: Boolean? = null,
    val captureWtf: Boolean? = null,
    val captureSelfHandled: Boolean? = null,
    val notifyMode: NotifyMode? = null,
    val onlyForeground: Boolean? = null,
    val foregroundUnknownNotifies: Boolean? = null,
    val onlyMainProcess: Boolean? = null,
    val includeSystemApps: Boolean? = null,
    val takeoverSystemDialog: Boolean? = null,
    val retention: RetentionPatch = RetentionPatch(),
)

/**
 * What to do with an app's notify-mode override.
 *
 * Three states, not two: leaving the override alone and clearing it back to
 * "follow global" are different intentions, and collapsing them is how forwarding
 * an untouched patch silently wipes a user's setting.
 */
sealed interface NotifyModeChange {
    /** Absent from the request. */
    data object Unchanged : NotifyModeChange

    /** Sent as `null`: drop the override. */
    data object FollowGlobal : NotifyModeChange

    data class SetTo(val mode: NotifyMode) : NotifyModeChange
}

@Serializable(with = AppConfigPatchSerializer::class)
data class AppConfigPatch(
    val notifyMode: NotifyModeChange = NotifyModeChange.Unchanged,
    val ignore: Boolean? = null,
    val mute: MuteScope? = null,
)

/**
 * Hand-written because the three-state [NotifyModeChange] needs the key omitted,
 * present-and-null, or present-with-a-value — a distinction no generated serializer
 * can express from a single nullable property.
 */
object AppConfigPatchSerializer : KSerializer<AppConfigPatch> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("AppConfigPatch") {
            element<String>("notify_mode", isOptional = true)
            element<Boolean>("ignore", isOptional = true)
            element<String>("mute", isOptional = true)
        }

    override fun serialize(encoder: Encoder, value: AppConfigPatch) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: error("AppConfigPatch is only defined for the JSON transport")

        jsonEncoder.encodeJsonElement(
            buildJsonObject {
                when (val change = value.notifyMode) {
                    NotifyModeChange.Unchanged -> Unit
                    NotifyModeChange.FollowGlobal -> put("notify_mode", JsonNull)
                    is NotifyModeChange.SetTo ->
                        put("notify_mode", JsonPrimitive(change.mode.wireName))
                }
                value.ignore?.let { put("ignore", JsonPrimitive(it)) }
                value.mute?.let { put("mute", JsonPrimitive(it.wireName)) }
            },
        )
    }

    override fun deserialize(decoder: Decoder): AppConfigPatch {
        val jsonDecoder = decoder as? JsonDecoder
            ?: error("AppConfigPatch is only defined for the JSON transport")
        val obj = jsonDecoder.decodeJsonElement().jsonObject

        val notifyMode = when (val element = obj["notify_mode"]) {
            null -> NotifyModeChange.Unchanged
            JsonNull -> NotifyModeChange.FollowGlobal
            else -> NotifyModeChange.SetTo(notifyModeFromWire(element.jsonPrimitive.content))
        }
        return AppConfigPatch(
            notifyMode = notifyMode,
            ignore = obj["ignore"]?.takeIf { it != JsonNull }?.jsonPrimitive?.content?.toBooleanStrictOrNull(),
            mute = obj["mute"]?.takeIf { it != JsonNull }?.jsonPrimitive?.content?.let(::muteScopeFromWire),
        )
    }
}

/**
 * Where a delete request applies.
 *
 * Not serializable itself: the daemon flattens these fields next to `method`, so
 * [WireRequest.DeleteRecords] carries them directly and this type exists to stop a
 * caller from assembling an invalid combination. The private constructor is the
 * point — `all()` has to be asked for by name, so it can never be what an
 * accidentally-empty id list turns into.
 */
data class DeleteTarget private constructor(
    val target: String,
    val ids: List<RecordId>? = null,
    val groupId: String? = null,
) {
    companion object {
        fun ids(ids: List<RecordId>) = DeleteTarget(target = "ids", ids = ids)
        fun group(groupId: String) = DeleteTarget(target = "group", groupId = groupId)

        /** Everything. Named so it can never be the result of an empty list. */
        fun all() = DeleteTarget(target = "all")
    }
}

@Serializable
@JsonClassDiscriminator("method")
sealed class WireRequest {

    @Serializable
    @SerialName("handshake")
    data class Handshake(val protocolVersion: Int, val clientVersion: String) : WireRequest()

    @Serializable
    @SerialName("module_status")
    data object ModuleStatusRequest : WireRequest()

    @Serializable
    @SerialName("list_groups")
    data class ListGroups(val page: PageRequest = PageRequest()) : WireRequest()

    @Serializable
    @SerialName("get_group")
    data class GetGroup(val groupId: String) : WireRequest()

    @Serializable
    @SerialName("list_records")
    data class ListRecords(val groupId: String, val page: PageRequest = PageRequest()) :
        WireRequest()

    @Serializable
    @SerialName("get_record")
    data class GetRecord(val id: RecordId) : WireRequest()

    @Serializable
    @SerialName("open_payload")
    data class OpenPayload(val id: RecordId) : WireRequest()

    @Serializable
    @SerialName("read_payload")
    data class ReadPayload(val handle: Long, val offset: Long, val len: Int) : WireRequest()

    @Serializable
    @SerialName("close_payload")
    data class ClosePayload(val handle: Long) : WireRequest()

    @Serializable
    @SerialName("export_records")
    data class ExportRecords(
        val ids: List<RecordId>,
        val format: ExportFormat,
        val redaction: ExportRedaction = ExportRedaction(),
    ) : WireRequest()

    @Serializable
    @SerialName("delete_records")
    data class DeleteRecords(
        val target: String,
        val ids: List<RecordId>? = null,
        val groupId: String? = null,
    ) : WireRequest() {
        constructor(target: DeleteTarget) : this(target.target, target.ids, target.groupId)
    }

    @Serializable
    @SerialName("get_global_config")
    data object GetGlobalConfig : WireRequest()

    @Serializable
    @SerialName("set_global_config")
    data class SetGlobalConfig(val patch: GlobalConfigPatch) : WireRequest()

    @Serializable
    @SerialName("get_app_config")
    data class GetAppConfig(val packageName: String) : WireRequest()

    @Serializable
    @SerialName("set_app_config")
    data class SetAppConfig(val packageName: String, val patch: AppConfigPatch) : WireRequest()

    @Serializable
    @SerialName("list_apps")
    data class ListApps(
        val includeSystemApps: Boolean = false,
        val includeSystemProcesses: Boolean = false,
        val query: String? = null,
        val limit: Int = 0,
    ) : WireRequest()

    @Serializable
    @SerialName("stats")
    data class StatsRequest(
        val timeFromMs: Long? = null,
        val timeToMs: Long? = null,
        /** `0` lets the daemon choose the trend bucket width. */
        val bucketMs: Long = 0,
    ) : WireRequest()

    @Serializable
    @SerialName("reopen_app")
    data class ReopenApp(val packageName: String, val userId: Int) : WireRequest()

    @Serializable
    @SerialName("mute_app")
    data class MuteApp(val packageName: String, val scope: MuteScope) : WireRequest()

    /**
     * Takes down the notification posted for a crash.
     *
     * Goes through the daemon because the notification belongs to the privileged bridge
     * that posted it, and a process can only cancel its own.
     */
    @Serializable
    @SerialName("dismiss_notification")
    data class DismissNotification(val recordId: RecordId) : WireRequest()

    @Serializable
    @SerialName("set_dialog_takeover")
    data class SetDialogTakeover(val enabled: Boolean) : WireRequest()
}

/**
 * Responses.
 *
 * **No `@SerialName` here may equal one on [WireRequest].** kotlinx compares serial
 * descriptors by name, arity and element *types* — property names are not part of
 * that comparison — so two different classes both named `handshake` with the same
 * `(Int, String)` shape compared equal and shared one cached field-name mapping.
 * The response then looked for the request's `client_version`, and reported
 * `daemonVersion` missing while `daemon_version` sat right there in the JSON.
 *
 * The three tags that would naturally collide carry a `_result` suffix, and
 * `WireNameCollisionTest` fails if a new one ever collides again.
 */
@Serializable
@JsonClassDiscriminator("response")
sealed class WireResponse {

    @Serializable
    @SerialName("handshake_result")
    data class Handshake(val protocolVersion: Int, val daemonVersion: String) : WireResponse()

    @Serializable
    @SerialName("module_status_result")
    data class ModuleStatusResponse(val status: ModuleStatus) : WireResponse()

    @Serializable
    @SerialName("groups")
    data class Groups(val page: Page<GroupSummary>) : WireResponse()

    @Serializable
    @SerialName("group")
    data class Group(val group: GroupSummary) : WireResponse()

    @Serializable
    @SerialName("records")
    data class Records(val page: Page<RecordSummary>) : WireResponse()

    @Serializable
    @SerialName("record")
    data class Record(val detail: RecordDetail) : WireResponse()

    @Serializable
    @SerialName("payload_opened")
    data class PayloadOpenedResponse(val payload: PayloadOpened) : WireResponse()

    @Serializable
    @SerialName("payload_chunk")
    data class PayloadChunkResponse(val chunk: PayloadChunk) : WireResponse()

    @Serializable
    @SerialName("closed")
    data object Closed : WireResponse()

    @Serializable
    @SerialName("export")
    data class Export(val text: String) : WireResponse()

    @Serializable
    @SerialName("deleted")
    data class Deleted(val removedRecords: Long, val removedGroups: Long) : WireResponse()

    @Serializable
    @SerialName("global_config")
    data class GlobalConfigResponse(val result: GlobalConfigResult) : WireResponse()

    @Serializable
    @SerialName("app_config")
    data class AppConfigResponse(val result: AppConfigResult) : WireResponse()

    @Serializable
    @SerialName("apps")
    data class Apps(val apps: List<AppEntry> = emptyList()) : WireResponse()

    @Serializable
    @SerialName("stats_result")
    data class StatsResponse(val stats: Stats) : WireResponse()

    @Serializable
    @SerialName("reopened")
    data class Reopened(val launched: Boolean) : WireResponse()

    /** False when the bridge was away, so there was nothing posted to take down. */
    @Serializable
    @SerialName("notification_dismissed")
    data class NotificationDismissed(val dismissed: Boolean) : WireResponse()

    @Serializable
    @SerialName("muted")
    data class Muted(val result: MuteResult) : WireResponse()

    @Serializable
    @SerialName("dialog_takeover")
    data class DialogTakeover(val result: DialogTakeoverResult) : WireResponse()
}

@Serializable
data class RequestEnvelope(val seq: Long, val request: WireRequest)

/**
 * Exactly one of [ok] or [err] is present.
 *
 * Two nullables rather than a sealed wrapper because that is what the daemon emits;
 * [result] restores the either-or guarantee for callers.
 */
@Serializable
data class ResponseEnvelope(
    val seq: Long,
    val ok: WireResponse? = null,
    val err: WireError? = null,
) {
    fun result(): WireResponse = when {
        ok != null && err == null -> ok
        err != null && ok == null -> throw DaemonException.Rejected(err)
        else -> throw DaemonException.MalformedFrame(
            "response $seq carries ${if (ok == null) "neither" else "both"} ok and err",
        )
    }
}

@Serializable
@JsonClassDiscriminator("event")
sealed class WireEvent {

    @Serializable
    @SerialName("crash_recorded")
    data class CrashRecorded(
        val record: RecordSummary,
        val group: GroupSummary,
        /** First sighting of this fingerprint: add a row rather than increment one. */
        val isNewGroup: Boolean,
    ) : WireEvent()

    @Serializable
    @SerialName("config_changed")
    data object ConfigChanged : WireEvent()

    @Serializable
    @SerialName("module_status_changed")
    data class ModuleStatusChanged(val status: ModuleStatus) : WireEvent()

    /** Events were coalesced away because this client could not keep up. */
    @Serializable
    @SerialName("dropped")
    data class Dropped(val count: Long, val sinceMs: Long) : WireEvent()

    companion object {
        private val KNOWN = setOf(
            "crash_recorded",
            "config_changed",
            "module_status_changed",
            "dropped",
        )

        /**
         * Parses an event, returning `null` for a kind this build does not know.
         *
         * Skipping rather than failing: dropping the subscription on an unknown tag
         * would make every future protocol addition a breaking change for older
         * managers. A genuinely malformed frame still throws.
         */
        fun parseLenient(json: String): WireEvent? {
            val element = try {
                DaemonJson.parseToJsonElement(json)
            } catch (cause: Exception) {
                throw DaemonException.MalformedFrame("event is not JSON: ${cause.message}")
            }
            val obj = element as? JsonObject
                ?: throw DaemonException.MalformedFrame("event frame is not an object")
            val tag = obj["event"]?.jsonPrimitive?.content
                ?: throw DaemonException.MalformedFrame("event frame has no `event` tag")

            if (tag !in KNOWN) return null

            return try {
                DaemonJson.decodeFromJsonElement(serializer(), obj)
            } catch (cause: Exception) {
                throw DaemonException.MalformedFrame("event $tag did not parse: ${cause.message}")
            }
        }
    }
}

/** First frame on a connection, choosing which lane it is. */
@Serializable
data class ChannelHello(val kind: String) {
    companion object {
        val Control = ChannelHello("control")
        val Subscribe = ChannelHello("subscribe")
    }
}

// Wire names for the enums the hand-written serializer has to spell out itself.

internal val NotifyMode.wireName: String
    get() = when (this) {
        NotifyMode.Dialog -> "dialog"
        NotifyMode.Notification -> "notification"
        NotifyMode.Toast -> "toast"
        NotifyMode.Nothing -> "nothing"
    }

internal fun notifyModeFromWire(value: String): NotifyMode = when (value) {
    "dialog" -> NotifyMode.Dialog
    "notification" -> NotifyMode.Notification
    "toast" -> NotifyMode.Toast
    "nothing" -> NotifyMode.Nothing
    else -> throw DaemonException.MalformedFrame("unknown notify mode $value")
}

internal val MuteScope.wireName: String
    get() = when (this) {
        MuteScope.None -> "none"
        MuteScope.UntilUnlock -> "until_unlock"
        MuteScope.UntilRestart -> "until_restart"
    }

internal fun muteScopeFromWire(value: String): MuteScope = when (value) {
    "none" -> MuteScope.None
    "until_unlock" -> MuteScope.UntilUnlock
    "until_restart" -> MuteScope.UntilRestart
    else -> throw DaemonException.MalformedFrame("unknown mute scope $value")
}
