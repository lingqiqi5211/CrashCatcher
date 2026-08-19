package io.github.lingqiqi5211.crashcatcher.ui.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.ui.components.TonalCard
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * The overview page's information card: label-over-value pairs in one untitled card.
 *
 * Nothing in it is tappable, so it deliberately does not read as a settings group,
 * and long values (fingerprints, ABI lists) wrap instead of being clipped into a
 * row's trailing slot.
 */
@Composable
internal fun HomeInfoCard(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    TonalCard(
        modifier = modifier,
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 18.dp, bottom = 18.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp), content = content)
    }
}

/**
 * One label-over-value pair.
 *
 * The value is not constrained to a single line: a build fingerprint is long, and
 * truncating the one thing the entry exists to show is worse than a taller card.
 */
@Composable
internal fun ColumnScope.HomeInfoEntry(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        Text(
            text = label,
            style = MeowTheme.typography.title,
            color = MeowTheme.colors.onSurface,
        )
        Text(
            text = value,
            style = MeowTheme.typography.summary,
            color = MeowTheme.colors.onSurfaceVariant,
        )
    }
}
