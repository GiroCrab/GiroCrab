package de.girocrab.app

import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.scale
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.background

import android.app.LocaleManager
import android.os.Build
import android.os.Bundle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.sp
import android.os.LocaleList
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.filled.Delete
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import de.girocrab.app.model.Booking
import de.girocrab.app.ui.screens.AccountsAndWealthScreen
import de.girocrab.app.ui.screens.AccountsBadges
import de.girocrab.app.ui.screens.PortfolioAndAssetsScreen
import de.girocrab.app.ui.screens.AddBankDialog
import de.girocrab.app.ui.screens.AnalyticsScreen
import de.girocrab.app.ui.screens.BookingDetailSheet
import de.girocrab.app.ui.screens.ImportPasswordDialog
import de.girocrab.app.ui.screens.BookingStreamScreen
import de.girocrab.app.ui.screens.CategoryManagerDialog
import de.girocrab.app.ui.screens.PaymentsScreen
import de.girocrab.app.ui.screens.RuleEditorDialog
import de.girocrab.app.ui.screens.SettingsScreen
import de.girocrab.app.ui.screens.TagManagerDialog
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Analytics
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Payments
import androidx.compose.material.icons.filled.Savings
import androidx.compose.material.icons.filled.ReceiptLong
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import de.girocrab.app.model.AppViewModel
import de.girocrab.app.model.VaultState
import de.girocrab.app.security.AndroidCipherPrompt
import de.girocrab.app.security.ImportResult
import de.girocrab.app.security.NativeVaultOpener
import de.girocrab.app.security.OwnVault
import de.girocrab.app.security.UnlockResult
import de.girocrab.app.security.VaultKeyStore
import de.girocrab.app.security.VaultImporter
import de.girocrab.app.security.VaultUnlocker
import de.girocrab.app.ui.screens.BiometricLockScreen
import de.girocrab.app.ui.screens.DemoBanner
import de.girocrab.app.ui.screens.EmptyVaultCard
import de.girocrab.app.ui.theme.GiroCrabTheme
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.distinctUntilChanged

enum class MainTab(val label: String, val icon: ImageVector) {
    Bookings("Umsätze", Icons.Default.ReceiptLong),
    Accounts("Konten", Icons.Default.AccountBalanceWallet),
    Analytics("Analysen", Icons.Default.Analytics),
    Payments("Zahlungen", Icons.Default.Payments),
    Portfolio("Depot", Icons.Default.Savings),
    More("Mehr", Icons.Default.MoreHoriz),
}

/** Prominent segmented toggle at the top of Konten: accounts vs. Depot / Vermögen. */
@androidx.compose.runtime.Composable
private fun AccountsWealthToggle(portfolio: Boolean, onSelect: (portfolio: Boolean) -> Unit) {
    val labels = listOf("Konten", "Depot / Vermögen")
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        labels.forEachIndexed { i, label ->
            SegmentedButton(
                selected = (i == 1) == portfolio,
                onClick = { onSelect(i == 1) },
                shape = SegmentedButtonDefaults.itemShape(index = i, count = labels.size),
            ) { Text(label, maxLines = 1) }
        }
    }
}

/** FragmentActivity (not ComponentActivity) because BiometricPrompt requires it. */
class MainActivity : FragmentActivity() {
    private val vm: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            GiroCrabTheme {
              Box(Modifier.fillMaxSize()) {
                GiroCrabRoot(
                    vm,
                    setSecure = ::setSecure,
                    onLanguage = ::applyLanguage,
                    onExportBackup = { exportLauncher.launch("girocrab-backup.vault") },
                    onRestoreBackup = { restoreLauncher.launch(arrayOf("*/*")) },
                )
                GiroCrabIntro(skip = intent?.getBooleanExtra("skip_intro", false) == true)
              }
            }
        }
    }

    // Backup = the vault file copied as-is, so it stays encrypted. Assumes the core keeps it at filesDir/girocrab.vault.
    private val vaultFile get() = File(filesDir, "girocrab.vault")

    private val exportLauncher = registerForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        if (uri != null && vaultFile.exists()) runCatching {
            contentResolver.openOutputStream(uri)?.use { out -> vaultFile.inputStream().use { it.copyTo(out) } }
        }
    }

    // Restore replaces the vault file, then locks so the next unlock re-reads it.
    private val restoreLauncher = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) runCatching {
            contentResolver.openInputStream(uri)?.use { input -> vaultFile.outputStream().use { input.copyTo(it) } }
            vm.lock()
        }
    }

    private fun applyLanguage(code: String) {
        if (Build.VERSION.SDK_INT >= 33) {
            getSystemService(LocaleManager::class.java)?.applicationLocales = LocaleList.forLanguageTags(code)
        }
    }

    /** FLAG_SECURE hides the app from screenshots and the recents thumbnail. */
    private fun setSecure(secure: Boolean) {
        if (secure) window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        else window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    override fun onStop() {
        super.onStop()
        // #1428 (2.5): leaving the foreground locks the window, not the vault. The snapshot stays
        // loaded, so coming back costs one prompt instead of a full re-read of the vault.
        if (!isChangingConfigurations) vm.lockForBackground()
    }
}

@Composable
fun GiroCrabRoot(
    vm: AppViewModel,
    setSecure: (Boolean) -> Unit,
    onLanguage: (String) -> Unit = {},
    onExportBackup: () -> Unit = {},
    onRestoreBackup: () -> Unit = {},
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val visible by vm.visibleBookings.collectAsStateWithLifecycle()
    var detail by remember { mutableStateOf<Booking?>(null) }
    var dialog by remember { mutableStateOf<ManagementDialog?>(null) }
    var tab by rememberSaveable { mutableStateOf(MainTab.Bookings) }
    var lockPreview by rememberSaveable { mutableStateOf(false) }

    val locked =
        (state.vaultState == VaultState.Locked && !state.loading) || state.backgroundLocked || lockPreview
    LaunchedEffect(state.privacyMode, locked) { setSecure(state.privacyMode) }

    // The unlocker owns the Keystore cipher: without an authenticated prompt nothing is unwrapped.
    val context = LocalContext.current
    val unlocker = remember(context) {
        (context as? FragmentActivity)?.let { activity ->
            VaultUnlocker(
                keyStore = VaultKeyStore(VaultKeyStore.defaultFile(activity.filesDir)),
                opener = NativeVaultOpener(),
                prompt = AndroidCipherPrompt(activity),
                filesDir = activity.filesDir,
            )
        }
    }
    var unlockMessage by remember { mutableStateOf<String?>(null) }
    // Only offered after a vault really opened: a labelled way out, not a silent demo fallback.
    var demoEscape by remember { mutableStateOf(false) }

    // Import path from #1428 (2.6): Android never creates a vault, so this is the only way in.
    val importer = remember(context) {
        (context as? FragmentActivity)?.let { activity ->
            VaultImporter(
                keyStore = VaultKeyStore(VaultKeyStore.defaultFile(activity.filesDir)),
                opener = NativeVaultOpener(),
                prompt = AndroidCipherPrompt(activity),
                filesDir = activity.filesDir,
            )
        }
    }
    var pendingBackup by remember { mutableStateOf<File?>(null) }
    val scope = rememberCoroutineScope()
    val pickBackup = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        // A slow provider must not block the lock screen: copy off the main thread, publish on it.
        scope.launch {
            // One file per selection: a second choice must not truncate the file the dialog reads.
            // A cache that cannot be written (full, permission gone) must not end the coroutine
            // with an uncaught IOException — the lock screen has to say something instead.
            val target = runCatching { File.createTempFile("import-", ".gcbackup", context.cacheDir) }
                .getOrElse {
                    unlockMessage = "Im Zwischenspeicher ließ sich keine Datei anlegen."
                    return@launch
                }
            val copied = withContext(Dispatchers.IO) {
                runCatching {
                    context.contentResolver.openInputStream(uri)?.use { input ->
                        target.outputStream().use { output -> input.copyTo(output) }
                    } ?: error("no stream")
                }.isSuccess
            }
            if (copied) {
                pendingBackup?.delete()
                pendingBackup = target
            } else {
                target.delete()
                unlockMessage = "Die gewählte Datei ließ sich nicht lesen."
            }
        }
    }

    if (state.loading && state.vaultState == VaultState.Locked) {
        Box(Modifier.fillMaxSize())
        return
    }

    if (locked) {
        BiometricLockScreen(
            previewMode = lockPreview,
            message = unlockMessage,
            onUnlockRequest = {
                val request = unlocker
                when {
                    request == null -> unlockMessage = "Entsperren ist auf diesem Gerät nicht verfügbar."
                    // #1428 (2.5): the vault never closed, so this is one prompt and no second read.
                    state.backgroundLocked -> {
                        unlockMessage = null
                        demoEscape = false
                        request.releaseSoftLock { result ->
                            if (result is UnlockResult.SoftReleased) {
                                unlockMessage = null
                                vm.clearBackgroundLock()
                            } else {
                                unlockMessage = describe(result)
                            }
                        }
                    }
                    else -> {
                        unlockMessage = null
                        demoEscape = false
                        request.unlock { result ->
                            // Reading the data is the mapper's job (#1429, 3.1); the handle is
                            // closed as soon as the copy exists, so the ViewModel holds no handle.
                            unlockMessage = if (result is UnlockResult.Opened) {
                                adoptOwnSnapshot(vm, result.vault, "Tresor geöffnet, Inhalt aber nicht lesbar")
                            } else {
                                describe(result)
                            }
                            demoEscape = false
                        }
                    }
                }
            },
            onBackToDemo = { if (lockPreview) lockPreview = false else vm.showDemo() },
            demoEscape = demoEscape,
            onImportRequest = if (lockPreview || state.backgroundLocked) null else {{
                pickBackup.launch(arrayOf("*/*"))
            }},
        )
        pendingBackup?.let { file ->
            ImportPasswordDialog(
                fileName = file.name,
                onSubmit = { password ->
                    pendingBackup = null
                    val request = importer
                    if (request == null) {
                        unlockMessage = "Import ist auf diesem Gerät nicht verfügbar."
                    } else {
                        unlockMessage = "Sicherung wird eingespielt …"
                        request.import(file.path, password) { result ->
                            try {
                                unlockMessage = if (result is ImportResult.Imported) {
                                    adoptOwnSnapshot(
                                        vm, result.vault, "Sicherung eingespielt, Inhalt aber nicht lesbar",
                                    )
                                } else {
                                    describeImport(result)
                                }
                            } finally {
                                demoEscape = result is ImportResult.Imported
                                file.delete()
                            }
                        }
                    }
                },
                onDismiss = {
                    pendingBackup?.delete()
                    pendingBackup = null
                },
            )
        }
        return
    }

    Scaffold(
        bottomBar = {
            NavigationBar(
                containerColor = MaterialTheme.colorScheme.surface,
                tonalElevation = 0.dp,
            ) {
                MainTab.entries.filter { it != MainTab.Portfolio }.forEach { t ->
                    NavigationBarItem(
                        selected = tab == t || (t == MainTab.Accounts && tab == MainTab.Portfolio),
                        onClick = { tab = t },
                        icon = { Icon(t.icon, contentDescription = t.label) },
                        label = { Text(t.label, fontSize = 11.sp, maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis) },
                    )
                }
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (state.demoActive) {
                DemoBanner(
                    vaultMayHoldRealData = state.vaultMayHoldRealData,
                    onCreateOwnAccount = { vm.showOwnVault(); tab = MainTab.Accounts },
                    onBackToOwnVault = vm::showOwnVault,
                    onTestLockScreen = { lockPreview = true },
                )
            }
            Box(Modifier.weight(1f).fillMaxSize()) {
                val ownEmpty = state.vaultState == VaultState.Own && state.data.accounts.isEmpty()
                when {
                    ownEmpty && tab == MainTab.Bookings -> EmptyVaultCard(
                        onAddBank = { tab = MainTab.Accounts },
                        onImport = { tab = MainTab.More },
                        onRestore = { tab = MainTab.More },
                        modifier = Modifier.align(Alignment.TopCenter),
                    )
                    else -> when (tab) {
                        MainTab.Bookings -> BookingStreamScreen(
                            state = state,
                            bookings = visible,
                            onSearch = vm::setSearch,
                            onFilter = vm::setFilter,
                            onNewTransfer = { tab = MainTab.Payments },
                            onOpenBooking = { detail = it },
                            onToggleChecked = vm::toggleBookingChecked,
                        )
                        MainTab.Accounts -> Column(Modifier.fillMaxSize()) {
                          AccountsWealthToggle(portfolio = false, onSelect = { tab = if (it) MainTab.Portfolio else MainTab.Accounts })
                          AccountsAndWealthScreen(
                            modifier = Modifier.weight(1f),
                            state = state,
                            badges = AccountsBadges(
                                unreadBookings = state.data.bookings.count { !it.checked },
                                documents = state.data.bookings.sumOf { it.documentCount },
                                unreadByAccount = state.data.bookings.filter { !it.checked }.groupingBy { it.accountId }.eachCount(),
                                documentsByAccount = state.data.bookings.groupBy { it.accountId }.mapValues { (_, v) -> v.sumOf { it.documentCount } },
                            ),
                            onSelectGroup = { vm.selectGroup(it); tab = MainTab.Bookings },
                            onSelectAccount = { vm.selectAccount(it); tab = MainTab.Bookings },
                          )
                        }
                        MainTab.Analytics -> AnalyticsScreen(state, vm, onShowBookings = { tab = MainTab.Bookings })
                        MainTab.Payments -> PaymentsScreen(state = state, onSubmitTransfer = vm::submitTransfer)
                        MainTab.Portfolio -> Column(Modifier.fillMaxSize()) {
                          AccountsWealthToggle(portfolio = true, onSelect = { tab = if (it) MainTab.Portfolio else MainTab.Accounts })
                          PortfolioAndAssetsScreen(
                            state = state,
                            onUpsertLoan = vm::upsertLoan,
                            onDeleteLoan = vm::deleteLoan,
                            onUpsertAsset = vm::upsertAsset,
                            onDeleteAsset = vm::deleteAsset,
                          )
                        }
                        MainTab.More -> SettingsScreen(
                            state = state,
                            onLanguage = { vm.setLanguage(it); onLanguage(it) },
                            onTogglePrivacy = vm::togglePrivacy,
                            onUseOwnFintsId = vm::setOwnFintsId,
                            onFintsProductId = vm::setFintsProductId,
                            onExportBackup = onExportBackup,
                            onRestoreBackup = onRestoreBackup,
                            onOpenRules = { dialog = ManagementDialog.Rules },
                            onOpenTags = { dialog = ManagementDialog.Tags },
                            onOpenCategories = { dialog = ManagementDialog.Categories },
                            onAddBank = { dialog = ManagementDialog.AddBank },
                            onOpenPortfolio = { tab = MainTab.Portfolio },
                        )
                    }
                }
            }
        }
    }

    detail?.let { BookingDetailSheet(it, state, vm, onDismiss = { detail = null }) }

    when (dialog) {
        ManagementDialog.AddBank -> AddBankDialog(
            state = state,
            onDismiss = { dialog = null },
            // Real connection setup is performed by the core bridge once it exposes FinTS sessions.
            onConnect = { },
            onTryDemo = { vm.showDemo(); dialog = null },
        )
        ManagementDialog.Rules -> RulesListDialog(state, vm, onDismiss = { dialog = null })
        ManagementDialog.Tags -> TagManagerDialog(state, onAdd = { vm.addTag(it) }, onDelete = vm::deleteTag, onDismiss = { dialog = null })
        ManagementDialog.Categories -> CategoryManagerDialog(
            state, onAdd = { n, p -> vm.addCategory(n, parentId = p) }, onDelete = vm::deleteCategory, onDismiss = { dialog = null },
        )
        null -> Unit
    }
}

enum class ManagementDialog { AddBank, Rules, Tags, Categories }

/** Rule list with edit/delete/add; opens [RuleEditorDialog]. */
@Composable
private fun RulesListDialog(state: de.girocrab.app.model.AppUiState, vm: AppViewModel, onDismiss: () -> Unit) {
    var editing by remember { mutableStateOf<de.girocrab.app.model.Rule?>(null) }
    var creating by remember { mutableStateOf(false) }
    if (creating || editing != null) {
        RuleEditorDialog(
            state = state,
            initial = editing,
            onDismiss = { creating = false; editing = null },
            onSave = vm::upsertRule,
            onApplyUncategorized = vm::applyRuleToUncategorized,
        )
        return
    }
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Regeln") },
        text = {
            androidx.compose.foundation.lazy.LazyColumn {
                items(state.data.rules, key = { it.id }) { r ->
                    androidx.compose.material3.ListItem(
                        modifier = Modifier.clickable { editing = r },
                        headlineContent = { Text("${r.field.name} ${r.matchKind.name} „${r.pattern}“") },
                        supportingContent = { Text(vm.category(r.categoryId)?.name ?: "–") },
                        trailingContent = {
                            androidx.compose.material3.IconButton(onClick = { vm.deleteRule(r.id) }) {
                                Icon(Icons.Default.Delete, "Löschen")
                            }
                        },
                    )
                }
            }
        },
        confirmButton = { androidx.compose.material3.TextButton(onClick = { creating = true }) { Text("Neue Regel") } },
        dismissButton = { androidx.compose.material3.TextButton(onClick = onDismiss) { Text("Fertig") } },
    )
}


/** Startup overlay: adaptive light/dark (system dark mode or 19:00-07:00), hero crab springs in, fades out after ~950 ms (`skip_intro` disables it). */
@Composable
private fun GiroCrabIntro(skip: Boolean) {
    var show by rememberSaveable { mutableStateOf(!skip) }
    var entered by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        entered = true
        if (show) {
            kotlinx.coroutines.delay(950)
            show = false
        }
    }
    val hour = remember { java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY) }
    val dark = androidx.compose.foundation.isSystemInDarkTheme() || hour >= 19 || hour < 7
    val bg = androidx.compose.ui.graphics.Color(if (dark) 0xFF000000 else 0xFFF2F2F7)
    val fg = androidx.compose.ui.graphics.Color(if (dark) 0xFFF9FAFB else 0xFF111827)
    val sub = androidx.compose.ui.graphics.Color(if (dark) 0xFF9CA3AF else 0xFF6B7280)
    val scale by androidx.compose.animation.core.animateFloatAsState(
        if (entered) 1f else 0.82f,
        androidx.compose.animation.core.spring(dampingRatio = 0.6f, stiffness = 200f),
        label = "introScale",
    )
    val alpha by androidx.compose.animation.core.animateFloatAsState(
        if (entered) 1f else 0f,
        androidx.compose.animation.core.tween(500),
        label = "introAlpha",
    )
    androidx.compose.animation.AnimatedVisibility(
        visible = show,
        enter = androidx.compose.animation.EnterTransition.None,
        exit = androidx.compose.animation.fadeOut(androidx.compose.animation.core.tween(300)),
    ) {
        Box(
            Modifier.fillMaxSize().background(bg).clickable(enabled = false) {},
            contentAlignment = Alignment.Center,
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                modifier = Modifier.scale(scale).graphicsLayer { this.alpha = alpha },
            ) {
                androidx.compose.foundation.Image(
                    painter = androidx.compose.ui.res.painterResource(R.drawable.girocrab_crab),
                    contentDescription = null,
                    modifier = Modifier
                        .size(152.dp)
                        .shadow(16.dp, androidx.compose.foundation.shape.RoundedCornerShape(34.dp), clip = false),
                )
                Text(
                    "GiroCrab",
                    color = fg,
                    fontSize = 28.sp,
                    fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                    modifier = Modifier.padding(top = 20.dp),
                )
                Text(
                    "Finanzen. Lokal. Verschlüsselt.",
                    color = sub,
                    fontSize = 14.sp,
                    modifier = Modifier.padding(top = 6.dp),
                )
            }
        }
    }
}

/**
 * Message for the lock screen. Opening the vault is proven by the account count — the data itself
 * still comes from the mapper in #1429 (3.1), so the handle is closed again right here.
 */
/**
 * Reads the vault into the snapshot the screens show (#1429, 3.1) and closes the handle in the same
 * breath: the ViewModel keeps data, never a native handle. A failure after a successful open is
 * reported as exactly that — the vault did open, only reading it back did not work.
 */
private fun adoptOwnSnapshot(vm: AppViewModel, vault: OwnVault, failure: String): String? = try {
    val snapshot = try {
        vault.snapshot()
    } finally {
        vault.close()
    }
    vm.adoptOwnVault(snapshot)
    null
} catch (e: Exception) {
    "$failure (${e.javaClass.simpleName}: ${e.message})."
}

private fun describeImport(result: ImportResult): String = when (result) {
    is ImportResult.Imported -> {
        // Counting is only a proof that data arrived; the list itself follows with #1429.
        val count = try {
            result.vault.accountCount
        } finally {
            result.vault.close()
        }
        "Sicherung eingespielt: $count Konten. Anzeige folgt mit dem Mapper (#1429)."
    }
    ImportResult.RefusedByUser -> "Import abgebrochen — es wurde nichts geschrieben."
    ImportResult.Unavailable -> "Import ist auf diesem Gerät nicht verfügbar."
    is ImportResult.Failed -> result.message
}

private fun describe(result: UnlockResult): String? = when (result) {
    is UnlockResult.Opened -> {
        // `accountCount` throws when the handle is unusable, so the handle is closed in `finally`
        // — otherwise a failed read leaks the native handle.
        val count = try {
            result.vault.accountCount
        } finally {
            result.vault.close()
        }
        "Tresor geöffnet: $count Konten. Die Anzeige folgt mit dem Mapper (#1429)."
    }
    UnlockResult.NoVaultOnDevice ->
        "Auf diesem Gerät liegt noch kein Tresor. Spiele eine Sicherung vom Desktop ein — dort über „Tresor sichern“ erzeugt."
    UnlockResult.KeyInvalidated ->
        "Der Geräteschlüssel ist ungültig: neue Biometrie oder zurückgesetzte Gerätesperre. " +
            "Ohne ihn sind die Daten nicht lesbar — spiele eine Sicherung vom Desktop ein."
    UnlockResult.VaultMissing ->
        "Die Tresordatei fehlt auf diesem Gerät. Nur eine Sicherung vom Desktop bringt die Daten zurück."
    // The soft lock is only a window: no message, the screen simply goes away.
    UnlockResult.SoftReleased -> null
    UnlockResult.Unavailable -> "Entsperren ist auf diesem Gerät nicht verfügbar."
    is UnlockResult.Refused -> result.message ?: "Entsperren abgebrochen."
}
