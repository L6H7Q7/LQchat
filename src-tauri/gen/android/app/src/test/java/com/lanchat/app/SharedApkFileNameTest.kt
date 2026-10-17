package com.lanchat.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SharedApkFileNameTest {
    @Test fun preservesLocalizedLabels() {
        assertEquals("微信.apk", SharedApkFileName.fromLabel("微信"))
        assertEquals("My App.apk", SharedApkFileName.fromLabel(" My App "))
    }

    @Test fun removesPathSeparatorsAndWindowsInvalidCharacters() {
        assertEquals("App_工具_Pro_.apk", SharedApkFileName.fromLabel("App/工具\\Pro?"))
        assertEquals("App_Name.apk", SharedApkFileName.fromLabel("App\nName"))
        assertEquals("App.apk", SharedApkFileName.fromLabel("App..."))
    }

    @Test fun handlesMissingAndReservedLabels() {
        assertNull(SharedApkFileName.fromLabel("   "))
        assertNull(SharedApkFileName.fromLabel("..."))
        assertEquals("_CON.apk", SharedApkFileName.fromLabel("CON"))
        assertEquals("_com1.release.apk", SharedApkFileName.fromLabel("com1.release"))
    }

    @Test fun boundsLongLabels() {
        val name = SharedApkFileName.fromLabel("应用".repeat(200))!!
        assertTrue(name.length <= 124)
        assertTrue(name.endsWith(".apk"))
    }
}
