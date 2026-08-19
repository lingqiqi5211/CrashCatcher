package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.RowScope
import androidx.compose.material3.Button as MaterialButton
import androidx.compose.material3.Checkbox as MaterialCheckbox
import androidx.compose.material3.CircularProgressIndicator as MaterialCircularProgressIndicator
import androidx.compose.material3.HorizontalDivider as MaterialHorizontalDivider
import androidx.compose.material3.IconButton as MaterialIconButton
import androidx.compose.material3.OutlinedButton as MaterialOutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton as MaterialRadioButton
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Switch as MaterialSwitch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton as MaterialTextButton
import androidx.compose.material3.VerticalDivider as MaterialVerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.ui.theme.isMiuixStyle
import io.github.lingqiqi5211.meowui.theme.MeowTheme
import top.yukonga.miuix.kmp.basic.Button as MiuixButton
import top.yukonga.miuix.kmp.basic.ButtonDefaults as MiuixButtonDefaults
import top.yukonga.miuix.kmp.basic.Checkbox as MiuixCheckbox
import top.yukonga.miuix.kmp.basic.CircularProgressIndicator as MiuixCircularProgressIndicator
import top.yukonga.miuix.kmp.basic.FloatingActionButton as MiuixFloatingActionButton
import top.yukonga.miuix.kmp.basic.HorizontalDivider as MiuixHorizontalDivider
import top.yukonga.miuix.kmp.basic.IconButton as MiuixIconButton
import top.yukonga.miuix.kmp.basic.RadioButton as MiuixRadioButton
import top.yukonga.miuix.kmp.basic.Switch as MiuixSwitch
import top.yukonga.miuix.kmp.basic.TextButton as MiuixTextButton
import top.yukonga.miuix.kmp.basic.TextField as MiuixTextField
import top.yukonga.miuix.kmp.basic.VerticalDivider as MiuixVerticalDivider

/*
 * Style-native primitives MeowUI does not expose.
 *
 * MeowUI's public surface is page-, group- and row-level: it owns scaffolds,
 * preference rows, dialogs, tips, tabs and navigation bars, but not the small
 * controls those are built from. Anything the crash viewer needs outside a MeowUI
 * component lives here, dispatched on `MeowTheme.style` so each style keeps its own
 * control, and coloured from MeowUI tokens rather than either design system's theme
 * object directly.
 *
 * None of these leak a Material or Miuix type through their signatures; the branch
 * is an implementation detail, which is what lets a call site stay a single page.
 *
 * Prefer a real MeowUI component whenever one fits. Reach for these only for
 * surfaces the library has no counterpart for.
 */

@Composable
internal fun CrashCatcherTextButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    if (isMiuixStyle()) {
        MiuixTextButton(text = text, onClick = onClick, modifier = modifier, enabled = enabled)
    } else {
        MaterialTextButton(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
    }
}

/**
 * The filled, primary button — the one call to action on a surface.
 *
 * Material's `Button` is already filled in the primary role, but Miuix's default
 * `ButtonColors` are its *secondary* variant, so the Miuix branch asks for the primary
 * set by name. Left on the defaults, the same call site rendered a call to action in
 * one style and a muted tonal button in the other.
 *
 * The label is a parameter rather than a content slot because the two styles carry the
 * content colour in different composition locals: a Material `Text` written at the call
 * site reads Material's `LocalContentColor` even when it is inside a Miuix button, so
 * it came out in the surface's text colour on top of the primary container.
 */
@Composable
internal fun CrashCatcherButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    if (isMiuixStyle()) {
        MiuixButton(
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
            colors = MiuixButtonDefaults.buttonColorsPrimary(),
        ) {
            Text(
                text = text,
                color = MeowTheme.colors.onPrimary,
                style = MeowTheme.typography.button,
            )
        }
    } else {
        MaterialButton(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
    }
}

/**
 * A secondary button.
 *
 * Miuix has no outlined button: its own hierarchy separates primary from secondary
 * by container colour, so an outline drawn on a Miuix button would be a Material
 * idiom in Miuix clothing. The Miuix branch therefore renders the ordinary button
 * and lets the style express the hierarchy its own way.
 */
@Composable
internal fun CrashCatcherOutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) {
    if (isMiuixStyle()) {
        MiuixButton(onClick = onClick, modifier = modifier, enabled = enabled, content = content)
    } else {
        MaterialOutlinedButton(
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
            content = content,
        )
    }
}

/**
 * An icon-only button.
 *
 * [selected] maps onto Miuix's hold-down state; Material expresses selection through
 * the icon itself, so the flag is only meaningful under Miuix.
 */
@Composable
internal fun CrashCatcherIconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    selected: Boolean = false,
    content: @Composable () -> Unit,
) {
    if (isMiuixStyle()) {
        MiuixIconButton(
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
            holdDownState = selected,
            content = content,
        )
    } else {
        MaterialIconButton(
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
            content = content,
        )
    }
}

@Composable
internal fun CrashCatcherSmallFloatingActionButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    if (isMiuixStyle()) {
        MiuixFloatingActionButton(onClick = onClick, modifier = modifier, content = content)
    } else {
        SmallFloatingActionButton(onClick = onClick, modifier = modifier, content = content)
    }
}

@Composable
internal fun CrashCatcherRadioButton(
    selected: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    if (isMiuixStyle()) {
        MiuixRadioButton(
            selected = selected,
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
        )
    } else {
        MaterialRadioButton(
            selected = selected,
            onClick = onClick,
            modifier = modifier,
            enabled = enabled,
        )
    }
}

@Composable
internal fun CrashCatcherCheckbox(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    if (isMiuixStyle()) {
        MiuixCheckbox(
            state = if (checked) ToggleableState.On else ToggleableState.Off,
            onClick = onCheckedChange?.let { callback -> { callback(!checked) } },
            modifier = modifier,
            enabled = enabled,
        )
    } else {
        MaterialCheckbox(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = modifier,
            enabled = enabled,
        )
    }
}

@Composable
internal fun CrashCatcherSwitch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    if (isMiuixStyle()) {
        MiuixSwitch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = modifier,
            enabled = enabled,
        )
    } else {
        MaterialSwitch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            modifier = modifier,
            enabled = enabled,
        )
    }
}

@Composable
internal fun CrashCatcherCircularProgressIndicator(modifier: Modifier = Modifier) {
    if (isMiuixStyle()) {
        MiuixCircularProgressIndicator(modifier = modifier)
    } else {
        MaterialCircularProgressIndicator(modifier = modifier)
    }
}

@Composable
internal fun CrashCatcherHorizontalDivider(modifier: Modifier = Modifier) {
    if (isMiuixStyle()) {
        MiuixHorizontalDivider(modifier = modifier)
    } else {
        MaterialHorizontalDivider(modifier = modifier)
    }
}

@Composable
internal fun CrashCatcherVerticalDivider(modifier: Modifier = Modifier) {
    if (isMiuixStyle()) {
        MiuixVerticalDivider(modifier = modifier)
    } else {
        MaterialVerticalDivider(modifier = modifier)
    }
}

/**
 * A free-standing text field, for editors that are not preference rows.
 *
 * Settings-shaped text input should use MeowUI's `MeowTextInputPreference` or
 * `MeowTextInputDialog` instead, which own their own commit and validation
 * behaviour. This is for multi-field editor forms — a retention-limit editor, a
 * package filter — where several fields are edited together and committed once.
 *
 * Miuix's own field has no supporting-text slot, so the Miuix branch stacks the
 * message under the field. It is drawn in the error colour because that is the only
 * thing this app uses supporting text for; a purely descriptive hint belongs in the
 * label.
 */
@Composable
internal fun CrashCatcherTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    supportingText: String? = null,
    supportingTextModifier: Modifier = Modifier,
    singleLine: Boolean = false,
    enabled: Boolean = true,
    placeholder: String? = null,
    minLines: Int = 1,
    isError: Boolean = false,
) {
    if (isMiuixStyle()) {
        Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            MiuixTextField(
                value = value,
                onValueChange = onValueChange,
                label = label.ifBlank { placeholder.orEmpty() },
                singleLine = singleLine,
                enabled = enabled,
                minLines = minLines,
            )
            supportingText?.let { message ->
                Text(
                    text = message,
                    modifier = supportingTextModifier,
                    color = MeowTheme.colors.error,
                    style = MeowTheme.typography.summary,
                )
            }
        }
        return
    }
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        singleLine = singleLine,
        enabled = enabled,
        placeholder = placeholder?.let { { Text(it) } },
        minLines = minLines,
        isError = isError,
        label = { Text(label) },
        supportingText = supportingText?.let { message ->
            { Text(message, modifier = supportingTextModifier) }
        },
    )
}
