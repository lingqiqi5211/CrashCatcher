package io.github.lingqiqi5211.crashcatcher.ui.components

import android.content.Context
import android.content.pm.PackageManager
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Rendered size of the icon bitmap, in pixels. */
private const val ICON_PIXELS = 144

/**
 * Loaded launcher icons, keyed by package name.
 *
 * Process-wide and bounded rather than remembered per composition: the crash list and
 * the app list show the same packages, a rotation or a tab switch would otherwise
 * decode every icon again, and decoding on the scrolling frame is exactly what makes a
 * list feel cheap.
 *
 * A package with no loadable icon caches as absent, so a missing icon costs one
 * PackageManager miss rather than one on every scroll pass.
 */
private object AppIconCache {
    private val icons = LruCache<String, Holder>(96)

    /** Boxed so "looked up, found nothing" is distinguishable from "not looked up". */
    private class Holder(val icon: ImageBitmap?)

    fun peek(packageName: String): ImageBitmap? = icons[packageName]?.icon

    suspend fun load(context: Context, packageName: String): ImageBitmap? {
        icons[packageName]?.let { return it.icon }
        val icon = withContext(Dispatchers.IO) { decode(context, packageName) }
        icons.put(packageName, Holder(icon))
        return icon
    }

    private fun decode(context: Context, packageName: String): ImageBitmap? = try {
        context.packageManager
            .getApplicationIcon(packageName)
            .toBitmap(width = ICON_PIXELS, height = ICON_PIXELS)
            .asImageBitmap()
    } catch (_: PackageManager.NameNotFoundException) {
        // Uninstalled since the crash was recorded. The record still matters, so this
        // is an ordinary outcome rather than an error worth surfacing.
        null
    }
}

/**
 * An app's launcher icon, with a readable placeholder while it loads or if it cannot.
 *
 * [label] seeds the placeholder's initial, so a row never renders as an empty square:
 * the fallback still says *which* app, which is the only job the icon has here.
 *
 * [isProcess] switches to a glyph instead. A platform process has no icon to find and no
 * useful initial either — every `/vendor/…` and `/system/…` binary would render as the same
 * `V` or `S` — so the slot says "not an app" rather than naming one badly.
 */
@Composable
internal fun AppIcon(
    packageName: String,
    label: String?,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
    isProcess: Boolean = false,
) {
    if (isProcess) {
        Box(
            modifier = modifier
                .size(size)
                .background(
                    MeowTheme.colors.surfaceContainerHighest,
                    RoundedCornerShape(size / 4),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = MeowIcons.SystemProcess,
                contentDescription = null,
                tint = MeowTheme.colors.onSurfaceVariant,
                modifier = Modifier.size(size * 0.55f),
            )
        }
        return
    }

    val context = LocalContext.current.applicationContext
    // Previews have no real PackageManager worth querying, and the placeholder is the
    // more useful thing to see while designing a row.
    val inspecting = LocalInspectionMode.current

    // Always assign from the cache rather than skipping when it looks warm: the first
    // composition of a row can start before its icon is cached and the `initialValue`
    // is only consulted once, which left a warm icon showing its placeholder. `load`
    // returns immediately on a hit, so the unconditional path costs nothing.
    val icon by produceState(
        initialValue = AppIconCache.peek(packageName),
        packageName,
        inspecting,
    ) {
        if (!inspecting) {
            value = AppIconCache.load(context, packageName)
        }
    }

    val shape = RoundedCornerShape(size / 4)
    val resolved = icon

    if (resolved == null) {
        Box(
            modifier = modifier
                .size(size)
                .background(MeowTheme.colors.secondaryContainer, shape),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = initialOf(label ?: packageName),
                color = MeowTheme.colors.onSecondaryContainer,
                style = MeowTheme.typography.title,
                fontWeight = FontWeight.SemiBold,
            )
        }
        return
    }

    Image(
        bitmap = resolved,
        // Decorative: every call site puts the app's name next to it, and announcing
        // "icon" as well would read the same app out twice.
        contentDescription = null,
        modifier = modifier
            .size(size)
            .clip(shape),
        contentScale = ContentScale.Fit,
    )
}

/**
 * The first character worth showing for a name.
 *
 * Skips the leading dots and separators a package name is full of, so
 * `io.github.example` yields `I` rather than a stop that identifies nothing.
 */
private fun initialOf(name: String): String =
    name.firstOrNull { it.isLetterOrDigit() }?.uppercase() ?: "?"

/** Resolved application labels, keyed by package name. */
private object AppLabelCache {
    private val labels = LruCache<String, String>(256)

    fun peek(packageName: String): String? = labels[packageName]

    suspend fun load(context: Context, packageName: String): String? {
        labels[packageName]?.let { return it }
        val label = withContext(Dispatchers.IO) { resolve(context, packageName) }
        if (label != null) labels.put(packageName, label)
        return label
    }

    private fun resolve(context: Context, packageName: String): String? = try {
        val manager = context.packageManager
        manager.getApplicationLabel(manager.getApplicationInfo(packageName, 0))
            .toString()
            .takeIf { it.isNotBlank() && it != packageName }
    } catch (_: PackageManager.NameNotFoundException) {
        null
    }
}

/**
 * An app's display name, or null while it resolves or if the package is gone.
 *
 * Read from PackageManager here rather than carried through the wire protocol: the
 * label is a purely local, presentational fact, it costs one cached lookup, and asking
 * the daemon for it would mean a crash record's heading depended on the privileged
 * bridge having enriched it — so an uninstalled app or a bridge hiccup would leave the
 * page titled with a raw package name.
 */
@Composable
internal fun rememberAppLabel(packageName: String): String? {
    val context = LocalContext.current.applicationContext
    val inspecting = LocalInspectionMode.current

    val label by produceState(
        initialValue = AppLabelCache.peek(packageName),
        packageName,
        inspecting,
    ) {
        if (!inspecting && packageName.isNotEmpty()) {
            value = AppLabelCache.load(context, packageName)
        }
    }
    return label
}
