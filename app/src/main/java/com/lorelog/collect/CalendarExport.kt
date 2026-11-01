package com.lorelog.collect

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.util.Log
import com.lorelog.data.LogRepository
import com.lorelog.net.EndpointConfig
import com.lorelog.net.HttpSink
import java.io.File

/**
 * 폰 캘린더에 등록된 일정을 서랍으로 보낸다(#77): 1년 전부터 1년 뒤까지, 새로 생기거나 바뀐 것과 지운 것([CalendarSync]).
 * 서랍은 이것을 일정으로 색인해 '이번 달'(#61), 먼저 말하기(#60), 일정 화면에서 쓴다. CalDAV로도 들어오는 일정은 서랍이
 * 한 번만 센다(UID, 없으면 시작 분과 제목).
 *
 * 개인정보: "캘린더" 수집을 끄거나 권한을 거두면 보내지 않고, 그때까지 보낸 일정은 지운 것으로 한 번 보내 서랍에서도
 * 빠지게 한다(주소록과 같은 규칙, 이것은 작아서 망을 가리지 않는다). 캘린더 앱에서 숨긴 캘린더(VISIBLE=0)와 취소된
 * 일정은 보내지 않는다. 다른 기록과 같은 서랍 주소로 보낸다([EndpointConfig]).
 * 기록 전송과 함께 돈다([LogRepository.pushToHttp]): 자동 전송은 "와이파이에서만" 설정을 따른다.
 */
object CalendarExport {
    private const val TAG = "CalendarExport"
    private const val STATE_FILE = "calendar_sent.json"
    private const val MAX_EVENTS = 50_000
    private const val MAX_TEXT = 4_000
    private const val MAX_ATTENDEES = 200
    private val ANGLES = Regex("[<>]")

    private fun stateFile(context: Context) = File(context.noBackupFilesDir, STATE_FILE)

    private fun allowed(context: Context): Boolean =
        CollectorPrefs.enabled(context, CollectorPrefs.CALENDAR) &&
            context.checkSelfPermission(Manifest.permission.READ_CALENDAR) == PackageManager.PERMISSION_GRANTED

    /** 범위 안의 일정 인스턴스(보이는 캘린더, 취소 안 된 것)와 다 읽었는지. 캘린더를 못 읽으면 예외(지운 것으로 보지 않게). */
    private fun read(context: Context, from: Long, to: Long): Pair<List<CalendarSync.PhoneEvent>, Boolean> {
        val builder = CalendarContract.Instances.CONTENT_URI.buildUpon()
        ContentUris.appendId(builder, from)
        ContentUris.appendId(builder, to)
        val cols = arrayOf(
            CalendarContract.Instances.EVENT_ID,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.END,
            CalendarContract.Instances.TITLE,
            CalendarContract.Instances.EVENT_LOCATION,
            CalendarContract.Instances.DESCRIPTION,
            CalendarContract.Instances.ALL_DAY,
            CalendarContract.Instances.CALENDAR_DISPLAY_NAME,
        )
        val selection = "${CalendarContract.Instances.VISIBLE} = 1 AND (${CalendarContract.Instances.STATUS} IS NULL OR " +
            "${CalendarContract.Instances.STATUS} != ${CalendarContract.Events.STATUS_CANCELED})"
        val ids = ArrayList<Long>()
        val events = ArrayList<CalendarSync.PhoneEvent>()
        var complete = true
        val c = context.contentResolver.query(builder.build(), cols, selection, null, "${CalendarContract.Instances.BEGIN} ASC")
            ?: error("the calendar provider gave no cursor")
        c.use {
            while (c.moveToNext()) {
                if (events.size >= MAX_EVENTS) {
                    complete = false
                    break
                }
                val eventId = c.getLong(0)
                val begin = c.getLong(1)
                val text = { i: Int -> c.getString(i)?.trim().orEmpty().take(MAX_TEXT) }
                ids.add(eventId)
                events.add(
                    CalendarSync.PhoneEvent(
                        key = "$eventId@$begin", uid = "", calendar = text(7), title = text(3),
                        start = begin, end = if (c.isNull(2)) begin else c.getLong(2), allDay = c.getInt(6) == 1,
                        location = text(4), description = text(5), attendees = emptyList(),
                    ),
                )
            }
        }
        val distinct = LinkedHashSet(ids)
        val people = attendees(context, distinct)
        val uids = uids(context, distinct)
        val out = events.mapIndexed { i, e ->
            e.copy(uid = uids[ids[i]].orEmpty(), attendees = people[ids[i]].orEmpty())
        }
        return out to complete
    }

    /** 일정마다 iCalendar UID(Events.UID_2445). 없는 일정은 빠진다. */
    private fun uids(context: Context, ids: Collection<Long>): Map<Long, String> {
        val out = HashMap<Long, String>()
        ids.chunked(500).forEach { chunk ->
            context.contentResolver.query(
                CalendarContract.Events.CONTENT_URI,
                arrayOf(CalendarContract.Events._ID, CalendarContract.Events.UID_2445),
                "${CalendarContract.Events._ID} IN (${chunk.joinToString(",")})", null, null,
            )?.use { c ->
                while (c.moveToNext()) c.getString(1)?.trim()?.takeIf { it.isNotEmpty() }?.let { out[c.getLong(0)] = it.take(300) }
            }
        }
        return out
    }

    /** 일정마다 참석자("이름 <메일>" 또는 "메일"). 메일 주소가 없는 참석자는 누구인지 확실하지 않아 뺀다. */
    private fun attendees(context: Context, ids: Collection<Long>): Map<Long, List<String>> {
        val out = HashMap<Long, MutableList<String>>()
        ids.chunked(500).forEach { chunk ->
            context.contentResolver.query(
                CalendarContract.Attendees.CONTENT_URI,
                arrayOf(CalendarContract.Attendees.EVENT_ID, CalendarContract.Attendees.ATTENDEE_NAME, CalendarContract.Attendees.ATTENDEE_EMAIL),
                "${CalendarContract.Attendees.EVENT_ID} IN (${chunk.joinToString(",")})", null, null,
            )?.use { c ->
                while (c.moveToNext()) {
                    val mail = c.getString(2)?.trim().orEmpty()
                    if (!mail.contains('@')) continue
                    val name = c.getString(1)?.trim().orEmpty().replace(ANGLES, "")
                    val list = out.getOrPut(c.getLong(0)) { ArrayList() }
                    val one = if (name.isEmpty() || name.equals(mail, ignoreCase = true)) mail else "$name <$mail>"
                    if (list.size < MAX_ATTENDEES && one !in list) list.add(one)
                }
            }
        }
        return out
    }

    /**
     * 바뀐 일정을 보낸다. [mayUpsert]가 아니면(와이파이에서만 켬, 지금은 데이터 망) 새것·바뀐 것은 다음으로 미루고,
     * 끄거나 권한을 거둔 뒤의 지우기만 보낸다.
     * 반환: 보낸 줄 수(0 바뀐 것 없음), -1 보내지 않음(주소·권한·설정·망), -2 실패(캘린더를 못 읽음, 전송 실패: 보낸 묶음까지는 기억).
     */
    suspend fun push(context: Context, mayUpsert: Boolean): Int = runCatching { pushInner(context, mayUpsert) }
        .getOrElse { Log.w(TAG, "push failed: ${it.javaClass.simpleName}"); -2 }
        .also { Log.i(TAG, "push -> $it") }

    private suspend fun pushInner(context: Context, mayUpsert: Boolean): Int {
        val url = EndpointConfig.url(context) ?: return -1
        val file = stateFile(context)
        val sent = HashMap(CalendarSync.decodeState(runCatching { file.readText() }.getOrNull(), url))
        val now = System.currentTimeMillis()
        val lines = ArrayList<String>()
        /** 줄마다 그 줄을 보내면 기억에 남길 것(null 이면 기억에서 뺀다) */
        val after = ArrayList<Pair<String, CalendarSync.Sent?>>()
        if (!allowed(context)) {
            if (sent.isEmpty()) return -1
            sent.forEach { (k, s) -> lines.add(CalendarSync.deletedRecord(k, s.start, now)); after.add(k to null) }
        } else {
            if (!mayUpsert) return -1
            val (from, to) = CalendarSync.window(now)
            val (events, complete) = read(context, from, to)
            if (!complete) Log.w(TAG, "more than $MAX_EVENTS events: sending what was read, deleting nothing")
            val plan = CalendarSync.plan(events, sent, now, complete)
            if (plan.isEmpty()) return 0
            plan.forgotten.forEach { sent.remove(it) }
            plan.upserts.forEach { e ->
                lines.add(CalendarSync.record(e, now)); after.add(e.key to CalendarSync.Sent(CalendarSync.digest(e), e.start))
            }
            plan.deleted.forEach { (k, start) -> lines.add(CalendarSync.deletedRecord(k, start, now)); after.add(k to null) }
            if (lines.isEmpty()) {
                save(file, url, sent) // only forgotten ones
                return 0
            }
        }
        val token = EndpointConfig.token(context)
        var count = 0
        for (chunk in CalendarSync.chunks(lines, LogRepository.PUSH_CHUNK_BYTES)) {
            if (!HttpSink.post(url, token, chunk.joinToString("\n", postfix = "\n") { lines[it] })) {
                save(file, url, sent)
                return -2
            }
            chunk.forEach { i -> val (k, s) = after[i]; if (s == null) sent.remove(k) else sent[k] = s }
            save(file, url, sent)
            count += chunk.size
        }
        return count
    }

    private fun save(file: File, url: String, sent: Map<String, CalendarSync.Sent>) {
        val tmp = File(file.parentFile, "${file.name}.tmp")
        tmp.writeText(CalendarSync.encodeState(url, sent))
        if (!tmp.renameTo(file)) file.writeText(tmp.readText())
    }
}
