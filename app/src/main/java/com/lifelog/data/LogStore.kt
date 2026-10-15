package com.lifelog.data

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import org.json.JSONArray
import org.json.JSONObject

/**
 * 기기 로컬 SQLite 저장. 의존성 없이 프레임워크만 쓴다.
 * 7단계에서 SQLCipher 암호화 또는 Room 으로 옮길 수 있게 접근을 이 클래스로 모은다.
 */
class LogStore(context: Context) : SQLiteOpenHelper(context.applicationContext, DB_NAME, null, DB_VERSION) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE_VOICE (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                text TEXT NOT NULL,
                speaker TEXT
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_voice_ts ON $TABLE_VOICE(ts)")
        createAppTable(db)
        createScreenTable(db)
        createLocationTable(db)
        createNotificationTable(db)
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) createAppTable(db)
        if (oldVersion < 3) createScreenTable(db)
        if (oldVersion < 4) createLocationTable(db)
        if (oldVersion < 5) createNotificationTable(db)
    }

    private fun createAppTable(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE_APP (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                pkg TEXT NOT NULL,
                label TEXT NOT NULL
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_app_ts ON $TABLE_APP(ts)")
    }

    private fun createScreenTable(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE_SCREEN (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                pkg TEXT NOT NULL,
                label TEXT NOT NULL,
                text TEXT NOT NULL
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_screen_ts ON $TABLE_SCREEN(ts)")
    }

    private fun createLocationTable(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE_LOCATION (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                lat REAL NOT NULL,
                lon REAL NOT NULL,
                accuracy REAL NOT NULL
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_location_ts ON $TABLE_LOCATION(ts)")
    }

    private fun createNotificationTable(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE $TABLE_NOTIF (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                pkg TEXT NOT NULL,
                label TEXT NOT NULL,
                title TEXT NOT NULL,
                text TEXT NOT NULL
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_notif_ts ON $TABLE_NOTIF(ts)")
    }

    fun insertVoice(timestampMs: Long, text: String, speaker: String?): Long {
        val values = ContentValues().apply {
            put("ts", timestampMs)
            put("text", text)
            put("speaker", speaker)
        }
        return writableDatabase.insert(TABLE_VOICE, null, values)
    }

    fun recentVoice(limit: Int): List<VoiceSegment> {
        val out = ArrayList<VoiceSegment>()
        readableDatabase.query(
            TABLE_VOICE, arrayOf("id", "ts", "text", "speaker"),
            null, null, null, null, "ts DESC", limit.toString(),
        ).use { c ->
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
        return writableDatabase.insert(TABLE_APP, null, values)
    }

    fun recentApp(limit: Int): List<AppEvent> {
        val out = ArrayList<AppEvent>()
        readableDatabase.query(
            TABLE_APP, arrayOf("id", "ts", "pkg", "label"),
            null, null, null, null, "ts DESC", limit.toString(),
        ).use { c ->
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
        return writableDatabase.insert(TABLE_SCREEN, null, values)
    }

    fun recentScreen(limit: Int): List<ScreenContentEvent> {
        val out = ArrayList<ScreenContentEvent>()
        readableDatabase.query(
            TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text"),
            null, null, null, null, "ts DESC", limit.toString(),
        ).use { c ->
            while (c.moveToNext()) {
                out.add(
                    ScreenContentEvent(
                        c.getLong(0), c.getLong(1), c.getString(2), c.getString(3), c.getString(4),
                    ),
                )
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
        return writableDatabase.insert(TABLE_LOCATION, null, values)
    }

    fun recentLocation(limit: Int): List<LocationPoint> {
        val out = ArrayList<LocationPoint>()
        readableDatabase.query(
            TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy"),
            null, null, null, null, "ts DESC", limit.toString(),
        ).use { c ->
            while (c.moveToNext()) {
                out.add(
                    LocationPoint(c.getLong(0), c.getLong(1), c.getDouble(2), c.getDouble(3), c.getFloat(4)),
                )
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
        return writableDatabase.insert(TABLE_NOTIF, null, values)
    }

    fun recentNotification(limit: Int): List<NotificationEvent> {
        val out = ArrayList<NotificationEvent>()
        readableDatabase.query(
            TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text"),
            null, null, null, null, "ts DESC", limit.toString(),
        ).use { c ->
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
        val db = readableDatabase
        val root = JSONObject()
        root.put("exportedAt", exportedAtMs)
        root.put("voice", dump(db, TABLE_VOICE, arrayOf("id", "ts", "text", "speaker")))
        root.put("apps", dump(db, TABLE_APP, arrayOf("id", "ts", "pkg", "label")))
        root.put("screen", dump(db, TABLE_SCREEN, arrayOf("id", "ts", "pkg", "label", "text")))
        root.put("locations", dump(db, TABLE_LOCATION, arrayOf("id", "ts", "lat", "lon", "accuracy")))
        root.put("notifications", dump(db, TABLE_NOTIF, arrayOf("id", "ts", "pkg", "label", "title", "text")))
        return root.toString(2)
    }

    private fun dump(db: SQLiteDatabase, table: String, cols: Array<String>): JSONArray {
        val arr = JSONArray()
        db.query(table, cols, null, null, null, null, "ts ASC").use { c ->
            while (c.moveToNext()) {
                val o = JSONObject()
                for (i in cols.indices) {
                    o.put(cols[i], valueAt(c, i))
                }
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

    /** 모든 기록을 지운다(되돌릴 수 없음). */
    fun deleteAll() {
        writableDatabase.apply {
            delete(TABLE_VOICE, null, null)
            delete(TABLE_APP, null, null)
            delete(TABLE_SCREEN, null, null)
            delete(TABLE_LOCATION, null, null)
            delete(TABLE_NOTIF, null, null)
        }
    }

    companion object {
        private const val DB_NAME = "lifelog.db"
        private const val DB_VERSION = 5
        private const val TABLE_VOICE = "voice_segment"
        private const val TABLE_APP = "app_event"
        private const val TABLE_SCREEN = "screen_content"
        private const val TABLE_LOCATION = "location_point"
        private const val TABLE_NOTIF = "notification_event"
    }
}
