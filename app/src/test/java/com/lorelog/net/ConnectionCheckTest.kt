package com.lorelog.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionCheckTest {
    @Test
    fun reachedWithRightTokenIsConnected() {
        assertEquals(ConnectionCheck.Result.Connected, ConnectionCheck.judge(200, null))
    }

    @Test
    fun unauthorizedMeansTheTokenIsWrong() {
        assertEquals(ConnectionCheck.Result.BadToken, ConnectionCheck.judge(401, null))
    }

    @Test
    fun notFoundMeansTheAddressIsNotADrawer() {
        assertEquals(ConnectionCheck.Result.NotDrawer(404), ConnectionCheck.judge(404, null))
    }

    @Test
    fun noAnswerIsUnreachable() {
        assertEquals(ConnectionCheck.Result.Unreachable("ConnectException"), ConnectionCheck.judge(null, "ConnectException"))
    }

    @Test
    fun everyResultSaysSomethingAndConnectedSaysSo() {
        assertTrue(ConnectionCheck.Result.Connected.message.startsWith("✓"))
        for (r in listOf(ConnectionCheck.Result.BadToken, ConnectionCheck.Result.NotDrawer(500), ConnectionCheck.Result.Unreachable("X"))) {
            assertTrue(r.message, r.message.startsWith("⚠"))
        }
    }
}
