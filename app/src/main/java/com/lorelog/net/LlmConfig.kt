package com.lorelog.net

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * 요약용 클라우드 LLM 설정. OpenAI 호환(/chat/completions) 방식이라 URL·모델·키만 바꾸면
 * Gemini(클라우드)·Groq·OpenAI 등을 쓸 수 있다. 키는 민감하므로 암호화 저장.
 */
object LlmConfig {
    private const val PREF = "lifelog_secret"
    private const val KEY_URL = "llm_base_url"
    private const val KEY_MODEL = "llm_model"
    private const val KEY_API = "llm_api_key"

    const val DEFAULT_URL = "https://generativelanguage.googleapis.com/v1beta/openai"
    const val DEFAULT_MODEL = "gemini-3.5-flash-lite"

    /** 서비스 종료된 모델 ID 접두어. 저장값이 이에 해당하면 기본 모델로 대체한다(2.0 Flash 는 2026-06-01 종료). */
    private val RETIRED_PREFIXES = listOf("gemini-2.0-", "gemini-1.5-", "gemini-1.0-")

    fun isRetired(model: String): Boolean = RETIRED_PREFIXES.any { model.trim().startsWith(it) }

    @Volatile private var cached: SharedPreferences? = null

    /** 암호화 프리퍼런스는 여는 비용이 커서(마스터키) 한 번 열어 재사용한다. 설정 화면이 글자마다 저장해도 가볍다. */
    private fun prefs(context: Context): SharedPreferences = cached ?: synchronized(this) {
        cached ?: EncryptedSharedPreferences.create(
            context.applicationContext,
            PREF,
            MasterKey.Builder(context.applicationContext)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build(),
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        ).also { cached = it }
    }

    /** 모델 id 정규화: 목록 API 가 주는 "models/" 접두어 제거. */
    fun normalizeModel(model: String): String = model.trim().removePrefix("models/")

    fun baseUrl(context: Context): String = prefs(context).getString(KEY_URL, null)?.takeIf { it.isNotBlank() } ?: DEFAULT_URL
    fun model(context: Context): String =
        prefs(context).getString(KEY_MODEL, null)?.let { normalizeModel(it) }?.takeIf { it.isNotBlank() && !isRetired(it) } ?: DEFAULT_MODEL
    fun apiKey(context: Context): String? = prefs(context).getString(KEY_API, null)?.takeIf { it.isNotBlank() }
    fun isConfigured(context: Context): Boolean = apiKey(context) != null

    fun set(context: Context, baseUrl: String, model: String, apiKey: String) {
        prefs(context).edit()
            .putString(KEY_URL, baseUrl.trim())
            .putString(KEY_MODEL, normalizeModel(model))
            .putString(KEY_API, apiKey.trim())
            .apply()
    }

    /** URL·키만 저장(설정 화면에서 입력 즉시 호출). */
    fun setApi(context: Context, baseUrl: String, apiKey: String) {
        prefs(context).edit().putString(KEY_URL, baseUrl.trim()).putString(KEY_API, apiKey.trim()).apply()
    }

    /** 요약 모델만 저장. */
    fun setModel(context: Context, model: String) {
        prefs(context).edit().putString(KEY_MODEL, normalizeModel(model)).apply()
    }
}
