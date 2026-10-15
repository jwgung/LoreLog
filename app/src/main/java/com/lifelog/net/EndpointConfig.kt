package com.lifelog.net

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/** 내보낼 HTTP 엔드포인트 설정(URL·토큰). 토큰은 민감하므로 암호화 저장한다. */
object EndpointConfig {
    private const val PREF = "lifelog_secret"
    private const val KEY_URL = "endpoint_url"
    private const val KEY_TOKEN = "endpoint_token"

    private fun prefs(context: Context) = EncryptedSharedPreferences.create(
        context.applicationContext,
        PREF,
        MasterKey.Builder(context.applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun url(context: Context): String? = prefs(context).getString(KEY_URL, null)?.takeIf { it.isNotBlank() }
    fun token(context: Context): String? = prefs(context).getString(KEY_TOKEN, null)?.takeIf { it.isNotBlank() }
    fun isConfigured(context: Context): Boolean = url(context) != null

    fun set(context: Context, url: String, token: String) {
        prefs(context).edit()
            .putString(KEY_URL, url.trim())
            .putString(KEY_TOKEN, token.trim())
            .apply()
    }
}
