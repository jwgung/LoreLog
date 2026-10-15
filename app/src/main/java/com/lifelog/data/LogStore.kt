package com.lifelog.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

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
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) createAppTable(db)
        if (oldVersion < 3) createScreenTable(db)
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

    companion object {
        private const val DB_NAME = "lifelog.db"
        private const val DB_VERSION = 3
        private const val TABLE_VOICE = "voice_segment"
        private const val TABLE_APP = "app_event"
        private const val TABLE_SCREEN = "screen_content"
    }
}
