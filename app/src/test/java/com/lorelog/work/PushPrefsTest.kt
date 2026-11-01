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
}
