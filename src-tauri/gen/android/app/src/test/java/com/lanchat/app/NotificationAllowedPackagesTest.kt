package com.lanchat.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class NotificationAllowedPackagesTest {
    private val edge = "com.microsoft.emmx"
    private val stale = "com.yaxisvip.pubgtool.iueg"
    private val messaging = "example.messages"
    private val systemUi = "com.android.systemui"

    @Test fun deselectingEdgeSucceedsDespiteAStaleSavedPackage() {
        val previous = setOf(edge, stale, messaging, systemUi)
        val eligible = setOf(edge, messaging, systemUi)
        assertEquals(setOf(messaging, systemUi), NotificationAllowedPackages.reconcile(
            previous - edge, previous, eligible,
        ))
        assertEquals(setOf(edge, stale, messaging, systemUi), previous)
    }

    @Test fun packageDisabledWhilePickerIsOpenIsRemovedIfItWasPreviouslySaved() {
        assertEquals(setOf(messaging), NotificationAllowedPackages.reconcile(
            setOf(edge, messaging), setOf(edge, messaging), setOf(messaging),
        ))
    }

    @Test fun validNewSelectionsAndAnEmptySelectionRemainSupported() {
        assertEquals(setOf(systemUi), NotificationAllowedPackages.reconcile(
            setOf(systemUi), setOf(stale), setOf(edge, systemUi),
        ))
        assertEquals(emptySet<String>(), NotificationAllowedPackages.reconcile(
            emptySet(), setOf(edge, stale), setOf(edge),
        ))
    }

    @Test fun newlySubmittedInvalidPackagesAreStillRejected() {
        assertThrows(IllegalArgumentException::class.java) {
            NotificationAllowedPackages.reconcile(setOf("not.installed.invalid"), setOf(stale), setOf(edge))
        }
    }
}
