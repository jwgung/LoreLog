package com.lorelog.data

import android.content.Context
import android.util.Log
import androidx.work.WorkManager
import com.lorelog.net.EndpointConfig
import java.io.File

/**
 * 폰이 수집만 하게 바꾼 뒤(#67) 예전 설치에 남은 것을 한 번 치운다. 예전에 고른 Gemini·whisper 설정은 따르지 않는다.
 * - 내려받은 온디바이스 whisper 모델 파일(filesDir/whisper)
 * - 암호화 저장소의 Gemini 키·주소·모델(서랍 주소·토큰과 DB 키는 그대로)
 * - 전사 엔진 선택(transcribe)과 폰 요약 켜기(summary) 설정
 * - 하루 한 번 돌던 폰 요약 작업(daily-summary)
 * 요약 표는 [LogStore]가 DB 를 열 때 지운다. 도중에 실패하면 끝났다고 적지 않아 다음 실행에 다시 한다.
 */
object LegacyCleanup {
    private const val TAG = "LegacyCleanup"
    const val VERSION = 1
    private const val PREF = "legacy_cleanup"
    private const val KEY_DONE = "done_version"
    const val WHISPER_DIR = "whisper"
    val OBSOLETE_PREFS = listOf("transcribe", "summary")
    val OBSOLETE_SECRET_KEYS = listOf("llm_base_url", "llm_model", "llm_api_key")
    const val OBSOLETE_WORK = "daily-summary"

    fun needed(doneVersion: Int): Boolean = doneVersion < VERSION

    fun run(context: Context) {
        val app = context.applicationContext
        val state = app.getSharedPreferences(PREF, Context.MODE_PRIVATE)
        if (!needed(state.getInt(KEY_DONE, 0))) return
        try {
            if (!deleteWhisperModels(app.filesDir)) Log.w(TAG, "whisper models not fully deleted")
            OBSOLETE_PREFS.forEach { app.deleteSharedPreferences(it) }
            EndpointConfig.secretPrefs(app).edit().apply { OBSOLETE_SECRET_KEYS.forEach { remove(it) } }.commit()
            WorkManager.getInstance(app).cancelUniqueWork(OBSOLETE_WORK)
            state.edit().putInt(KEY_DONE, VERSION).apply()
        } catch (e: Exception) {
            Log.w(TAG, "cleanup failed, retry next start: ${e.message}")
        }
    }

    /** whisper 모델 폴더를 통째로 지운다(받다 만 .part 포함). 없으면 true. */
    fun deleteWhisperModels(filesDir: File): Boolean {
        val dir = File(filesDir, WHISPER_DIR)
        return !dir.exists() || dir.deleteRecursively()
    }
}
