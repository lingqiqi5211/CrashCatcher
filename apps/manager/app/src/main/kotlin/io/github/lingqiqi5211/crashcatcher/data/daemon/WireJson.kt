package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNamingStrategy

/**
 * The single [Json] configuration used on this socket.
 *
 * - `SnakeCase` because every field on the Rust side is snake_case; declaring the
 *   mapping once beats an `@SerialName` on a hundred properties, each of which
 *   could be forgotten.
 * - `ignoreUnknownKeys` because the protocol grows by adding fields. A newer daemon
 *   must not be able to break an older manager simply by sending more.
 * - `explicitNulls = false` so an absent optional stays absent instead of becoming
 *   `null`, which matters for patch semantics where the two differ.
 * - `classDiscriminator` is set per sealed hierarchy via `@JsonClassDiscriminator`,
 *   matching serde's internal tags (`method`, `response`, `event`).
 */
@OptIn(ExperimentalSerializationApi::class)
val DaemonJson: Json = Json {
    namingStrategy = JsonNamingStrategy.SnakeCase
    ignoreUnknownKeys = true
    explicitNulls = false
    encodeDefaults = true
}
