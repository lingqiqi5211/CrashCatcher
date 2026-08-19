package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.descriptors.SerialDescriptor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Guards the protocol against a kotlinx.serialization trap that produces a silent,
 * badly-misleading failure.
 *
 * `SerialDescriptor.equals` compares serial name, element count and element *types*.
 * Property names are not part of it. So two unrelated classes both tagged
 * `handshake`, each `(Int, String)`, compared **equal** — and the JSON field-name
 * cache, keyed by descriptor, handed the response the request's field mapping. The
 * decoder then insisted `daemonVersion` was missing while `daemon_version` was
 * plainly present in the payload.
 *
 * Nothing about that error points at the real cause, so it is worth a test that
 * fails loudly the moment a new tag collides.
 */
class WireNameCollisionTest {

    @OptIn(ExperimentalSerializationApi::class)
    @Test
    fun `no response tag collides with a request tag`() {
        val requestTags = sealedTags(WireRequest.serializer().descriptor)
        val responseTags = sealedTags(WireResponse.serializer().descriptor)

        assertTrue("no request tags were discovered — the reflection broke", requestTags.isNotEmpty())
        assertTrue("no response tags were discovered — the reflection broke", responseTags.isNotEmpty())

        val collisions = requestTags intersect responseTags
        assertEquals(
            "a request and a response share a wire tag; give the response a `_result` " +
                "suffix, or kotlinx may quietly swap their field mappings",
            emptySet<String>(),
            collisions,
        )
    }

    @OptIn(ExperimentalSerializationApi::class)
    @Test
    fun `the two handshake shapes have distinct descriptors`() {
        // The specific pair that broke. Equal descriptors here means equal cache keys.
        assertNotEquals(
            WireRequest.Handshake.serializer().descriptor,
            WireResponse.Handshake.serializer().descriptor,
        )
    }

    @Test
    fun `both handshake shapes decode their own fields`() {
        val request = DaemonJson.decodeFromString<RequestEnvelope>(
            """{"seq":1,"request":{"method":"handshake","protocol_version":1,"client_version":"0.1.0"}}""",
        )
        assertEquals(
            WireRequest.Handshake(protocolVersion = 1, clientVersion = "0.1.0"),
            request.request,
        )

        val response = DaemonJson.decodeFromString<ResponseEnvelope>(
            """{"seq":1,"ok":{"response":"handshake_result","protocol_version":1,"daemon_version":"0.1.0"}}""",
        )
        assertEquals(
            WireResponse.Handshake(protocolVersion = 1, daemonVersion = "0.1.0"),
            response.result(),
        )
    }

    @Test
    fun `decoding one shape does not poison the other`() {
        // The original failure only appeared in this order: the request descriptor was
        // cached first, then the response reused its field mapping.
        DaemonJson.decodeFromString<RequestEnvelope>(
            """{"seq":1,"request":{"method":"handshake","protocol_version":1,"client_version":"0.1.0"}}""",
        )
        val response = DaemonJson.decodeFromString<ResponseEnvelope>(
            """{"seq":2,"ok":{"response":"handshake_result","protocol_version":1,"daemon_version":"9.9.9"}}""",
        ).result() as WireResponse.Handshake

        assertEquals("9.9.9", response.daemonVersion)
    }

    /**
     * Wire tags of a sealed hierarchy, read from its own descriptor.
     *
     * A sealed serializer's descriptor has two elements — the discriminator and a
     * `value` element whose element *names* are the subclass serial names. Reading
     * them here rather than reflecting over `sealedSubclasses` means the test sees
     * exactly what the serializer sees, including any `@SerialName` override.
     */
    @OptIn(ExperimentalSerializationApi::class)
    private fun sealedTags(descriptor: SerialDescriptor): Set<String> {
        val values = descriptor.getElementDescriptor(VALUE_ELEMENT_INDEX)
        return (0 until values.elementsCount).map { values.getElementName(it) }.toSet()
    }

    private companion object {
        const val VALUE_ELEMENT_INDEX = 1
    }
}
