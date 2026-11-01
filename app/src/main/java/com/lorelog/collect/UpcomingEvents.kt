package com.lorelog.collect

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.provider.CalendarContract
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject

/**
 * 앞으로 시작할 시스템 캘린더 일정(#60): 비서가 그 직전에 먼저 말할 것을 서랍에 묻는 재료. 시작 시각, 제목,
 * 참석자 메일 주소, 종일 여부만 보낸다. 캘린더 권한이 없으면 빈 목록(서랍은 자기 캘린더 일정으로 답한다).
 */
object UpcomingEvents {
    fun between(context: Context, fromMs: Long, toMs: Long): JSONArray {
        val out = JSONArray()
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CALENDAR) != PackageManager.PERMISSION_GRANTED) {
            return out
        }
        val cr = context.contentResolver
        val builder = CalendarContract.Instances.CONTENT_URI.buildUpon()
        ContentUris.appendId(builder, fromMs)
        ContentUris.appendId(builder, toMs)
        val cols = arrayOf(
            CalendarContract.Instances.EVENT_ID,
            CalendarContract.Instances.BEGIN,
            CalendarContract.Instances.TITLE,
            CalendarContract.Instances.ALL_DAY,
        )
        runCatching {
            cr.query(builder.build(), cols, "${CalendarContract.Instances.BEGIN} > ?", arrayOf(fromMs.toString()), "${CalendarContract.Instances.BEGIN} ASC")?.use { c ->
                while (c.moveToNext() && out.length() < 100) {
                    val eventId = c.getLong(0)
                    val begin = c.getLong(1)
                    out.put(
                        JSONObject()
                            .put("key", "$eventId@$begin")
                            .put("start", begin)
                            .put("title", c.getString(2).orEmpty())
                            .put("all_day", c.getInt(3) == 1)
                            .put("attendees", JSONArray(emails(context, eventId))),
                    )
                }
            }
        }
        return out
    }

    private fun emails(context: Context, eventId: Long): List<String> {
        val out = ArrayList<String>()
        context.contentResolver.query(
            CalendarContract.Attendees.CONTENT_URI,
            arrayOf(CalendarContract.Attendees.ATTENDEE_EMAIL),
            "${CalendarContract.Attendees.EVENT_ID} = ?", arrayOf(eventId.toString()), null,
        )?.use { c ->
            while (c.moveToNext()) c.getString(0)?.trim()?.takeIf { it.contains('@') }?.let { out.add(it) }
        }
        return out
    }
}
