package com.lorelog.work

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Calendar
import java.util.TimeZone

class AheadRulesTest {
    private val seoul = TimeZone.getTimeZone("Asia/Seoul")

    private fun at(h: Int, m: Int, day: Int = 2): Long = Calendar.getInstance(seoul).apply {
        clear(); set(2026, Calendar.NOVEMBER, day, h, m, 0)
    }.timeInMillis

    private fun line(event: String, startH: Int, startM: Int = 0, stop: String = "org:client.co.kr", matter: String = "case:1") =
        AheadRules.Line(
            event = event, start = at(startH, startM), at = at(startH, startM) - 10 * 60_000L,
            title = "10분 뒤 「회의」", text = "이담당님이 3월 25일에 요청한 「설치 사진」에 아직 답하지 않았어요.",
            question = "q", stop = stop, edge = "edge:0123456789abcdef", wrong = "w", matter = matter,
        )

    @Test
    fun nothingIsSaidFromTenAtNightToSevenInTheMorning() {
        assertTrue(AheadRules.quiet(at(22, 0), seoul))
        assertTrue(AheadRules.quiet(at(3, 0), seoul))
        assertTrue(AheadRules.quiet(at(6, 59), seoul))
        assertFalse(AheadRules.quiet(at(7, 0), seoul))
        assertFalse(AheadRules.quiet(at(21, 59), seoul))
    }

    @Test
    fun aLineIsSaidTenMinutesBeforeItsEventAndNotAfterItStarted() {
        val l = line("e1", 10)
        assertEquals(AheadRules.Decision.Wait(10 * 60_000L), AheadRules.decide(l, at(9, 40), seoul, emptySet(), emptySet()))
        assertEquals(AheadRules.Decision.Say, AheadRules.decide(l, at(9, 50), seoul, emptySet(), emptySet()))
        assertEquals(AheadRules.Decision.Say, AheadRules.decide(l, at(9, 55), seoul, emptySet(), emptySet()))
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(l, at(10, 0), seoul, emptySet(), emptySet()))
    }

    @Test
    fun aLineDueAtNightIsDropped() {
        val l = line("e1", 22, 15)
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(l, at(21, 0), seoul, emptySet(), emptySet()))
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(l, at(22, 5), seoul, emptySet(), emptySet()))
    }

    @Test
    fun anEventIsSpokenOfOnceAndTheSameEventAndMatterNeverAgain() {
        val l = line("e1", 10)
        val said = setOf(AheadRules.saidKey(l))
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(l, at(9, 50), seoul, said, emptySet()))
        // another matter of the same event: still once per event
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(line("e1", 10, matter = "case:2"), at(9, 50), seoul, said, emptySet()))
        // the same matter at another event may be said
        assertEquals(AheadRules.Decision.Say, AheadRules.decide(line("e2", 11), at(10, 50), seoul, said, emptySet()))
    }

    @Test
    fun aStoppedOrganizationIsNotSpokenOf() {
        val l = line("e1", 10)
        assertEquals(AheadRules.Decision.Drop, AheadRules.decide(l, at(9, 50), seoul, emptySet(), setOf("org:client.co.kr")))
    }

    @Test
    fun threeSwipesFromTheSameOrganizationStopIt() {
        var counts = emptyMap<String, Int>()
        repeat(2) {
            val (c, stopped) = AheadRules.swiped(counts, "org:client.co.kr")
            assertFalse(stopped)
            counts = c
        }
        val (other, otherStopped) = AheadRules.swiped(counts, "org:other.co.kr")
        assertFalse(otherStopped)
        assertEquals(1, other["org:other.co.kr"])
        val (_, stopped) = AheadRules.swiped(other, "org:client.co.kr")
        assertTrue(stopped)
    }

    @Test
    fun theWorkNameIsTheEventAndMatter() {
        assertEquals(AheadRules.workName(line("e1", 10)), AheadRules.workName(line("e1", 10)))
        assertFalse(AheadRules.workName(line("e1", 10)) == AheadRules.workName(line("e2", 10)))
        assertTrue(AheadRules.workName(line("e1", 10)).startsWith("ahead-"))
    }
}
