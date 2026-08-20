package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.encodeToJsonElement
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Cross-language wire vectors.
 *
 * The literals here are the same ones `crates/cch_wire/tests/vectors.rs` asserts, and
 * both come from `docs/wire-vectors.md`. A protocol change should turn both suites
 * red; one suite going red alone means the two sides have drifted.
 *
 * Comparison is on the parsed JSON tree with null-valued keys stripped, because on
 * this wire null and absent mean the same thing. The single documented exception —
 * an app config patch's `notify_mode` — is asserted on its exact shape instead.
 */
class WireVectorsTest {

    @Test
    fun `channel hello vectors`() {
        assertSerializes(ChannelHello.Control, """{"kind":"control"}""")
        assertSerializes(ChannelHello.Subscribe, """{"kind":"subscribe"}""")
    }

    @Test
    fun `list groups request vector`() {
        val envelope = RequestEnvelope(
            seq = 7,
            request = WireRequest.ListGroups(
                page = PageRequest(
                    filter = CrashFilter(
                        packages = listOf("com.example.app"),
                        kinds = listOf(CrashKind.Anr),
                    ),
                    sort = SortKey.OccurrenceDesc,
                    limit = 25,
                ),
            ),
        )

        assertSerializes(
            envelope,
            """
            {
              "seq": 7,
              "request": {
                "method": "list_groups",
                "page": {
                  "filter": {
                    "packages": ["com.example.app"],
                    "kinds": ["anr"],
                    "user_ids": [],
                    "include_system_apps": false,
                    "include_system_processes": false,
                    "only_main_process": false,
                    "only_self_handled": false
                  },
                  "sort": "occurrence_desc",
                  "limit": 25
                }
              }
            }
            """.trimIndent(),
        )
    }

    @Test
    fun `delete records flattens its target next to the method`() {
        val envelope = RequestEnvelope(
            seq = 8,
            request = WireRequest.DeleteRecords(
                DeleteTarget.group("0123456789abcdef0123456789abcdef"),
            ),
        )

        assertSerializes(
            envelope,
            """
            {
              "seq": 8,
              "request": {
                "method": "delete_records",
                "target": "group",
                "group_id": "0123456789abcdef0123456789abcdef"
              }
            }
            """.trimIndent(),
        )
    }

    @Test
    fun `an empty id list never becomes delete-all`() {
        val envelope = RequestEnvelope(
            seq = 8,
            request = WireRequest.DeleteRecords(DeleteTarget.ids(emptyList())),
        )
        val target = envelope.toJsonTree()
            .let { (it as JsonObject)["request"] as JsonObject }
            .let { it["target"].toString() }
        assertEquals("\"ids\"", target)
    }

    @Test
    fun `the three notify mode patch states have distinct shapes`() {
        // The one place where null and absent differ, so the exact shape is asserted
        // rather than normalized.
        val unchanged = AppConfigPatch().toJsonTree() as JsonObject
        assertNull(
            "an untouched patch must omit the key, not send null",
            unchanged["notify_mode"],
        )

        val followGlobal =
            AppConfigPatch(notifyMode = NotifyModeChange.FollowGlobal).toJsonTree() as JsonObject
        assertEquals(
            "clearing the override must send an explicit null",
            JsonNull,
            followGlobal["notify_mode"],
        )

        val setTo = AppConfigPatch(
            notifyMode = NotifyModeChange.SetTo(NotifyMode.Toast),
        ).toJsonTree() as JsonObject
        assertEquals("\"toast\"", setTo["notify_mode"].toString())
    }

    @Test
    fun `an untouched patch round trips without turning into a clear`() {
        // The regression this whole three-state design exists for: forwarding a patch
        // the user did not edit must not wipe their per-app override.
        val json = DaemonJson.encodeToString(AppConfigPatch())
        val parsed = DaemonJson.decodeFromString<AppConfigPatch>(json)
        assertEquals(NotifyModeChange.Unchanged, parsed.notifyMode)
    }

    @Test
    fun `patch states survive a round trip`() {
        val cases = listOf(
            NotifyModeChange.Unchanged,
            NotifyModeChange.FollowGlobal,
            NotifyModeChange.SetTo(NotifyMode.Dialog),
        )
        for (change in cases) {
            val patch = AppConfigPatch(notifyMode = change, ignore = true)
            val json = DaemonJson.encodeToString(patch)
            val parsed = DaemonJson.decodeFromString<AppConfigPatch>(json)
            assertEquals("round trip for $change", change, parsed.notifyMode)
            assertEquals(true, parsed.ignore)
        }
    }

    @Test
    fun `set app config request vectors`() {
        fun request(patch: AppConfigPatch, seq: Long) = RequestEnvelope(
            seq = seq,
            request = WireRequest.SetAppConfig("com.example.app", patch),
        )

        assertSerializes(
            request(AppConfigPatch(), 9),
            """{"seq":9,"request":{"method":"set_app_config","package_name":"com.example.app","patch":{}}}""",
        )

        // Normalizing would erase the meaningful null, so reach for it directly.
        val cleared = request(AppConfigPatch(notifyMode = NotifyModeChange.FollowGlobal), 10)
            .toJsonTree() as JsonObject
        val patch = (cleared["request"] as JsonObject)["patch"] as JsonObject
        assertEquals(JsonNull, patch["notify_mode"])

        assertSerializes(
            request(AppConfigPatch(notifyMode = NotifyModeChange.SetTo(NotifyMode.Toast)), 11),
            """{"seq":11,"request":{"method":"set_app_config","package_name":"com.example.app","patch":{"notify_mode":"toast"}}}""",
        )
    }

    @Test
    fun `mute request vector`() {
        assertSerializes(
            RequestEnvelope(
                seq = 13,
                request = WireRequest.MuteApp("com.example.app", MuteScope.UntilUnlock),
            ),
            """
            {
              "seq": 13,
              "request": {
                "method": "mute_app",
                "package_name": "com.example.app",
                "scope": "until_unlock"
              }
            }
            """.trimIndent(),
        )
    }

    @Test
    fun `response envelope vectors parse`() {
        val deleted = DaemonJson.decodeFromString<ResponseEnvelope>(
            """{"seq":8,"ok":{"response":"deleted","removed_records":3,"removed_groups":1}}""",
        )
        assertEquals(8L, deleted.seq)
        assertEquals(
            WireResponse.Deleted(removedRecords = 3, removedGroups = 1),
            deleted.result(),
        )

        val rejected = DaemonJson.decodeFromString<ResponseEnvelope>(
            """
            {"seq":8,"err":{"code":"cursor_invalidated","message":"cursor was issued for LastSeenDesc but the request sorts by PackageAsc"}}
            """.trimIndent(),
        )
        val failure = runCatching { rejected.result() }.exceptionOrNull()
        assertTrue(failure is DaemonException.Rejected)
        assertEquals(
            WireErrorCode.CursorInvalidated,
            (failure as DaemonException.Rejected).error.code,
        )

        val apps = DaemonJson.decodeFromString<ResponseEnvelope>(
            """{"seq":12,"ok":{"response":"apps","apps":[]}}""",
        )
        assertEquals(WireResponse.Apps(emptyList()), apps.result())
    }

    @Test
    fun `an envelope with neither or both is refused`() {
        for (json in listOf(
            """{"seq":1}""",
            """{"seq":1,"ok":{"response":"closed"},"err":{"code":"internal","message":"x"}}""",
        )) {
            val envelope = DaemonJson.decodeFromString<ResponseEnvelope>(json)
            val failure = runCatching { envelope.result() }.exceptionOrNull()
            assertTrue("$json should be refused", failure is DaemonException.MalformedFrame)
        }
    }

    @Test
    fun `a nested response payload is not flattened`() {
        // If the daemon ever flattened these, this decode would fail — which is the
        // point of pinning it.
        val status = DaemonJson.decodeFromString<ResponseEnvelope>(
            """
            {"seq":1,"ok":{"response":"module_status_result","status":{
              "daemon_version":"0.1.0","protocol_version":1,"uptime_ms":1234,
              "collectors":[{"source":"events","enabled":true,"ever_received":true,"last_received_ms":9}],
              "bridge_connected":true,
              "dialog_takeover":{"requested":false,"effective":false,"anr_show_background_conflict":false},
              "storage":{"group_count":2,"record_count":5,"payload_bytes":100,"database_bytes":200,"evicted_payload_count":0},
              "runtime":{"pid":2726,"abi":"aarch64","android_sdk":37,"selinux":"enforcing",
                "store_schema_version":2,"debug_logging":false,"active_mutes":1,
                "package_index":{"package_count":656,"system_flags_known":true},
                "bridge":{"connected":true,"version":"1","android_sdk":37}}
            }}}
            """.trimIndent(),
        ).result() as WireResponse.ModuleStatusResponse

        assertEquals("0.1.0", status.status.daemonVersion)
        assertEquals(1, status.status.collectors.size)
        assertEquals(CollectorSource.Events, status.status.collectors[0].source)
        assertTrue(status.status.collectors[0].everReceived)
        assertEquals(5L, status.status.storage.recordCount)
        // The diagnostics facts nest the same way, and each is a separate object rather than
        // fields flattened next to the status's own.
        assertEquals(2726, status.status.runtime.pid)
        assertEquals(656, status.status.runtime.packageIndex.packageCount)
        assertTrue(status.status.runtime.packageIndex.systemFlagsKnown)
        assertEquals(37, status.status.runtime.bridge.androidSdk)
    }

    @Test
    fun `cursor vectors`() {
        assertEquals(
            "1|last_seen_desc|i|1755440000123|0123456789abcdef0123456789abcdef",
            Cursor("1|last_seen_desc|i|1755440000123|0123456789abcdef0123456789abcdef").value,
        )

        // A cursor is opaque: it survives a round trip untouched, which is all the
        // client is allowed to rely on.
        val original = Cursor("1|package_asc|t|com.example.app|0123456789abcdef0123456789abcdef")
        val request = PageRequest(cursor = original, sort = SortKey.PackageAsc)
        val parsed = DaemonJson.decodeFromString<PageRequest>(DaemonJson.encodeToString(request))
        assertEquals(original, parsed.cursor)
    }

    @Test
    fun `event vectors`() {
        assertEquals(
            WireEvent.ConfigChanged,
            WireEvent.parseLenient("""{"event":"config_changed"}"""),
        )
        assertEquals(
            WireEvent.Dropped(count = 12, sinceMs = 1_755_440_000_000),
            WireEvent.parseLenient("""{"event":"dropped","count":12,"since_ms":1755440000000}"""),
        )
    }

    @Test
    fun `an unknown event is skipped and a broken known one is not`() {
        assertNull(WireEvent.parseLenient("""{"event":"invented_later","whatever":1}"""))

        val failure = runCatching { WireEvent.parseLenient("""{"event":"dropped"}""") }
            .exceptionOrNull()
        assertTrue(
            "a known event missing its fields must be reported",
            failure is DaemonException.MalformedFrame,
        )
    }

    @Test
    fun `unknown fields from a newer daemon are ignored`() {
        val group = DaemonJson.decodeFromString<GroupSummary>(
            """
            {"group_id":"g","package_name":"com.example.app","process_name":"com.example.app",
             "user_id":0,"kind":"java_exception","is_system_app":false,"is_main_process":true,
             "self_handled":false,"occurrence":3,"first_seen_ms":1,"last_seen_ms":2,
             "payload_bytes":0,"invented_by_a_newer_daemon":{"nested":true}}
            """.trimIndent(),
        )
        assertEquals(3L, group.occurrence)
    }

    // --- helpers ---

    private inline fun <reified T> assertSerializes(value: T, expectedJson: String) {
        val actual = normalize(DaemonJson.encodeToJsonElement(value))
        val expected = normalize(DaemonJson.parseToJsonElement(expectedJson))
        assertEquals(expected, actual)
    }

    private inline fun <reified T> T.toJsonTree(): JsonElement =
        DaemonJson.encodeToJsonElement(this)

    /** Drops null-valued keys so "absent" and "null" compare equal. */
    private fun normalize(element: JsonElement): JsonElement = when (element) {
        is JsonObject -> buildJsonObject {
            for ((key, value) in element) {
                if (value != JsonNull) put(key, normalize(value))
            }
        }

        is JsonArray -> buildJsonArray {
            for (value in element) add(normalize(value))
        }

        else -> element
    }
}
