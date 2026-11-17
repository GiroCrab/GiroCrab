package de.girocrab.app

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Analytics
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Payments
import androidx.compose.material.icons.filled.ReceiptLong
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
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
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import de.girocrab.app.model.AppViewModel
import de.girocrab.app.model.VaultState
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
    More("Mehr", Icons.Default.MoreHoriz),
}

/** FragmentActivity (not ComponentActivity) because BiometricPrompt requires it. */
class MainActivity : FragmentActivity() {
    private val vm: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            GiroCrabTheme {
                GiroCrabRoot(vm, setSecure = ::setSecure)
            }
        }
    }

    /** FLAG_SECURE hides the app from screenshots and the recents thumbnail. */
    private fun setSecure(secure: Boolean) {
        if (secure) window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        else window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    override fun onStop() {
        super.onStop()
        // Re-lock the own vault when the app leaves the foreground (parity with iOS lock on background).
        if (!isChangingConfigurations && vm.state.value.vaultState == VaultState.Own) vm.lock()
    }
}

@Composable
fun GiroCrabRoot(vm: AppViewModel, setSecure: (Boolean) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var tab by rememberSaveable { mutableStateOf(MainTab.Bookings) }
    var lockPreview by rememberSaveable { mutableStateOf(false) }

    val locked = (state.vaultState == VaultState.Locked && !state.loading) || lockPreview
    LaunchedEffect(state.privacyMode, locked) { setSecure(state.privacyMode || locked) }

    if (state.loading && state.vaultState == VaultState.Locked) {
        Box(Modifier.fillMaxSize())
        return
    }

    if (locked) {
        BiometricLockScreen(
            previewMode = lockPreview,
            onUnlock = { vm.bootstrap() },
            onBackToDemo = { lockPreview = false },
        )
        return
    }

    Scaffold(
        bottomBar = {
            NavigationBar {
                MainTab.entries.forEach { t ->
                    NavigationBarItem(
                        selected = tab == t,
                        onClick = { tab = t },
                        icon = { Icon(t.icon, contentDescription = t.label) },
                        label = { Text(t.label) },
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
                    // Tab content screens are supplied by later milestones.
                    else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(tab.label, style = MaterialTheme.typography.titleMedium)
                    }
                }
            }
        }
    }
}
