package com.lorelog.data

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import net.zetetic.database.sqlcipher.SQLiteDatabase
import org.json.JSONArray
import org.json.JSONObject

/**
 * 기기 로컬 저장. SQLCipher 로 DB 파일을 암호화한다. 암호는 [DbKey] 가
 * Android Keystore 로 보호해 둔 랜덤 키를 쓴다. 모든 DB 접근을 이 클래스로 모은다.
 */
class LogStore(context: Context, password: ByteArray) {

    private val db: SQLiteDatabase

    init {
        System.loadLibrary("sqlcipher")
        val file = context.applicationContext.getDatabasePath(DB_NAME)
        file.parentFile?.mkdirs()
        db = SQLiteDatabase.openOrCreateDatabase(file, password, null, null)
        createSchema(db)
    }

    private fun createSchema(db: SQLiteDatabase) {
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_VOICE (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, text TEXT NOT NULL, speaker TEXT)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_voice_ts ON $TABLE_VOICE(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_APP (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, pkg TEXT NOT NULL, label TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_app_ts ON $TABLE_APP(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_SCREEN (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, pkg TEXT NOT NULL, label TEXT NOT NULL, text TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_screen_ts ON $TABLE_SCREEN(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_LOCATION (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, lat REAL NOT NULL, lon REAL NOT NULL, accuracy REAL NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_location_ts ON $TABLE_LOCATION(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_NOTIF (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, pkg TEXT NOT NULL, label TEXT NOT NULL, title TEXT NOT NULL, text TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_notif_ts ON $TABLE_NOTIF(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_SUMMARY (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, period_start INTEGER NOT NULL, period_end INTEGER NOT NULL, text TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_summary_ts ON $TABLE_SUMMARY(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_COMM (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, kind TEXT NOT NULL, direction TEXT NOT NULL, party TEXT NOT NULL, party_name TEXT, content TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_comm_ts ON $TABLE_COMM(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_CAL (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, title TEXT NOT NULL, location TEXT NOT NULL, end_ms INTEGER NOT NULL, attendees TEXT NOT NULL DEFAULT '')")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_cal_ts ON $TABLE_CAL(ts)")
        db.execSQL("CREATE TABLE IF NOT EXISTS $TABLE_CONV (id INTEGER PRIMARY KEY AUTOINCREMENT, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, participants TEXT NOT NULL, source TEXT NOT NULL, status TEXT NOT NULL, context TEXT NOT NULL)")
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_conv_start ON $TABLE_CONV(start_ms)")
        // 기존 DB(열 없는 calendar_event)를 위한 보강. 이미 있으면 무시.
        runCatching { db.execSQL("ALTER TABLE $TABLE_CAL ADD COLUMN attendees TEXT NOT NULL DEFAULT ''") }
        // 음성 행에 보관 오디오 파일명(voice_kept/ 아래). 내보낸 뒤 또는 보존 기간 뒤 지우면 NULL.
        runCatching { db.execSQL("ALTER TABLE $TABLE_VOICE ADD COLUMN audio TEXT") }
    }

    fun insertVoice(timestampMs: Long, text: String, speaker: String?, audio: String? = null): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("text", text)
            put("speaker", speaker)
            put("audio", audio)
        }
        return db.insert(TABLE_VOICE, null, values)
    }

    fun recentVoice(limit: Int): List<VoiceSegment> {
        val out = ArrayList<VoiceSegment>()
        db.query(TABLE_VOICE, arrayOf("id", "ts", "text", "speaker", "audio"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(
                    VoiceSegment(
                        id = c.getLong(0),
                        timestampMs = c.getLong(1),
                        text = c.getString(2),
                        speaker = if (c.isNull(3)) null else c.getString(3),
                        audio = if (c.isNull(4)) null else c.getString(4),
                    ),
                )
            }
        }
        return out
    }

    /** 보관 오디오가 붙은 음성 행 중 조건에 맞는 (id, 파일명). [maxId] 이하 또는 ts < [beforeMs]. */
    fun voiceAudioToDrop(maxId: Long, beforeMs: Long): List<Pair<Long, String>> {
        val out = ArrayList<Pair<Long, String>>()
        db.query(TABLE_VOICE, arrayOf("id", "audio"), "audio IS NOT NULL AND (id <= ? OR ts < ?)",
            arrayOf(maxId.toString(), beforeMs.toString()), null, null, "id ASC").use { c ->
            while (c.moveToNext()) out.add(c.getLong(0) to c.getString(1))
        }
        return out
    }

    /** 음성 행들의 보관 오디오 참조를 지운다(파일은 호출자가 지운다). */
    fun clearVoiceAudio(ids: Collection<Long>) {
        if (ids.isEmpty()) return
        db.beginTransaction()
        try {
            val values = ContentValues().apply { putNull("audio") }
            ids.chunked(500).forEach { chunk ->
                db.update(TABLE_VOICE, values, "id IN (${chunk.joinToString(",")})", null)
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    /** 현재 보관 오디오로 참조 중인 파일명 전체(고아 파일 정리용). */
    fun referencedVoiceAudio(): Set<String> {
        val out = HashSet<String>()
        db.query(TABLE_VOICE, arrayOf("audio"), "audio IS NOT NULL", null, null, null, null).use { c ->
            while (c.moveToNext()) out.add(c.getString(0))
        }
        return out
    }

    fun insertApp(timestampMs: Long, packageName: String, label: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("pkg", packageName)
            put("label", label)
        }
        return db.insert(TABLE_APP, null, values)
    }

    fun recentApp(limit: Int): List<AppEvent> {
        val out = ArrayList<AppEvent>()
        db.query(TABLE_APP, arrayOf("id", "ts", "pkg", "label"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(AppEvent(c.getLong(0), c.getLong(1), c.getString(2), c.getString(3)))
            }
        }
        return out
    }

    fun insertScreen(timestampMs: Long, packageName: String, label: String, text: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("pkg", packageName)
            put("label", label)
            put("text", text)
        }
        return db.insert(TABLE_SCREEN, null, values)
    }

    fun recentScreen(limit: Int): List<ScreenContentEvent> {
        val out = ArrayList<ScreenContentEvent>()
        db.query(TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(ScreenContentEvent(c.getLong(0), c.getLong(1), c.getString(2), c.getString(3), c.getString(4)))
            }
        }
        return out
    }

    fun insertLocation(timestampMs: Long, lat: Double, lon: Double, accuracyM: Float): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("lat", lat)
            put("lon", lon)
            put("accuracy", accuracyM)
        }
        return db.insert(TABLE_LOCATION, null, values)
    }

    fun recentLocation(limit: Int): List<LocationPoint> {
        val out = ArrayList<LocationPoint>()
        db.query(TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(LocationPoint(c.getLong(0), c.getLong(1), c.getDouble(2), c.getDouble(3), c.getFloat(4)))
            }
        }
        return out
    }

    fun insertNotification(timestampMs: Long, pkg: String, label: String, title: String, text: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("pkg", pkg)
            put("label", label)
            put("title", title)
            put("text", text)
        }
        return db.insert(TABLE_NOTIF, null, values)
    }

    fun recentNotification(limit: Int): List<NotificationEvent> {
        val out = ArrayList<NotificationEvent>()
        db.query(TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(
                    NotificationEvent(
                        c.getLong(0), c.getLong(1), c.getString(2),
                        c.getString(3), c.getString(4), c.getString(5),
                    ),
                )
            }
        }
        return out
    }

    /** 모든 기록을 JSON 문자열로. 지식그래프 등 외부로 내보낼 때 쓴다. */
    fun exportJson(exportedAtMs: Long, includeSummary: Boolean = true): String {
        val root = JSONObject()
        root.put("exportedAt", exportedAtMs)
        root.put("voice", dump(TABLE_VOICE, arrayOf("id", "ts", "text", "speaker")))
        root.put("apps", dump(TABLE_APP, arrayOf("id", "ts", "pkg", "label")))
        root.put("screen", dump(TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text")))
        root.put("locations", dump(TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy")))
        root.put("notifications", dump(TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text")))
        if (includeSummary) root.put("summaries", dump(TABLE_SUMMARY, arrayOf("id", "ts", "period_start", "period_end", "text")))
        root.put("comms", dump(TABLE_COMM, arrayOf("id", "ts", "kind", "direction", "party", "party_name", "content")))
        root.put("calendar", dump(TABLE_CAL, arrayOf("id", "ts", "title", "location", "end_ms", "attendees")))
        root.put("conversations", dump(TABLE_CONV, arrayOf("id", "start_ms", "end_ms", "participants", "source", "status", "context")))
        return root.toString(2)
    }

    private fun dump(table: String, cols: Array<String>): JSONArray {
        val arr = JSONArray()
        db.query(table, cols, null, null, null, null, "id ASC").use { c ->
            while (c.moveToNext()) {
                val o = JSONObject()
                for (i in cols.indices) o.put(cols[i], valueAt(c, i))
                arr.put(o)
            }
        }
        return arr
    }

    private fun valueAt(c: Cursor, i: Int): Any = when (c.getType(i)) {
        Cursor.FIELD_TYPE_INTEGER -> c.getLong(i)
        Cursor.FIELD_TYPE_FLOAT -> c.getDouble(i)
        Cursor.FIELD_TYPE_NULL -> JSONObject.NULL
        else -> c.getString(i)
    }

    fun insertComm(timestampMs: Long, kind: String, direction: String, party: String, partyName: String?, content: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("kind", kind)
            put("direction", direction)
            put("party", party)
            put("party_name", partyName)
            put("content", content)
        }
        return db.insert(TABLE_COMM, null, values)
    }

    fun recentComm(limit: Int): List<CommEvent> {
        val out = ArrayList<CommEvent>()
        db.query(TABLE_COMM, arrayOf("id", "ts", "kind", "direction", "party", "party_name", "content"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(
                    CommEvent(
                        c.getLong(0), c.getLong(1), c.getString(2), c.getString(3),
                        c.getString(4), if (c.isNull(5)) null else c.getString(5), c.getString(6),
                    ),
                )
            }
        }
        return out
    }

    fun insertCalendar(timestampMs: Long, title: String, location: String, endMs: Long, attendees: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("title", title)
            put("location", location)
            put("end_ms", endMs)
            put("attendees", attendees)
        }
        return db.insert(TABLE_CAL, null, values)
    }

    fun recentCalendar(limit: Int): List<CalendarEntry> {
        val out = ArrayList<CalendarEntry>()
        db.query(TABLE_CAL, arrayOf("id", "ts", "title", "location", "end_ms", "attendees"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(CalendarEntry(c.getLong(0), c.getLong(1), c.getString(2), c.getString(3), c.getLong(4), c.getString(5)))
            }
        }
        return out
    }

    /** 대화 자동 연결용: 구간과 겹치는 통화. */
    fun commsBetween(startMs: Long, endMs: Long): List<CommEvent> {
        val out = ArrayList<CommEvent>()
        db.query(TABLE_COMM, arrayOf("id", "ts", "kind", "direction", "party", "party_name", "content"),
            "ts >= ? AND ts <= ?", arrayOf(startMs.toString(), endMs.toString()), null, null, "ts ASC").use { c ->
            while (c.moveToNext()) {
                out.add(CommEvent(c.getLong(0), c.getLong(1), c.getString(2), c.getString(3), c.getString(4), if (c.isNull(5)) null else c.getString(5), c.getString(6)))
            }
        }
        return out
    }

    /** 대화 자동 연결용: 구간과 겹치는 일정(begin < end AND end_ms > start). */
    fun calendarOverlapping(startMs: Long, endMs: Long): List<CalendarEntry> {
        val out = ArrayList<CalendarEntry>()
        db.query(TABLE_CAL, arrayOf("id", "ts", "title", "location", "end_ms", "attendees"),
            "ts <= ? AND end_ms >= ?", arrayOf(endMs.toString(), startMs.toString()), null, null, "ts ASC").use { c ->
            while (c.moveToNext()) {
                out.add(CalendarEntry(c.getLong(0), c.getLong(1), c.getString(2), c.getString(3), c.getLong(4), c.getString(5)))
            }
        }
        return out
    }

    fun insertConversation(startMs: Long, endMs: Long, participants: String, source: String, status: String, context: String): Long {
        val values = ContentValues().apply {
            put("start_ms", startMs)
            put("end_ms", endMs)
            put("participants", participants)
            put("source", source)
            put("status", status)
            put("context", context)
        }
        return db.insert(TABLE_CONV, null, values)
    }

    fun recentConversation(limit: Int): List<Conversation> {
        val out = ArrayList<Conversation>()
        db.query(TABLE_CONV, arrayOf("id", "start_ms", "end_ms", "participants", "source", "status", "context"), null, null, null, null, "start_ms DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(Conversation(c.getLong(0), c.getLong(1), c.getLong(2), c.getString(3), c.getString(4), c.getString(5), c.getString(6)))
            }
        }
        return out
    }

    fun tagConversation(id: Long, participants: String) {
        val values = ContentValues().apply {
            put("participants", participants)
            put("source", "manual")
            put("status", "tagged")
        }
        db.update(TABLE_CONV, values, "id = ?", arrayOf(id.toString()))
    }

    fun insertSummary(timestampMs: Long, periodStartMs: Long, periodEndMs: Long, text: String): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("period_start", periodStartMs)
            put("period_end", periodEndMs)
            put("text", text)
        }
        return db.insert(TABLE_SUMMARY, null, values)
    }

    fun recentSummary(limit: Int): List<Summary> {
        val out = ArrayList<Summary>()
        db.query(TABLE_SUMMARY, arrayOf("id", "ts", "period_start", "period_end", "text"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(Summary(c.getLong(0), c.getLong(1), c.getLong(2), c.getLong(3), c.getString(4)))
            }
        }
        return out
    }

    /** 요약 입력용 묶음 텍스트. 지정 구간의 음성·화면·알림·앱 기록을 시간순 한 줄씩. */
    fun digest(sinceMs: Long, untilMs: Long, maxChars: Int): String {
        val sb = StringBuilder()
        data class Row(val ts: Long, val line: String)
        val rows = ArrayList<Row>()
        fun collect(table: String, cols: Array<String>, render: (android.database.Cursor) -> String) {
            db.query(table, cols, "ts >= ? AND ts < ?", arrayOf(sinceMs.toString(), untilMs.toString()), null, null, "ts ASC").use { c ->
                while (c.moveToNext()) rows.add(Row(c.getLong(0), render(c)))
            }
        }
        collect(TABLE_VOICE, arrayOf("ts", "text")) { "말: ${it.getString(1)}" }
        collect(TABLE_SCREEN, arrayOf("ts", "label", "text")) { "화면(${it.getString(1)}): ${it.getString(2).take(200)}" }
        collect(TABLE_NOTIF, arrayOf("ts", "label", "title", "text")) { "알림(${it.getString(1)}): ${it.getString(2)} ${it.getString(3)}" }
        collect(TABLE_APP, arrayOf("ts", "label")) { "앱 전환: ${it.getString(1)}" }
        rows.sortBy { it.ts }
        for (r in rows) {
            if (sb.length + r.line.length + 1 > maxChars) break
            sb.append(r.line).append('\n')
        }
        return sb.toString().trim()
    }

    /** 증분 내보내기용 새 행(각 테이블에서 id > 마지막 내보낸 id). 줄마다 type 이 붙은 NDJSON. */
    data class NewRows(val lines: List<String>, val newLastIds: Map<String, Long>)

    fun newRows(lastIds: Map<String, Long>, includeSummary: Boolean = true): NewRows {
        val lines = ArrayList<String>()
        val newIds = HashMap(lastIds)
        fun emit(type: String, table: String, cols: Array<String>) {
            val last = lastIds[table] ?: 0L
            db.query(table, cols, "id > ?", arrayOf(last.toString()), null, null, "id ASC").use { c ->
                while (c.moveToNext()) {
                    val o = JSONObject()
                    o.put("type", type)
                    for (i in cols.indices) o.put(cols[i], valueAt(c, i))
                    lines.add(o.toString())
                    val id = c.getLong(0)
                    if (id > (newIds[table] ?: 0L)) newIds[table] = id
                }
            }
        }
        emit("voice", TABLE_VOICE, arrayOf("id", "ts", "text", "speaker"))
        emit("app", TABLE_APP, arrayOf("id", "ts", "pkg", "label"))
        emit("screen", TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text"))
        emit("location", TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy"))
        emit("notification", TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text"))
        if (includeSummary) emit("summary", TABLE_SUMMARY, arrayOf("id", "ts", "period_start", "period_end", "text"))
        emit("comm", TABLE_COMM, arrayOf("id", "ts", "kind", "direction", "party", "party_name", "content"))
        emit("calendar", TABLE_CAL, arrayOf("id", "ts", "title", "location", "end_ms", "attendees"))
        emit("conversation", TABLE_CONV, arrayOf("id", "start_ms", "end_ms", "participants", "source", "status", "context"))
        return NewRows(lines, newIds)
    }

    /** 증분 내보내기가 커서로 쓰는 테이블 이름들. */
    fun tables(): List<String> = listOf(TABLE_VOICE, TABLE_APP, TABLE_SCREEN, TABLE_LOCATION, TABLE_NOTIF, TABLE_SUMMARY, TABLE_COMM, TABLE_CAL, TABLE_CONV)

    /**
     * 보존 정리. 각 원본 테이블에서 ts < [cutoffMs] 이고 이미 내보낸(id <= 해당 테이블의
     * [exportedLastIds]) 행만 지운다. 요약은 건드리지 않는다. 지운 행 수를 반환.
     */
    fun prune(cutoffMs: Long, exportedLastIds: Map<String, Long>): Int {
        var deleted = 0
        for (table in listOf(TABLE_VOICE, TABLE_APP, TABLE_SCREEN, TABLE_LOCATION, TABLE_NOTIF, TABLE_COMM, TABLE_CAL)) {
            val lastExported = exportedLastIds[table] ?: 0L
            deleted += db.delete(table, "ts < ? AND id <= ?", arrayOf(cutoffMs.toString(), lastExported.toString()))
        }
        return deleted
    }

    /** 모든 기록을 지운다(되돌릴 수 없음). */
    fun deleteAll() {
        db.delete(TABLE_VOICE, null, null)
        db.delete(TABLE_APP, null, null)
        db.delete(TABLE_SCREEN, null, null)
        db.delete(TABLE_LOCATION, null, null)
        db.delete(TABLE_NOTIF, null, null)
        db.delete(TABLE_SUMMARY, null, null)
        db.delete(TABLE_COMM, null, null)
        db.delete(TABLE_CAL, null, null)
        db.delete(TABLE_CONV, null, null)
    }

    companion object {
        private const val DB_NAME = "lifelog_enc.db"
        private const val TABLE_VOICE = "voice_segment"
        private const val TABLE_APP = "app_event"
        private const val TABLE_SCREEN = "screen_content"
        private const val TABLE_LOCATION = "location_point"
        private const val TABLE_NOTIF = "notification_event"
        private const val TABLE_SUMMARY = "summary"
        private const val TABLE_COMM = "comm_event"
        private const val TABLE_CAL = "calendar_event"
        private const val TABLE_CONV = "conversation"
    }
}
