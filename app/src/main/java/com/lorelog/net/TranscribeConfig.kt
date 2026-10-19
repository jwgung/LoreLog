package com.lorelog.net

import android.content.Context

/**
 * 전사 엔진 선택. 민감하지 않은 설정이라 평문 SharedPreferences 에 둔다.
 * - GEMINI: 요약용 LlmConfig(Gemini) 키를 그대로 재사용해 오디오를 전사(클라우드).
 * - WHISPER: 온디바이스 whisper.cpp. 모델을 다운로드해 기기 안에서 전사(기기 밖 안 나감).
 */
object TranscribeConfig {
    enum class Engine { GEMINI, WHISPER }

    private const val PREF = "transcribe"
    private const val KEY_ENGINE = "engine"
    private const val KEY_MODEL = "whisper_model"

    private fun prefs(c: Context) =
        c.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun engine(c: Context): Engine =
        if (prefs(c).getString(KEY_ENGINE, null) == "whisper") Engine.WHISPER else Engine.GEMINI

    fun setEngine(c: Context, e: Engine) {
        prefs(c).edit().putString(KEY_ENGINE, if (e == Engine.WHISPER) "whisper" else "gemini").apply()
    }

    fun modelId(c: Context): String =
        prefs(c).getString(KEY_MODEL, null) ?: WhisperModels.DEFAULT_ID

    fun setModelId(c: Context, id: String) =
        prefs(c).edit().putString(KEY_MODEL, id).apply()
}
