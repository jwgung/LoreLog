package com.lorelog.net

import android.content.Context
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
    const val DEFAULT_MODEL = "gemini-2.0-flash"

    private fun prefs(context: Context) = EncryptedSharedPreferences.create(
        context.applicationContext,
        PREF,
        MasterKey.Builder(context.applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun baseUrl(context: Context): String = prefs(context).getString(KEY_URL, null)?.takeIf { it.isNotBlank() } ?: DEFAULT_URL
    fun model(context: Context): String = prefs(context).getString(KEY_MODEL, null)?.takeIf { it.isNotBlank() } ?: DEFAULT_MODEL
    fun apiKey(context: Context): String? = prefs(context).getString(KEY_API, null)?.takeIf { it.isNotBlank() }
    fun isConfigured(context: Context): Boolean = apiKey(context) != null

    fun set(context: Context, baseUrl: String, model: String, apiKey: String) {
        prefs(context).edit()
            .putString(KEY_URL, baseUrl.trim())
            .putString(KEY_MODEL, model.trim())
            .putString(KEY_API, apiKey.trim())
            .apply()
    }
}
