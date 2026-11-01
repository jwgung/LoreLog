package com.lorelog.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDate

class ScheduleTest {
    private val today = LocalDate.of(2026, 11, 2)

    @Test
    fun aPageIsSevenDaysFromTodayAndTurnsByAWeek() {
        assertEquals(Schedule.Span("2026-11-02", "2026-11-08"), Schedule.page(today, 0))
        assertEquals(Schedule.Span("2026-11-09", "2026-11-15"), Schedule.page(today, 1))
        assertEquals(Schedule.Span("2026-10-26", "2026-11-01"), Schedule.page(today, -1))
        // across a month and a year
        assertEquals(Schedule.Span("2026-12-28", "2027-01-03"), Schedule.page(LocalDate.of(2026, 12, 28), 0))
    }

    private val body = JSONObject(
        """
        {"from": "2026-11-02", "to": "2026-11-08", "items": [
          {"key": "a", "day": "2026-11-02", "start": "10:00", "end": "11:00", "all_day": false, "title": "설치 사진",
           "place": "현장", "attendees": ["이담당"], "question": "q1",
           "notes": [{"kind": "request", "text": "이담당님이 요청한 「설치 사진」", "guess": false},
                     {"kind": "state", "text": "추정 마지막 상태: 일정 확정", "guess": true}]},
          {"key": "b", "day": "2026-11-04", "start": "", "end": "", "all_day": true, "title": "휴가",
           "place": "", "attendees": [], "notes": [], "question": "q2"},
          {"key": "c", "day": "2026-11-02", "start": "09:00", "title": "아침", "notes": "not a list"},
          {"title": "no day"},
          "not an object"
        ]}
        """.trimIndent(),
    )

    @Test
    fun theEventsAreReadAndWhatIsNotAnEventIsLeftOut() {
        val got = Schedule.parse(body)
        assertEquals(listOf("a", "b", "c"), got.map { it.key })
        val a = got[0]
        assertEquals("10:00", a.start)
        assertEquals("11:00", a.end)
        assertEquals(listOf("이담당"), a.attendees)
        assertEquals(listOf("request", "state"), a.notes.map { it.kind })
        assertTrue(a.notes[1].guess)
        assertTrue(got[1].allDay)
        assertTrue("a broken notes field is no notes", got[2].notes.isEmpty())
    }

    @Test
    fun everyDayOfThePageIsShownInOrderWithItsEventsByTime() {
        val days = Schedule.byDay(Schedule.page(today, 0), Schedule.parse(body))
        assertEquals(7, days.size)
        assertEquals("2026-11-02", days[0].day)
        assertEquals(listOf("c", "a"), days[0].events.map { it.key })
        assertTrue(days[1].events.isEmpty())
        assertEquals(listOf("b"), days[2].events.map { it.key })
        assertEquals("2026-11-08", days[6].day)
    }

    @Test
    fun aDayIsNamedForTheUserWithTodayAndTomorrowSaid() {
        assertEquals("오늘 · 11월 2일 월요일", Schedule.dayLabel("2026-11-02", today))
        assertEquals("내일 · 11월 3일 화요일", Schedule.dayLabel("2026-11-03", today))
        assertEquals("11월 5일 목요일", Schedule.dayLabel("2026-11-05", today))
        assertEquals("2027년 1월 1일 금요일", Schedule.dayLabel("2027-01-01", today))
    }

    @Test
    fun anEventsTimeIsItsStartAndEndOrTheWholeDay() {
        val got = Schedule.parse(body)
        assertEquals("10:00~11:00", Schedule.time(got[0]))
        assertEquals("하루 종일", Schedule.time(got[1]))
        assertEquals("09:00", Schedule.time(got[2]))
    }
}
