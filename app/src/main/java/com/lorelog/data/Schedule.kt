package com.lorelog.data

import org.json.JSONArray
import org.json.JSONObject
import java.time.LocalDate
import java.time.format.TextStyle
import java.util.Locale

/**
 * 일정 화면(#62)의 규칙. 서버의 GET /schedule 이 서랍에 들어온 일정(CalDAV와 이 폰의 캘린더)에 비서 메모를 얹어
 * 돌려주고, 화면은 오늘부터 7일을 보여 주며 한 주씩 앞뒤로 넘긴다. 메모는 서버의 템플릿 문장이고, 추정은 서버가
 * "추정"이라고 쓴다. 화면은 그대로 보여 줄 뿐 새로 만들지 않는다.
 */
object Schedule {
    const val DAYS = 7L

    data class Span(val from: String, val to: String)
    data class Note(val kind: String, val text: String, val guess: Boolean)
    data class Event(
        val key: String,
        val day: String,
        val start: String,
        val end: String,
        val allDay: Boolean,
        val title: String,
        val place: String,
        val attendees: List<String>,
        val notes: List<Note>,
        val question: String,
    )
    data class Day(val day: String, val events: List<Event>)

    /** 오늘부터 [offset] 주 뒤(음수면 앞)의 7일. */
    fun page(today: LocalDate, offset: Int): Span {
        val from = today.plusDays(DAYS * offset)
        return Span(from.toString(), from.plusDays(DAYS - 1).toString())
    }

    private fun strings(a: JSONArray?): List<String> =
        if (a == null) emptyList() else (0 until a.length()).mapNotNull { a.opt(it) as? String }

    /** 서버의 답에서 일정만 읽는다. 날짜가 없거나 모양이 다른 것은 뺀다. */
    fun parse(o: JSONObject): List<Event> {
        val items = o.optJSONArray("items") ?: return emptyList()
        return (0 until items.length()).mapNotNull { i ->
            val e = items.opt(i) as? JSONObject ?: return@mapNotNull null
            val day = e.optString("day")
            if (runCatching { LocalDate.parse(day) }.isFailure) return@mapNotNull null
            val notes = (e.opt("notes") as? JSONArray)?.let { a ->
                (0 until a.length()).mapNotNull { j ->
                    val n = a.opt(j) as? JSONObject ?: return@mapNotNull null
                    Note(n.optString("kind"), n.optString("text"), n.optBoolean("guess")).takeIf { it.text.isNotBlank() }
                }
            } ?: emptyList()
            Event(
                key = e.optString("key"), day = day, start = e.optString("start"), end = e.optString("end"),
                allDay = e.optBoolean("all_day"), title = e.optString("title").ifBlank { "(제목 없음)" },
                place = e.optString("place"), attendees = strings(e.optJSONArray("attendees")), notes = notes,
                question = e.optString("question"),
            )
        }
    }

    /** 쪽의 날마다 그날 일정: 하루 종일인 것 먼저, 그다음 시작 시각 순. 일정이 없는 날도 있다. */
    fun byDay(span: Span, events: List<Event>): List<Day> {
        val first = LocalDate.parse(span.from)
        val last = LocalDate.parse(span.to)
        val out = mutableListOf<Day>()
        var d = first
        while (!d.isAfter(last)) {
            val day = d.toString()
            out.add(Day(day, events.filter { it.day == day }.sortedWith(compareBy({ !it.allDay }, { it.start }))))
            d = d.plusDays(1)
        }
        return out
    }

    /** "오늘 · 11월 2일 월요일". 올해가 아니면 연도를 붙인다. */
    fun dayLabel(day: String, today: LocalDate): String {
        val d = LocalDate.parse(day)
        val week = d.dayOfWeek.getDisplayName(TextStyle.FULL, Locale.KOREAN)
        val date = (if (d.year != today.year) "${d.year}년 " else "") + "${d.monthValue}월 ${d.dayOfMonth}일 $week"
        return when (d) {
            today -> "오늘 · $date"
            today.plusDays(1) -> "내일 · $date"
            else -> date
        }
    }

    fun time(e: Event): String = when {
        e.allDay || e.start.isBlank() -> "하루 종일"
        e.end.isBlank() -> e.start
        else -> "${e.start}~${e.end}"
    }
}
