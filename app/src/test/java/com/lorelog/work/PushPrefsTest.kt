package com.lorelog.work

import androidx.work.NetworkType
import org.junit.Assert.assertEquals
import org.junit.Test

class PushPrefsTest {
    @Test
    fun wifiOnlyRequiresUnmeteredNetwork() {
        assertEquals(NetworkType.UNMETERED, PushPrefs.networkType(wifiOnly = true))
    }

    @Test
    fun defaultSendsOnAnyConnectedNetwork() {
        assertEquals(NetworkType.CONNECTED, PushPrefs.networkType(wifiOnly = false))
    }

    @Test
    fun wifiOnlyHoldsAutomaticSendsOffWifiButNotWhatTheUserAsksFor() {
        assertEquals(false, PushPrefs.mayAutoSend(wifiOnly = true, unmetered = false))
        assertEquals(true, PushPrefs.mayAutoSend(wifiOnly = true, unmetered = true))
        assertEquals(true, PushPrefs.mayAutoSend(wifiOnly = false, unmetered = false))
    }
}
