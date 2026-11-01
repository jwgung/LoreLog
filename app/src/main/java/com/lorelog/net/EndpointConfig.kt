package com.lorelog.net

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/** 금고 연결 설정(기록을 보낼 URL·토큰). 토큰은 민감하므로 암호화 저장한다. */
object EndpointConfig {
    private const val PREF = "lifelog_secret"
    private const val KEY_URL = "endpoint_url"
    private const val KEY_TOKEN = "endpoint_token"

    /** 금고 주소·토큰과 DB 키가 함께 있는 암호화 저장소. 지난 설정을 치울 때([com.lorelog.data.LegacyCleanup])도 쓴다. */
    internal fun secretPrefs(context: Context) = EncryptedSharedPreferences.create(
        context.applicationContext,
        PREF,
        MasterKey.Builder(context.applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    fun url(context: Context): String? = secretPrefs(context).getString(KEY_URL, null)?.takeIf { it.isNotBlank() }
    fun token(context: Context): String? = secretPrefs(context).getString(KEY_TOKEN, null)?.takeIf { it.isNotBlank() }
    fun isConfigured(context: Context): Boolean = url(context) != null

    fun set(context: Context, url: String, token: String) {
        secretPrefs(context).edit()
            .putString(KEY_URL, url.trim())
            .putString(KEY_TOKEN, token.trim())
            .apply()
    }
}
