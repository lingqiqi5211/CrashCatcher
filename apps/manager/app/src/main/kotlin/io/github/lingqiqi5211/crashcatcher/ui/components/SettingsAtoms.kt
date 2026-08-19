package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.meowui.component.MeowPreferenceSection
import io.github.lingqiqi5211.meowui.component.MeowPreferenceSectionScope
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * A titled settings group.
 *
 * Delegates to [MeowPreferenceSection], so the group's container, title style,
 * segmented corners and item spacing come from whichever MeowUI style is active.
 * Rows must be declared through the scope helpers below (or
 * [MeowPreferenceSectionScope.item] for genuinely custom content) — a composable
 * emitted directly in [content] is not part of the group and will not get its
 * container or its share of the segmented corners.
 */
@Composable
internal fun SettingsSection(
    title: String,
    testTag: String,
    content: @Composable MeowPreferenceSectionScope.() -> Unit,
) {
    MeowPreferenceSection(
        modifier = Modifier.testTag(testTag),
        title = title,
        content = content,
    )
}

/** An untitled settings group, for pages that supply their own heading. */
@Composable
internal fun SettingsCard(
    modifier: Modifier = Modifier,
    content: @Composable MeowPreferenceSectionScope.() -> Unit,
) {
    MeowPreferenceSection(
        modifier = modifier,
        content = content,
    )
}

/**
 * A settings row that either navigates/acts on tap, or just displays a value.
 *
 * Tappable rows use MeowUI's action row so the chevron, press feedback and semantics
 * are the style's own. Display-only rows have no MeowUI counterpart (every MeowUI row
 * is interactive), so they are emitted as a custom group item drawn from MeowUI's
 * typography and dimension tokens.
 */
internal fun MeowPreferenceSectionScope.SettingsRow(
    title: String,
    modifier: Modifier = Modifier,
    description: String? = null,
    value: String? = null,
    enabled: Boolean = true,
    navigation: Boolean = false,
    leadingContent: (@Composable () -> Unit)? = null,
    trailingContent: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
) {
    if (onClick != null) {
        MeowActionPreference(
            title = title,
            modifier = modifier,
            summary = description,
            value = value,
            enabled = enabled,
            navigation = navigation,
            leading = leadingContent,
            trailing = trailingContent,
            onClick = onClick,
        )
        return
    }
    item(key = title) {
        SettingsStaticRow(
            title = title,
            modifier = modifier,
            description = description,
            value = value,
            enabled = enabled,
            leading = leadingContent,
            trailing = trailingContent,
        )
    }
}

/** A boolean settings row: the main on/off switch for a feature. */
internal fun MeowPreferenceSectionScope.SettingsSwitchRow(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    MeowSwitchPreference(
        title = title,
        checked = checked,
        onCheckedChange = onCheckedChange,
        modifier = modifier,
        summary = description.takeIf(String::isNotBlank),
        enabled = enabled,
    )
}

/**
 * A settings row for one member of a multi-select set.
 *
 * A switch answers "is this feature on"; a checkbox answers "is this one of the
 * chosen ones", which is what a list of collector sources or crash kinds is.
 */
internal fun MeowPreferenceSectionScope.SettingsCheckboxRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    description: String? = null,
    enabled: Boolean = true,
) {
    MeowCheckboxPreference(
        title = title,
        checked = checked,
        onCheckedChange = onCheckedChange,
        modifier = modifier,
        summary = description?.takeIf(String::isNotBlank),
        enabled = enabled,
    )
}

/**
 * An immediate single-choice settings row.
 *
 * MeowUI renders a Material dropdown or a Miuix window spinner and reports the new
 * value once, so nothing here maintains either popup.
 */
internal fun <T> MeowPreferenceSectionScope.SettingsDropdownRow(
    title: String,
    selected: T,
    options: List<T>,
    optionLabel: (T) -> String,
    onSelected: (T) -> Unit,
    modifier: Modifier = Modifier,
    description: String? = null,
    enabled: Boolean = true,
) {
    MeowPopupPreference(
        title = title,
        value = selected,
        options = options,
        onValueChange = onSelected,
        modifier = modifier,
        summary = description,
        enabled = enabled,
        optionLabel = optionLabel,
    )
}

/** A row that opens another page. */
internal fun MeowPreferenceSectionScope.SettingsNavigationRow(
    title: String,
    description: String? = null,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leading: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
) {
    SettingsRow(
        title = title,
        modifier = modifier,
        description = description,
        enabled = enabled,
        // Opens another page, so the row carries the trailing chevron.
        navigation = true,
        leadingContent = leading,
        onClick = onClick,
    )
}

/**
 * A labelled group of radio options inside one group item.
 *
 * MeowUI has no multi-option row component, so the options are drawn here from MeowUI
 * tokens; the radio control itself still comes from the active style via
 * [CrashCatcherRadioButton]. The whole option row carries the single click and the
 * selection semantics — the control is decorative, so assistive technology reads one
 * target per option rather than two.
 */
internal fun <T> MeowPreferenceSectionScope.SettingsRadioRow(
    label: String,
    options: List<T>,
    selected: T,
    optionLabel: (T) -> String,
    onSelected: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    optionTestTag: (T) -> String? = { null },
) {
    item(key = label) {
        Column(
            modifier = modifier
                .fillMaxWidth()
                .padding(
                    horizontal = MeowTheme.dimensions.itemHorizontalPadding,
                    vertical = MeowTheme.dimensions.itemVerticalPadding,
                ),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = label,
                style = MeowTheme.typography.title,
                color = rowTitleColor(enabled),
            )
            options.forEach { option ->
                val optionSelected = option == selected
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .then(optionTestTag(option)?.let { Modifier.testTag(it) } ?: Modifier)
                        .rowClickable(
                            enabled = enabled,
                            role = Role.RadioButton,
                            onClick = { onSelected(option) },
                        )
                        .semantics { this.selected = optionSelected },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    CrashCatcherRadioButton(
                        selected = optionSelected,
                        onClick = null,
                        enabled = enabled,
                    )
                    Text(
                        text = optionLabel(option),
                        style = MeowTheme.typography.summary,
                        color = rowTitleColor(enabled),
                    )
                }
            }
        }
    }
}

/**
 * A non-interactive settings row.
 *
 * Mirrors the title/summary/value hierarchy and vertical rhythm of MeowUI's own rows
 * so a read-only row sitting between interactive ones does not break the group's
 * alignment.
 */
@Composable
private fun SettingsStaticRow(
    title: String,
    modifier: Modifier = Modifier,
    description: String? = null,
    value: String? = null,
    enabled: Boolean = true,
    leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            // A read-only row still announces itself as disabled; without this a row
            // that only displays a value is indistinguishable from one that is merely
            // unlabelled.
            .then(if (enabled) Modifier else Modifier.semantics { disabled() })
            .padding(
                horizontal = MeowTheme.dimensions.itemHorizontalPadding,
                vertical = MeowTheme.dimensions.itemVerticalPadding,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        leading?.invoke()
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = title,
                style = MeowTheme.typography.title,
                color = rowTitleColor(enabled),
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (!description.isNullOrEmpty()) {
                Text(
                    text = description,
                    style = MeowTheme.typography.summary,
                    color = rowSummaryColor(enabled),
                )
            }
        }
        if (!value.isNullOrBlank()) {
            Text(
                text = value,
                style = MeowTheme.typography.value,
                color = rowSummaryColor(enabled),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        trailing?.invoke()
    }
}

/**
 * Click behaviour for this app's own rows.
 *
 * A disabled row still announces itself as disabled instead of silently losing its
 * semantics, matching how MeowUI's own rows behave.
 */
private fun Modifier.rowClickable(
    enabled: Boolean,
    role: Role,
    onClick: () -> Unit,
): Modifier = if (enabled) {
    clickable(role = role, onClick = onClick)
} else {
    semantics { disabled() }
}

@Composable
internal fun rowTitleColor(enabled: Boolean): Color = if (enabled) {
    MeowTheme.colors.onSurface
} else {
    MeowTheme.colors.onSurfaceVariant.copy(alpha = DisabledTitleAlpha)
}

@Composable
internal fun rowSummaryColor(enabled: Boolean): Color = if (enabled) {
    MeowTheme.colors.onSurfaceVariant
} else {
    MeowTheme.colors.onSurfaceVariant.copy(alpha = DisabledSummaryAlpha)
}

private const val DisabledTitleAlpha = 0.55f
private const val DisabledSummaryAlpha = 0.45f
