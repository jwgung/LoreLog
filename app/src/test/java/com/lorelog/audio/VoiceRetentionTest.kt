package com.lorelog.audio

import com.lorelog.data.Conversation
import org.junit.Assert.assertEquals
import org.junit.Test

class VoiceRetentionTest {
    private val day = 24 * 60 * 60 * 1000L
    private val now = 100 * day

    private fun conv(start: Long, end: Long, status: String) = Conversation(1, start, end, "", "", status, "")

    @Test
    fun aConversationNotAnsweredYetKeepsItsVoiceThoughItWasSent() {
        val voices = listOf(1L to now - 3 * 3_600_000L) // three hours ago, sent long since
        val waiting = listOf(conv(now - 4 * 3_600_000L, now - 2 * 3_600_000L, "pending"))
        assertEquals(emptyList<Long>(), VoiceRetention.toDrop(voices, waiting, now))
    }

    @Test
    fun anAnsweredConversationLetsItsVoiceGo() {
        val at = now - 3 * 3_600_000L
        for (status in listOf("tagged", "skipped", "ambient", "auto")) {
            val c = listOf(conv(at - 60_000, at + 60_000, status))
            assertEquals(status, listOf(1L), VoiceRetention.toDrop(listOf(1L to at), c, now))
        }
    }

    @Test
    fun voiceOfNoConversationYetIsKeptAWeekAndNothingIsKeptLonger() {
        val young = 1L to now - 2 * day
        val old = 2L to now - 8 * day
        val oldWaiting = listOf(conv(now - 8 * day - 60_000, now - 8 * day + 60_000, "pending"))
        assertEquals(listOf(2L), VoiceRetention.toDrop(listOf(young, old), emptyList(), now))
        assertEquals("a week at most, answered or not", listOf(2L), VoiceRetention.toDrop(listOf(old), oldWaiting, now))
    }
}
