package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.lingqiqi5211.crashcatcher.ui.theme.isMiuixStyle
import io.github.lingqiqi5211.meowui.theme.MeowTheme
import top.yukonga.miuix.kmp.basic.Card as MiuixCard
import top.yukonga.miuix.kmp.basic.CardDefaults as MiuixCardDefaults
import top.yukonga.miuix.kmp.basic.Text as MiuixText

/**
 * The severity a [StatusTag] carries.
 *
 * Deliberately semantic rather than chromatic: a call site says what a record *is*
 * (`Error` for a native crash, `Warning` for an ANR, `Info` for one the app handled
 * itself) and the tone table below decides how that looks in the active palette.
 */
internal enum class StatusTagTone { Neutral, Success, Warning, Error, Info }

/**
 * A compact state label: `ANR`, `Native`, `已回收`, `应用自行处理`.
 *
 * MeowUI has no badge or chip component and `MeowCard` always fills its width, so the
 * container stays this app's own. Under Miuix it is a real Miuix `Card` so the
 * squircle is native rather than a rounded rectangle pretending to be one.
 *
 * The type metrics are stated explicitly instead of taken from a token because the
 * two styles' nearest body styles differ in size: a tag sitting in a list row has to
 * keep the same height whichever style is active, or the row's baseline grid shifts
 * when the user switches style.
 */
@Composable
internal fun StatusTag(
    text: String,
    tone: StatusTagTone,
    modifier: Modifier = Modifier,
) {
    val colors = toneColors(tone)
    if (isMiuixStyle()) {
        MiuixCard(
            modifier = modifier,
            insideMargin = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            colors = MiuixCardDefaults.defaultColors(color = colors.background),
        ) {
            MiuixText(
                text = text,
                color = colors.foreground,
                fontSize = TagFontSize,
                fontWeight = FontWeight.Medium,
                lineHeight = TagLineHeight,
            )
        }
        return
    }
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        color = colors.background,
        contentColor = colors.foreground,
    ) {
        Text(
            text = text,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            color = colors.foreground,
            style = MeowTheme.typography.value.copy(
                fontSize = TagFontSize,
                fontWeight = FontWeight.Medium,
                lineHeight = TagLineHeight,
            ),
        )
    }
}

private data class StatusTagColors(val background: Color, val foreground: Color)

/**
 * Tonal container pairs for each tone.
 *
 * From `MeowTheme.colors`, which now carries the container roles, so a tag is drawn in
 * whichever palette the active style actually uses. Reading Material's roles here — even
 * through the bridge, which derives a Material scheme from the Miuix seed — meant a Miuix
 * skin got Material's tonal containers: noticeably more tinted than the flat, grey
 * containers everything around them is painted with.
 */
@Composable
private fun toneColors(tone: StatusTagTone): StatusTagColors {
    val scheme = MeowTheme.colors
    return when (tone) {
        StatusTagTone.Neutral -> StatusTagColors(
            background = scheme.surfaceContainerHighest,
            foreground = scheme.onSurfaceContainerHighest,
        )
        // Success and warning come from MeowUI's semantic pair rather than from a palette
        // role: neither design system defines them, and the substitutes both had were
        // wrong in opposite ways — `tertiaryContainer` is the informational tint, so it
        // collided with Info, and Miuix's `secondaryContainer` is a mid grey for switch
        // tracks, which made a degraded collector look exactly like a neutral one.
        StatusTagTone.Success -> StatusTagColors(
            background = scheme.successContainer,
            foreground = scheme.onSuccessContainer,
        )
        StatusTagTone.Warning -> StatusTagColors(
            background = scheme.warningContainer,
            foreground = scheme.onWarningContainer,
        )
        StatusTagTone.Error -> StatusTagColors(
            background = scheme.errorContainer,
            foreground = scheme.onErrorContainer,
        )
        // The informational tint, not the filled accent. Miuix's `primaryContainer` is the
        // solid colour its filled buttons use, so "应用自行处理" came out as a saturated blue
        // chip shouting louder than the crash it was annotating.
        StatusTagTone.Info -> StatusTagColors(
            background = scheme.tertiaryContainer,
            foreground = scheme.onTertiaryContainer,
        )
    }
}

private val TagFontSize = 11.sp
private val TagLineHeight = 16.sp
