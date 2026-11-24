package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The window lock (#1428, 2.5) must survive every path that puts data back into the state. Only a
 * successful prompt clears it — everything else would hand out unlocked financial data.
 */
@RunWith(AndroidJUnit4::class)
class AppViewModelLockTest {

    /** `showOwnVault()` restores the cached snapshot; that must not lift the lock. */
    @Test
    fun cachedAdoptionKeepsTheBackgroundLock() {
        val vm = AppViewModel()
        vm.adoptOwnVault(VaultSnapshot())
        vm.lockForBackground()
        assertTrue("the lock has to be set first", vm.state.value.backgroundLocked)

        vm.showOwnVault()

        assertTrue("the cache must not lift the lock", vm.state.value.backgroundLocked)
        assertFalse("but the vault is still there", vm.state.value.vaultState == VaultState.Locked)
    }

    /** A lock that arrives before the vault does (import in the background) applies to the import. */
    @Test
    fun lockBeforeTheVaultArrivesAppliesToTheImport() {
        val vm = AppViewModel()
        assertFalse(vm.state.value.backgroundLocked)

        vm.lockForBackground()
        vm.adoptOwnVault(VaultSnapshot())

        assertTrue("imported data has to start locked", vm.state.value.backgroundLocked)
    }
}
