package com.lifelog.net

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * 녹음+클라우드 음성 전사 설정. OpenAI 호환 /audio/transcriptions 방식.
 * 기본은 Groq Whisper(무료 티어). 키는 암호화 저장.
 * [cloudMode] 가 켜져 있고 키가 있으면 음성 수집을 '녹음+클라우드'로 한다(아니면 구글 실시간).
 */
object SttConfig {
    private const val PREF = "lifelog_secret"
    private const val KEY_URL = "stt_base_url"
    private const val KEY_MODEL = "stt_model"
    private const val KEY_API = "stt_api_key"
    private const val KEY_MODE = "stt_cloud_mode"

    const val DEFAULT_URL = "https://api.groq.com/openai/v1"
    const val DEFAULT_MODEL = "whisper-large-v3-turbo"

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
    fun cloudMode(context: Context): Boolean = prefs(context).getBoolean(KEY_MODE, false)
    fun isConfigured(context: Context): Boolean = apiKey(context) != null

    /** 녹음+클라우드로 수집할지. 모드가 켜져 있고 키가 있어야 한다. */
    fun useCloud(context: Context): Boolean = cloudMode(context) && isConfigured(context)

    fun set(context: Context, baseUrl: String, model: String, apiKey: String, cloudMode: Boolean) {
        prefs(context).edit()
            .putString(KEY_URL, baseUrl.trim())
            .putString(KEY_MODEL, model.trim())
            .putString(KEY_API, apiKey.trim())
            .putBoolean(KEY_MODE, cloudMode)
            .apply()
    }
}
