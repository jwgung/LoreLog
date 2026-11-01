package com.lorelog.collect

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CalendarSyncTest {
    private val day = 86_400_000L
    private val now = 1_793_000_000_000L // 2026-10-26

    private fun ev(key: String, start: Long, title: String = "회의", attendees: List<String> = emptyList()) =
        CalendarSync.PhoneEvent(
            key = key, uid = "", calendar = "업무", title = title, start = start, end = start + 3_600_000L,
            allDay = false, location = "본사", description = "", attendees = attendees,
        )

    @Test
    fun theWindowIsAYearBackAndAYearAhead() {
        val (from, to) = CalendarSync.window(now)
        assertEquals(now - 365 * day, from)
        assertEquals(now + 365 * day, to)
    }

    @Test
    fun anEventIsSentWhenNewOrChangedAndNotAgainWhenTheSame() {
        val a = ev("1@100", now + day)
        val first = CalendarSync.plan(listOf(a), emptyMap(), now)
        assertEquals(listOf(a), first.upserts)
        val sent = first.upserts.associate { it.key to CalendarSync.Sent(CalendarSync.digest(it), it.start) }
        assertTrue(CalendarSync.plan(listOf(a), sent, now).isEmpty())
        val renamed = a.copy(title = "분기 회의")
        assertEquals(listOf(renamed), CalendarSync.plan(listOf(renamed), sent, now).upserts)
        val moreAttendees = a.copy(attendees = listOf("lee@client.co.kr"))
        assertEquals(listOf(moreAttendees), CalendarSync.plan(listOf(moreAttendees), sent, now).upserts)
    }

    @Test
    fun anEventGoneFromTheWindowIsDeletedButOneThatOnlyFellOutOfItIsForgottenQuietly() {
        val gone = ev("1@100", now + day)
        val old = ev("2@100", now - 366 * day)
        val sent = listOf(gone, old).associate { it.key to CalendarSync.Sent(CalendarSync.digest(it), it.start) }
        val plan = CalendarSync.plan(emptyList(), sent, now)
        assertEquals(listOf("1@100"), plan.deleted.map { it.first })
        assertEquals(listOf("2@100"), plan.forgotten)
        assertTrue(plan.upserts.isEmpty())
    }

    @Test
    fun aListReadOnlyInPartDeletesNothing() {
        val gone = ev("1@100", now + day)
        val sent = mapOf(gone.key to CalendarSync.Sent(CalendarSync.digest(gone), gone.start))
        val fresh = ev("3@100", now + 2 * day)
        val plan = CalendarSync.plan(listOf(fresh), sent, now, complete = false)
        assertEquals(listOf(fresh), plan.upserts)
        assertTrue(plan.deleted.isEmpty())
        assertTrue(plan.forgotten.isEmpty())
    }

    @Test
    fun aRecordCarriesWhatTheVaultIndexes() {
        val e = ev("7@1", now + day, title = "분기 회의", attendees = listOf("이담당 <lee@client.co.kr>"))
            .copy(uid = "abc@google.com")
        val r = JSONObject(CalendarSync.record(e, now))
        assertEquals("calendar_event", r.getString("type"))
        assertEquals(now, r.getLong("ts"))
        assertEquals("7@1", r.getString("key"))
        assertEquals("abc@google.com", r.getString("uid"))
        assertEquals("업무", r.getString("calendar"))
        assertEquals("분기 회의", r.getString("title"))
        assertEquals(now + day, r.getLong("start"))
        assertEquals(now + day + 3_600_000L, r.getLong("end"))
        assertFalse(r.getBoolean("all_day"))
        assertEquals("본사", r.getString("location"))
        assertEquals("이담당 <lee@client.co.kr>", r.getJSONArray("attendees").getString(0))
        assertEquals("upsert", r.getString("status"))
        val d = JSONObject(CalendarSync.deletedRecord("7@1", now + day, now))
        assertEquals("calendar_event", d.getString("type"))
        assertEquals("deleted", d.getString("status"))
        assertEquals(now + day, d.getLong("start"))
    }

    @Test
    fun whatWasSentIsKeptPerServer() {
        val sent = mapOf("1@100" to CalendarSync.Sent("ab", 5L))
        val text = CalendarSync.encodeState("https://vault/ingest/lorelog", sent)
        assertEquals(sent, CalendarSync.decodeState(text, "https://vault/ingest/lorelog"))
        assertTrue("a new server gets everything", CalendarSync.decodeState(text, "https://other/ingest/lorelog").isEmpty())
        assertTrue(CalendarSync.decodeState("not json", "https://vault/ingest/lorelog").isEmpty())
    }

    @Test
    fun linesAreSentInPiecesUnderTheBodyLimit() {
        val lines = listOf("a".repeat(40), "b".repeat(40), "c".repeat(40))
        assertEquals(listOf(listOf(0, 1), listOf(2)), CalendarSync.chunks(lines, 90))
        assertEquals(listOf(listOf(0), listOf(1), listOf(2)), CalendarSync.chunks(lines, 10))
    }
}
