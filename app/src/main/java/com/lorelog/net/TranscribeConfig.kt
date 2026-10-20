package com.lorelog.net

import android.content.Context

/**
 * 전사 엔진 선택. 민감하지 않은 설정이라 평문 SharedPreferences 에 둔다.
 * - GEMINI: 요약용 LlmConfig(Gemini) 키·URL 을 재사용하되 모델은 따로 고른다(클라우드).
 *   받아쓰기는 가벼워 Flash-Lite 로 충분하고, 요약 모델과 분리해야 한도를 따로 쓸 수 있다.
 * - WHISPER: 온디바이스 whisper.cpp. 모델을 다운로드해 기기 안에서 전사(기기 밖 안 나감).
 */
object TranscribeConfig {
    enum class Engine { GEMINI, WHISPER }

    private const val PREF = "transcribe"
    private const val KEY_ENGINE = "engine"
    private const val KEY_MODEL = "whisper_model"
    private const val KEY_GEMINI_MODEL = "gemini_model"

    private const val KEY_FALLBACK = "gemini_fallback"

    /** 전사용 Gemini 기본 모델. 무료 티어·가장 저렴. */
    const val DEFAULT_GEMINI_MODEL = "gemini-3.5-flash-lite"

    /**
     * 일일 한도 폴백 후보(순서대로). 무료 한도가 모델당 따로 잡히므로 주 모델이 바닥나면 다음으로 넘어간다.
     * 2026-10-20 실측(회의 녹음 15초, thinking off): 3.5-flash 2.5초 · 3.6-flash 2.7초 · 3-flash-preview 2.2초 ·
     * 3.1-flash-lite 5초. 3.7/3.8-flash 는 thinking 을 꺼도 30~56초라 제외. Pro/Omni 는 무료 한도 0 이라 제외.
     * 3.5-transcribe 는 generateContent 로는 빈 응답이라 제외.
     */
    val GEMINI_FALLBACKS = listOf("gemini-3.5-flash", "gemini-3.6-flash", "gemini-3.1-flash-lite", "gemini-3-flash-preview")

    fun fallbackEnabled(c: Context): Boolean = prefs(c).getBoolean(KEY_FALLBACK, true)
    fun setFallbackEnabled(c: Context, on: Boolean) = prefs(c).edit().putBoolean(KEY_FALLBACK, on).apply()

    /** 실제 시도 순서: 선택한 모델 + (켜져 있으면) 폴백들. */
    fun geminiCandidates(c: Context): List<String> {
        val primary = geminiModel(c)
        return if (fallbackEnabled(c)) listOf(primary) + GEMINI_FALLBACKS.filter { it != primary } else listOf(primary)
    }

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

    /** 전사용 Gemini 모델. 비어 있거나 종료된 모델이면 기본값. */
    fun geminiModel(c: Context): String =
        prefs(c).getString(KEY_GEMINI_MODEL, null)?.trim()?.takeIf { it.isNotEmpty() && !LlmConfig.isRetired(it) }
            ?: DEFAULT_GEMINI_MODEL

    fun setGeminiModel(c: Context, id: String) =
        prefs(c).edit().putString(KEY_GEMINI_MODEL, LlmConfig.normalizeModel(id)).apply()
}
