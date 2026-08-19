package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.meowui.component.MeowCard

/**
 * A standalone tonal surface for content MeowUI has no page-level component for.
 *
 * The crash viewer's status surfaces pick their container from runtime state — an
 * unreachable daemon tints the whole card with the error role, a degraded collector
 * with a warning tone — which no preference row models. [color] and [contentColor]
 * are therefore first-class parameters rather than something a caller has to reach
 * into a theme for, and both default to the active style's ordinary card face so an
 * untinted card looks like every other card on the page.
 *
 * [index] and [count] make adjacent cards read as one grouped card under Material
 * (segmented corners, group gaps) while staying separate cards under Miuix; they are
 * for lists whose length comes from data and therefore cannot go through
 * `MeowPreferenceSection`'s compile-time collecting DSL.
 */
@Composable
internal fun TonalCard(
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    contentColor: Color = Color.Unspecified,
    contentPadding: PaddingValues = PaddingValues(16.dp),
    index: Int = 0,
    count: Int = 1,
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    // Delegating to MeowCard rather than composing a Material Surface and a Miuix
    // Card by hand: the library card already owns the squircle, the press feedback,
    // the segmented shapes and the per-style default container, and it resolves the
    // content colour down into LocalContentColor so a tinted card stays readable.
    // A hand-rolled version would have to keep all of that in step by hand.
    MeowCard(
        modifier = modifier,
        index = index,
        count = count,
        containerColor = color,
        contentColor = contentColor,
        contentPadding = contentPadding,
        onClick = onClick,
        content = content,
    )
}

/**
 * Inner padding for a row in a scrollable list of cards.
 *
 * Tighter than the card default, which is sized for a settings row that stands on its own.
 * A list is read by scanning down it, and the default 20dp on every edge turned three short
 * lines into a card most of a thumb tall — so a page held three rows where it should hold
 * five or six. Shared so the crash list and the app list stay the same height per line.
 */
internal val ListRowPadding = PaddingValues(horizontal = 16.dp, vertical = 14.dp)
