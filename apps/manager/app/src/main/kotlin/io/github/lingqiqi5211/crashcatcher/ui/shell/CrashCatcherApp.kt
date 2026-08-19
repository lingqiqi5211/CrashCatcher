package io.github.lingqiqi5211.crashcatcher.ui.shell

import android.content.Intent
import android.os.Build
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.core.net.toUri
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppEntry
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordSummary
import io.github.lingqiqi5211.crashcatcher.ui.components.LocalCrashCatcherContentBottomPadding
import io.github.lingqiqi5211.crashcatcher.ui.components.LocalCrashCatcherContentTopPadding
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppDetailActions
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppDetailScreen
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppDetailViewModel
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppsActions
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppsScreen
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppsUiState
import io.github.lingqiqi5211.crashcatcher.ui.apps.AppsViewModel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashTab
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashesActions
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashesScreen
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashesUiState
import io.github.lingqiqi5211.crashcatcher.ui.crashes.CrashesViewModel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.labelRes
import io.github.lingqiqi5211.crashcatcher.ui.crashes.GroupDetailActions
import io.github.lingqiqi5211.crashcatcher.ui.crashes.GroupDetailEvent
import io.github.lingqiqi5211.crashcatcher.ui.crashes.GroupDetailScreen
import io.github.lingqiqi5211.crashcatcher.ui.crashes.GroupDetailViewModel
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailActions
import io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailEvent
import io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailScreen
import io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailViewModel
import io.github.lingqiqi5211.crashcatcher.ui.util.copyLog
import io.github.lingqiqi5211.crashcatcher.ui.util.errorTitle
import io.github.lingqiqi5211.crashcatcher.ui.util.shareLog
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.crashcatcher.ui.home.HomeScreen
import io.github.lingqiqi5211.crashcatcher.ui.home.HomeViewModel
import io.github.lingqiqi5211.crashcatcher.data.device.readDeviceInfo
import io.github.lingqiqi5211.crashcatcher.ui.settings.AboutPage
import io.github.lingqiqi5211.crashcatcher.ui.settings.AppearanceScreen
import io.github.lingqiqi5211.crashcatcher.ui.settings.CaptureSettingsPage
import io.github.lingqiqi5211.crashcatcher.ui.settings.DialogSettingsPage
import io.github.lingqiqi5211.crashcatcher.ui.settings.NotifySettingsPage
import io.github.lingqiqi5211.crashcatcher.ui.settings.SettingsActions
import io.github.lingqiqi5211.crashcatcher.ui.settings.SettingsScreenContent
import io.github.lingqiqi5211.crashcatcher.ui.settings.SettingsUiState
import io.github.lingqiqi5211.crashcatcher.ui.settings.SettingsViewModel
import io.github.lingqiqi5211.crashcatcher.ui.settings.StorageSettingsPage
import kotlin.math.abs
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.launch
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherPredictiveBack
import io.github.lingqiqi5211.crashcatcher.ui.components.crashCatcherContentScaffoldPadding
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.component.MeowMenuItem
import io.github.lingqiqi5211.meowui.component.MeowNavHost
import io.github.lingqiqi5211.meowui.component.MeowPreferenceScreen
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import io.github.lingqiqi5211.meowui.component.rememberMeowSnackbarState

/**
 * The app shell.
 *
 * Two axes, borrowed from the reference implementation:
 *
 * - Root destinations are pages of a [HorizontalPager], not a navigation graph. All
 *   four stay composed, so switching tabs keeps scroll positions and in-flight loads
 *   instead of tearing each screen down and starting over.
 * - Anything deeper is a caller-owned back stack fed to [MeowNavHost]. Each pushed
 *   page brings its own scaffold, so the whole surface — top bar included — slides.
 */
@Composable
internal fun CrashCatcherApp(
    container: AppContainer,
    modifier: Modifier = Modifier,
    pendingRecord: RecordId? = null,
    onPendingRecordOpened: () -> Unit = {},
) {
    val factory = remember(container) { AppViewModelFactory(container) }
    val scope = rememberCoroutineScope()

    var backStack by rememberSaveable(saver = PageStackSaver) {
        mutableStateOf(listOf<Page>(Page.Shell))
    }

    // An alert or a notification asked for one record. Replacing the stack rather than
    // pushing onto it: a second notification while a record is already open should show the
    // new crash, not bury it under the old one, and back from here goes to the shell —
    // where the user would have been had they opened the app themselves.
    LaunchedEffect(pendingRecord) {
        val record = pendingRecord ?: return@LaunchedEffect
        backStack = listOf(Page.Shell, Page.RecordDetail(record))
        onPendingRecordOpened()
    }
    // Guarded, not `dropLast(1)`: the root page is the app, and popping it leaves
    // MeowNavHost with an empty stack, which it rejects — `IllegalArgumentException:
    // backStack must not be empty`, which this app recorded against itself. Deletion used
    // to unwind more pages than were pushed (see the view models' single-shot close), and a
    // close that arrives one time too many must not be able to take the app down.
    val pop = { if (backStack.size > 1) backStack = backStack.dropLast(1) }

    MeowNavHost(
        backStack = backStack,
        modifier = modifier.fillMaxSize(),
        onBack = pop,
        predictiveBackEnabled = LocalCrashCatcherPredictiveBack.current,
    ) { page ->
        when (page) {
            is Page.Shell -> RootShell(
                factory = factory,
                onPush = { backStack = backStack + it },
            )

            is Page.GroupDetail -> GroupDetailPage(
                factory = factory,
                groupId = page.groupId,
                onBack = pop,
                onOpenRecord = { record -> backStack = backStack + Page.RecordDetail(record.id) },
            )

            is Page.RecordDetail -> RecordDetailPage(
                factory = factory,
                id = page.id,
                onBack = pop,
            )

            is Page.AppDetail -> AppDetailPage(
                factory = factory,
                packageName = page.packageName,
                onBack = pop,
                onOpenGroup = { group -> backStack = backStack + Page.GroupDetail(group.groupId) },
            )

            is Page.Appearance -> {
                val settings by container.appearance.appearance.collectAsStateWithLifecycle()
                AppearanceScreen(
                    settings = settings,
                    onSettingsChange = { updated ->
                        scope.launch { container.appearance.update(updated) }
                    },
                    onBack = pop,
                )
            }

            is Page.CaptureSettings -> SettingsSubPage(factory) { state, actions ->
                CaptureSettingsPage(state = state, actions = actions, onBack = pop)
            }

            is Page.NotifySettings -> SettingsSubPage(factory) { state, actions ->
                NotifySettingsPage(state = state, actions = actions, onBack = pop)
            }

            is Page.DialogSettings -> SettingsSubPage(factory) { state, actions ->
                DialogSettingsPage(state = state, actions = actions, onBack = pop)
            }

            is Page.StorageSettings -> SettingsSubPage(factory) { state, actions ->
                StorageSettingsPage(state = state, actions = actions, onBack = pop)
            }

            is Page.About -> {
                val context = LocalContext.current
                SettingsSubPage(factory) { state, _ ->
                    AboutPage(
                        state = state,
                        deviceInfo = remember { readDeviceInfo() },
                        // Wrapped: a device with no browser, or a link the system cannot
                        // resolve, must not take the app down over an about-page tap.
                        onOpenUrl = { url ->
                            runCatching {
                                context.startActivity(
                                    Intent(Intent.ACTION_VIEW, url.toUri()),
                                )
                            }
                        },
                        onBack = pop,
                    )
                }
            }
        }
    }
}

/**
 * Hosts a settings sub-page against its own `SettingsViewModel`.
 *
 * Each pushed page gets a fresh instance rather than sharing the tab's. They read the
 * same repositories — whose flows are the actual source of truth — so nothing is stale,
 * and a page that owns its view model can be opened from anywhere without the shell
 * having to keep the tab's instance alive for it.
 */
@Composable
private fun SettingsSubPage(
    factory: AppViewModelFactory,
    content: @Composable (SettingsUiState, SettingsActions) -> Unit,
) {
    val viewModel: SettingsViewModel = viewModel(factory = factory)
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    content(state, viewModel.settingsActions())
}

@Composable
private fun RootShell(
    factory: AppViewModelFactory,
    onPush: (Page) -> Unit,
) {
    val destinations = Destination.entries
    // The pager is the single source of truth for which tab is showing; the bar
    // follows it. Holding a second copy of the selection would let the two disagree
    // mid-swipe.
    var selected by rememberSaveable { mutableStateOf(0) }
    val pagerState = rememberPagerState(
        initialPage = selected,
        pageCount = { destinations.size },
    )
    val stateHolder = rememberSaveableStateHolder()
    val layoutDirection = LocalLayoutDirection.current
    val navigationScope = rememberCoroutineScope()
    var navigationJob by remember { mutableStateOf<Job?>(null) }
    // True only while a tap-driven jump is animating. A swipe and a jump want opposite
    // readings of the pager: see the destination sync below.
    var jumping by remember { mutableStateOf(false) }

    val navigateTo: (Destination) -> Unit = { destination ->
        selected = destination.ordinal
        navigationJob?.cancel()
        jumping = true
        val distance = abs(destination.ordinal - pagerState.currentPage).coerceAtLeast(2)
        navigationJob = navigationScope.launch {
            // Deliberately not animateScrollToPage. For a jump of two or more tabs it
            // teleports to one page short of the target and animates only that last hop,
            // and `currentPage` flips to each page swept through on the way. The sync
            // below then wrote that intermediate page back into `selected`, which
            // restarted this effect against the page the pager had just landed on — so a
            // two-tab tap stopped one short and stuck there.
            //
            // Walking the same distance in pixels keeps every intermediate page on screen
            // for the whole tween and never retargets. All pages stay composed either way
            // (beyondViewportPageCount).
            val pageSize = pagerState.layoutInfo.pageSize + pagerState.layoutInfo.pageSpacing
            if (pageSize <= 0) {
                // Nothing has been laid out yet, so there is no distance to travel.
                pagerState.scrollToPage(destination.ordinal)
            } else {
                val from = pagerState.currentPage + pagerState.currentPageOffsetFraction
                pagerState.animateScrollBy(
                    value = (destination.ordinal - from) * pageSize,
                    animationSpec = tween(
                        durationMillis = 100 * distance + 100,
                        easing = EaseInOut,
                    ),
                )
                // A pixel walk can stop a fraction short of the boundary, and unlike a
                // gesture there is no fling to snap it; settle it exactly on the page.
                pagerState.scrollToPage(destination.ordinal)
            }
            jumping = false
        }
    }

    // ViewModels are hoisted here rather than created inside each page so the shell
    // can build the top bar's filter menu from the same state the page renders. A page
    // that owned its ViewModel would have to push its filter state back up through a
    // callback for the bar to show which option is ticked.
    val homeViewModel: HomeViewModel = viewModel(factory = factory)
    val crashesViewModel: CrashesViewModel = viewModel(factory = factory)
    val appsViewModel: AppsViewModel = viewModel(factory = factory)
    val settingsViewModel: SettingsViewModel = viewModel(factory = factory)

    val homeState by homeViewModel.uiState.collectAsStateWithLifecycle()
    val crashesState by crashesViewModel.uiState.collectAsStateWithLifecycle()
    val appsState by appsViewModel.uiState.collectAsStateWithLifecycle()
    val settingsState by settingsViewModel.uiState.collectAsStateWithLifecycle()

    // The bar follows whichever page reading matches how the pager is being moved.
    //
    // A swipe crosses one page at a time and `currentPage` flips at the halfway mark,
    // which is when the bar should light up the page being dragged in. Waiting for the
    // pager to settle would make the indicator lag a whole gesture behind.
    //
    // A tap-driven jump is the opposite: `currentPage` reports every page swept
    // through, so following it would drag the indicator across the intermediates. Those
    // jumps read `settledPage` instead — and only while the jump is in flight, so a
    // settle that lands after it (a swipe back mid-animation, a tap superseding
    // another) is never dropped.
    LaunchedEffect(pagerState) {
        snapshotFlow { if (jumping) pagerState.settledPage else pagerState.currentPage }
            .distinctUntilChanged()
            .collect { page -> selected = page }
    }

    val current = destinations[selected.coerceIn(destinations.indices)]

    // Reconnect is reachable from two places — the settings row and the disconnected
    // banner on the overview — and neither can report the outcome itself: the row looks
    // identical afterwards, and the banner disappears on success and is unchanged on
    // failure, so a failed attempt was indistinguishable from a press that did nothing.
    // The host is the shell's, so the message survives the tab the press came from.
    val snackbarState = rememberMeowSnackbarState()
    val reconnected = stringResource(R.string.reconnect_succeeded)
    val reconnectFailed = stringResource(R.string.reconnect_failed)
    LaunchedEffect(homeViewModel, settingsViewModel) {
        merge(homeViewModel.reconnectOutcomes, settingsViewModel.reconnectOutcomes)
            .collect { outcome ->
                snackbarState.show(
                    if (outcome.connected) reconnected else reconnectFailed,
                )
            }
    }

    RootScaffold(
        destination = current,
        snackbarState = snackbarState,
        actionItems = when (current) {
            Destination.Crashes -> listOf(crashesFilterMenu(crashesState, crashesViewModel))
            Destination.Apps -> listOf(appsFilterMenu(appsState, appsViewModel))
            Destination.Home, Destination.Settings -> emptyList()
        },
        onDestinationSelected = navigateTo,
    ) { contentPadding ->
        // Top and bottom are *published*, not applied: a destination's own scroll
        // container takes them as content padding, so items pass under the frosted
        // bars instead of stopping at an opaque band. Only the sides are layout
        // padding. Applying the top here as well is what left a screen-high gap above
        // the first card.
        CompositionLocalProvider(
            LocalCrashCatcherContentTopPadding provides contentPadding.calculateTopPadding(),
            LocalCrashCatcherContentBottomPadding provides contentPadding.calculateBottomPadding(),
        ) {
            HorizontalPager(
                state = pagerState,
                modifier = Modifier
                    .fillMaxSize()
                    .testTag("crashcatcher.shell.pager"),
                // Keep every page composed: a tab switch should not discard a list the
                // user scrolled or a request already in flight.
                beyondViewportPageCount = destinations.lastIndex,
            ) { index ->
                val destination = destinations[index]
                stateHolder.SaveableStateProvider(destination.route) {
                    Box(
                        Modifier.padding(
                            start = contentPadding.calculateStartPadding(layoutDirection),
                            end = contentPadding.calculateEndPadding(layoutDirection),
                        ),
                    ) {
                        when (destination) {
                            Destination.Home -> HomeScreen(
                                state = homeState,
                                onRefresh = homeViewModel::refresh,
                                onReconnect = homeViewModel::reconnect,
                            )

                            Destination.Crashes -> CrashesScreen(
                                state = crashesState,
                                actions = crashesViewModel.actions { group ->
                                    onPush(Page.GroupDetail(group.groupId))
                                },
                            )

                            Destination.Apps -> AppsScreen(
                                state = appsState,
                                actions = appsViewModel.actions { app ->
                                    onPush(Page.AppDetail(app.packageName))
                                },
                            )

                            Destination.Settings -> SettingsTab(
                                state = settingsState,
                                viewModel = settingsViewModel,
                                onPush = onPush,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SettingsTab(
    state: SettingsUiState,
    viewModel: SettingsViewModel,
    onPush: (Page) -> Unit,
) {
    // `MeowPreferenceScreen`, not `MeowPreferencePage`: the page variant brings its
    // own top bar, which inside the shell's scaffold would stack a second one.
    MeowPreferenceScreen(
        modifier = Modifier.testTag("crashcatcher.settings.scroll"),
        scaffoldPadding = crashCatcherContentScaffoldPadding,
    ) {
        SettingsScreenContent(state = state, actions = viewModel.settingsActions(onPush))
    }
}

/**
 * The settings callback bag.
 *
 * Built in one place because the tab and five sub-pages all need it, and a bag assembled
 * per call site is how one of them ends up wired to the wrong method. [onPush] is absent
 * on a sub-page, which has nowhere further to go.
 */
private fun SettingsViewModel.settingsActions(
    onPush: ((Page) -> Unit)? = null,
) = SettingsActions(
    onCaptureJavaChange = ::onCaptureJavaChange,
    onCaptureAnrChange = ::onCaptureAnrChange,
    onCaptureNativeChange = ::onCaptureNativeChange,
    onCaptureSelfHandledChange = ::onCaptureSelfHandledChange,
    onNotifyModeChange = ::onNotifyModeChange,
    onOnlyForegroundChange = ::onOnlyForegroundChange,
    onOnlyMainProcessChange = ::onOnlyMainProcessChange,
    onIncludeSystemAppsChange = ::onIncludeSystemAppsChange,
    onDialogTakeoverChange = ::onDialogTakeoverChange,
    onRetentionDaysChange = ::onRetentionDaysChange,
    onMaxRecordsTotalChange = ::onMaxRecordsTotalChange,
    onDeleteAll = ::onDeleteAll,
    onReconnect = ::onReconnect,
    onOpenAppearance = { onPush?.invoke(Page.Appearance) },
    onOpenAbout = { onPush?.invoke(Page.About) },
    onOpenCapture = { onPush?.invoke(Page.CaptureSettings) },
    onOpenNotify = { onPush?.invoke(Page.NotifySettings) },
    onOpenDialog = { onPush?.invoke(Page.DialogSettings) },
    onOpenStorage = { onPush?.invoke(Page.StorageSettings) },
)

private fun CrashesViewModel.actions(onOpenGroup: (GroupSummary) -> Unit) = CrashesActions(
    onTabSelected = ::onTabSelected,
    onQueryChange = ::onQueryChange,
    onSearchExpandedChange = ::onSearchExpandedChange,
    onSearchSubmit = ::onSearchSubmit,
    onIncludeSystemAppsChange = ::onIncludeSystemAppsChange,
    onOnlySelfHandledChange = ::onOnlySelfHandledChange,
    onRefresh = { refresh() },
    onPullToRefresh = ::onPullToRefresh,
    onLoadMore = ::loadMore,
    onOpenGroup = onOpenGroup,
)

private fun AppsViewModel.actions(onOpenApp: (AppEntry) -> Unit) = AppsActions(
    onQueryChange = ::onQueryChange,
    onSearchExpandedChange = ::onSearchExpandedChange,
    onIncludeSystemAppsChange = ::onIncludeSystemAppsChange,
    onRefresh = ::refresh,
    onPullToRefresh = ::onPullToRefresh,
    onOpenApp = onOpenApp,
)

/**
 * The crash list's filter menu.
 *
 * Single-select for the kind, checkable toggles for scope, with
 * `collapseOnSelection = false` so trying two filters does not mean reopening the
 * menu each time.
 */
@Composable
private fun crashesFilterMenu(
    state: CrashesUiState,
    viewModel: CrashesViewModel,
): MeowTopBarAction = MeowTopBarAction.Menu(
    icon = MeowIcons.Filter,
    contentDescription = stringResource(R.string.crashes_filter),
    modifier = Modifier.testTag("crashcatcher.crashes.filter"),
    collapseOnSelection = false,
    groups = listOf(
        CrashTab.entries.map { tab ->
            MeowMenuItem(
                text = stringResource(tab.labelRes),
                selected = state.tab == tab,
                onClick = { viewModel.onTabSelected(tab) },
            )
        },
        listOf(
            MeowMenuItem(
                text = stringResource(R.string.settings_include_system_apps),
                selected = state.includeSystemApps,
                onClick = { viewModel.onIncludeSystemAppsChange(!state.includeSystemApps) },
            ),
            MeowMenuItem(
                text = stringResource(R.string.crashes_only_self_handled),
                selected = state.onlySelfHandled,
                onClick = { viewModel.onOnlySelfHandledChange(!state.onlySelfHandled) },
            ),
        ),
    ),
)

@Composable
private fun appsFilterMenu(
    state: AppsUiState,
    viewModel: AppsViewModel,
): MeowTopBarAction = MeowTopBarAction.Menu(
    icon = MeowIcons.Filter,
    contentDescription = stringResource(R.string.crashes_filter),
    modifier = Modifier.testTag("crashcatcher.apps.filter"),
    collapseOnSelection = false,
    items = listOf(
        MeowMenuItem(
            text = stringResource(R.string.settings_include_system_apps),
            selected = state.includeSystemApps,
            onClick = { viewModel.onIncludeSystemAppsChange(!state.includeSystemApps) },
        ),
    ),
)

@Composable
private fun RecordDetailPage(
    factory: AppViewModelFactory,
    id: RecordId,
    onBack: () -> Unit,
) {
    val viewModel: RecordDetailViewModel = viewModel(factory = factory)
    LaunchedEffect(id) { viewModel.load(id) }
    val state by viewModel.uiState.collectAsStateWithLifecycle()

    val context = LocalContext.current
    val snackbarState = rememberMeowSnackbarState()
    val copied = stringResource(R.string.detail_copied)
    val noPayload = stringResource(R.string.detail_copy_empty)
    val copyFailed = stringResource(R.string.detail_copy_failed)
    val shareFailed = stringResource(R.string.detail_share_failed)

    // Held as an error rather than a string: turning one into something readable is a
    // composable read (see `errorTitle`), which the collector below cannot do.
    var failure by remember { mutableStateOf<DomainError?>(null) }
    val failureMessage = failure?.let { errorTitle(it) }
    LaunchedEffect(failureMessage) {
        val message = failureMessage ?: return@LaunchedEffect
        snackbarState.show(message)
        failure = null
    }

    LaunchedEffect(viewModel) {
        viewModel.uiEvents.collect { event ->
            when (event) {
                is RecordDetailEvent.Copied -> when {
                    event.text.isBlank() -> snackbarState.show(noPayload)
                    !copyLog(context, id.value, event.text) -> snackbarState.show(copyFailed)
                    // Android 13 shows its own confirmation for a copy the foreground app
                    // makes, and two of them for one tap reads as a stutter.
                    Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ->
                        snackbarState.show(copied)
                }

                is RecordDetailEvent.Exported -> {
                    val group = viewModel.uiState.value.detail?.group
                    val name = group?.summaryClass?.let(::shortTypeName)
                        ?: group?.packageName
                        ?: context.getString(R.string.app_name)
                    val subject = context.getString(R.string.detail_share_subject, name)
                    when {
                        event.text.isBlank() -> snackbarState.show(noPayload)
                        !shareLog(context, subject, event.text) ->
                            snackbarState.show(shareFailed)
                    }
                }

                // The record is gone, so its page goes with it — whether this screen
                // deleted it or the storage page cleared everything underneath it.
                RecordDetailEvent.Deleted -> onBack()

                is RecordDetailEvent.Failed -> failure = event.error
            }
        }
    }

    RecordDetailScreen(
        state = state,
        actions = RecordDetailActions(
            onBack = onBack,
            // Both take the field choices the screen collected before asking.
            onCopy = viewModel::onCopy,
            onShare = viewModel::onShare,
            onDelete = viewModel::onDelete,
            onExpandFold = viewModel::onExpandFold,
            onToggleWrap = viewModel::onToggleWrap,
        ),
        snackbarState = snackbarState,
    )
}

@Composable
private fun GroupDetailPage(
    factory: AppViewModelFactory,
    groupId: String,
    onBack: () -> Unit,
    onOpenRecord: (RecordSummary) -> Unit,
) {
    val viewModel: GroupDetailViewModel = viewModel(factory = factory)
    LaunchedEffect(groupId) { viewModel.load(groupId) }
    val state by viewModel.uiState.collectAsStateWithLifecycle()

    // Deleting the group takes its own screen away with it.
    LaunchedEffect(viewModel) {
        viewModel.uiEvents.collect { event ->
            if (event is GroupDetailEvent.Deleted) onBack()
        }
    }

    GroupDetailScreen(
        state = state,
        actions = GroupDetailActions(
            onBack = onBack,
            onDelete = viewModel::delete,
            onLoadMore = viewModel::loadMore,
            onOpenRecord = onOpenRecord,
        ),
    )
}

@Composable
private fun AppDetailPage(
    factory: AppViewModelFactory,
    packageName: String,
    onBack: () -> Unit,
    onOpenGroup: (GroupSummary) -> Unit,
) {
    val viewModel: AppDetailViewModel = viewModel(factory = factory)
    LaunchedEffect(packageName) { viewModel.load(packageName) }
    val state by viewModel.uiState.collectAsStateWithLifecycle()

    AppDetailScreen(
        state = state,
        actions = AppDetailActions(
            onBack = onBack,
            onNotifyChoiceChange = viewModel::onNotifyChoiceChange,
            onIgnoreChange = viewModel::onIgnoreChange,
            onMuteChange = viewModel::onMuteChange,
            onReopen = viewModel::onReopen,
            onOpenGroup = onOpenGroup,
        ),
    )
}

/**
 * Persists the back stack across configuration changes.
 *
 * Pages carry typed arguments, so they are saved as route strings and rebuilt
 * rather than made `Parcelable`: the stack is short, the mapping is explicit, and an
 * unrecognised route is dropped instead of crashing a restore.
 */
private val PageStackSaver = listSaver<MutableState<List<Page>>, String>(
    save = { state -> state.value.map { it.toRoute() } },
    restore = { routes ->
        mutableStateOf(routes.mapNotNull { it.toPage() }.ifEmpty { listOf(Page.Shell) })
    },
)

private fun Page.toRoute(): String = when (this) {
    is Page.Shell -> "shell"
    is Page.GroupDetail -> "group/$groupId"
    is Page.RecordDetail -> "record/${id.value}"
    is Page.AppDetail -> "app/$packageName"
    is Page.Appearance -> "appearance"
    is Page.CaptureSettings -> "settings/capture"
    is Page.NotifySettings -> "settings/notify"
    is Page.DialogSettings -> "settings/dialog"
    is Page.StorageSettings -> "settings/storage"
    is Page.About -> "about"
}

private fun String.toPage(): Page? = when {
    this == "shell" -> Page.Shell
    this == "appearance" -> Page.Appearance
    this == "settings/capture" -> Page.CaptureSettings
    this == "settings/notify" -> Page.NotifySettings
    this == "settings/dialog" -> Page.DialogSettings
    this == "settings/storage" -> Page.StorageSettings
    this == "about" -> Page.About
    startsWith("group/") -> Page.GroupDetail(removePrefix("group/"))
    startsWith("record/") -> Page.RecordDetail(RecordId(removePrefix("record/")))
    startsWith("app/") -> Page.AppDetail(removePrefix("app/"))
    else -> null
}
