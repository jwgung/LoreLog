package com.lorelog.collect

import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest

/**
 * 폰 캘린더를 금고로 보낼 때의 규칙(#77). 안드로이드에 묶이지 않은 부분만 모았다(읽기와 전송은 [CalendarExport]).
 *
 * - 범위: 지금부터 1년 전 ~ 1년 뒤에 시작하는 일정(반복 일정은 한 번 한 번을 따로, 키 "이벤트id@시작ms").
 * - 새로 생겼거나 바뀐 일정만 `calendar_event` 줄로 보낸다(status "upsert"). 금고는 키마다 마지막 줄을 쓴다.
 * - 보냈던 일정이 범위 안에서 사라지면(폰에서 지움, 시간을 옮김) status "deleted" 줄을 보낸다. 범위 밖으로 밀려난
 *   지난 일정은 지운 것이 아니므로 보내지 않고 기억에서만 뺀다(금고에는 그대로 남는다).
 */
object CalendarSync {
    const val WINDOW_DAYS = 365L
    private const val DAY_MS = 86_400_000L

    data class PhoneEvent(
        val key: String,
        /** 캘린더의 iCalendar UID(UID_2445). CalDAV로도 들어오는 같은 일정을 금고가 한 번만 세는 데 쓴다. 없으면 "". */
        val uid: String,
        val calendar: String,
        val title: String,
        val start: Long,
        val end: Long,
        val allDay: Boolean,
        val location: String,
        val description: String,
        /** "이름 <메일>" 또는 "메일" */
        val attendees: List<String>,
    )

    /** 보낸 일정: 그 내용의 지문과 시작 시각(범위를 벗어났는지 보려고). */
    data class Sent(val digest: String, val start: Long)

    data class Plan(
        val upserts: List<PhoneEvent>,
        /** (키, 시작) */
        val deleted: List<Pair<String, Long>>,
        /** 범위 밖으로 밀려나 기억에서만 뺄 키 */
        val forgotten: List<String>,
    ) {
        fun isEmpty() = upserts.isEmpty() && deleted.isEmpty() && forgotten.isEmpty()
    }

    fun window(nowMs: Long): Pair<Long, Long> = (nowMs - WINDOW_DAYS * DAY_MS) to (nowMs + WINDOW_DAYS * DAY_MS)

    fun digest(e: PhoneEvent): String {
        val text = fields(e).entries.joinToString("\u0001") { "${it.key}=${it.value}" }
        return MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    /**
     * 보낼 것. [complete]가 아니면(읽다 만 목록: 한도에 닿음) 없는 일정을 지운 것으로 보지 않는다(새것·바뀐 것만).
     */
    fun plan(current: List<PhoneEvent>, sent: Map<String, Sent>, nowMs: Long, complete: Boolean = true): Plan {
        val (from, to) = window(nowMs)
        val keys = current.mapTo(HashSet()) { it.key }
        val upserts = current.filter { sent[it.key]?.digest != digest(it) }
        val deleted = ArrayList<Pair<String, Long>>()
        val forgotten = ArrayList<String>()
        for ((key, s) in if (complete) sent else emptyMap()) {
            if (key in keys) continue
            if (s.start in from..to) deleted.add(key to s.start) else forgotten.add(key)
        }
        return Plan(upserts, deleted.sortedBy { it.first }, forgotten.sorted())
    }

    private fun fields(e: PhoneEvent): Map<String, Any> = linkedMapOf(
        "key" to e.key, "uid" to e.uid, "calendar" to e.calendar, "title" to e.title, "start" to e.start,
        "end" to e.end, "all_day" to e.allDay, "location" to e.location, "description" to e.description,
        "attendees" to JSONArray(e.attendees),
    )

    /** 금고로 보낼 한 줄. ts 는 보낸 때(금고는 그날 파일에 둔다: 나중 파일·나중 줄이 이긴다). */
    fun record(e: PhoneEvent, nowMs: Long): String = JSONObject().apply {
        put("type", "calendar_event"); put("ts", nowMs)
        fields(e).forEach { (k, v) -> put(k, v) }
        put("status", "upsert")
    }.toString()

    fun deletedRecord(key: String, start: Long, nowMs: Long): String = JSONObject().apply {
        put("type", "calendar_event"); put("ts", nowMs); put("key", key); put("start", start); put("status", "deleted")
    }.toString()

    /** 보낸 기억. 서버 주소별(새 서버는 전부 받는다). */
    fun encodeState(url: String, sent: Map<String, Sent>): String {
        val m = JSONObject()
        sent.forEach { (k, s) -> m.put(k, JSONArray().put(s.digest).put(s.start)) }
        return JSONObject().put("url", url).put("sent", m).toString()
    }

    fun decodeState(text: String?, url: String): Map<String, Sent> {
        val root = runCatching { JSONObject(text ?: return emptyMap()) }.getOrNull() ?: return emptyMap()
        if (root.optString("url") != url) return emptyMap()
        val m = root.optJSONObject("sent") ?: return emptyMap()
        val out = HashMap<String, Sent>()
        for (k in m.keys()) {
            val a = m.optJSONArray(k) ?: continue
            out[k] = Sent(a.optString(0), a.optLong(1))
        }
        return out
    }

    /** 줄들을 [maxBytes] 안으로 나눈 묶음(줄 번호). 한 줄이 넘어도 그 줄 하나는 한 묶음. */
    fun chunks(lines: List<String>, maxBytes: Long): List<List<Int>> {
        val out = ArrayList<List<Int>>()
        var cur = ArrayList<Int>()
        var bytes = 0L
        lines.forEachIndexed { i, l ->
            val size = l.toByteArray(Charsets.UTF_8).size + 1L
            if (cur.isNotEmpty() && bytes + size > maxBytes) {
                out.add(cur); cur = ArrayList(); bytes = 0L
            }
            cur.add(i); bytes += size
        }
        if (cur.isNotEmpty()) out.add(cur)
        return out
    }
}
