package com.lifelog.data

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
    }

    fun insertVoice(timestampMs: Long, text: String, speaker: String?): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("text", text)
            put("speaker", speaker)
        }
        return db.insert(TABLE_VOICE, null, values)
    }

    fun recentVoice(limit: Int): List<VoiceSegment> {
        val out = ArrayList<VoiceSegment>()
        db.query(TABLE_VOICE, arrayOf("id", "ts", "text", "speaker"), null, null, null, null, "ts DESC", limit.toString()).use { c ->
            while (c.moveToNext()) {
                out.add(
                    VoiceSegment(
                        id = c.getLong(0),
                        timestampMs = c.getLong(1),
                        text = c.getString(2),
                        speaker = if (c.isNull(3)) null else c.getString(3),
                    ),
                )
            }
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
    fun exportJson(exportedAtMs: Long): String {
        val root = JSONObject()
        root.put("exportedAt", exportedAtMs)
        root.put("voice", dump(TABLE_VOICE, arrayOf("id", "ts", "text", "speaker")))
        root.put("apps", dump(TABLE_APP, arrayOf("id", "ts", "pkg", "label")))
        root.put("screen", dump(TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text")))
        root.put("locations", dump(TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy")))
        root.put("notifications", dump(TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text")))
        return root.toString(2)
    }

    private fun dump(table: String, cols: Array<String>): JSONArray {
        val arr = JSONArray()
        db.query(table, cols, null, null, null, null, "ts ASC").use { c ->
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

    /** 증분 내보내기용 새 행(각 테이블에서 id > 마지막 내보낸 id). 줄마다 type 이 붙은 NDJSON. */
    data class NewRows(val lines: List<String>, val newLastIds: Map<String, Long>)

    fun newRows(lastIds: Map<String, Long>): NewRows {
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
        return NewRows(lines, newIds)
    }

    /** 증분 내보내기가 커서로 쓰는 테이블 이름들. */
    fun tables(): List<String> = listOf(TABLE_VOICE, TABLE_APP, TABLE_SCREEN, TABLE_LOCATION, TABLE_NOTIF)

    /** 모든 기록을 지운다(되돌릴 수 없음). */
    fun deleteAll() {
        db.delete(TABLE_VOICE, null, null)
        db.delete(TABLE_APP, null, null)
        db.delete(TABLE_SCREEN, null, null)
        db.delete(TABLE_LOCATION, null, null)
        db.delete(TABLE_NOTIF, null, null)
    }

    companion object {
        private const val DB_NAME = "lifelog_enc.db"
        private const val TABLE_VOICE = "voice_segment"
        private const val TABLE_APP = "app_event"
        private const val TABLE_SCREEN = "screen_content"
        private const val TABLE_LOCATION = "location_point"
        private const val TABLE_NOTIF = "notification_event"
    }
}
