package com.lorelog.data

import android.content.Context
import android.util.Base64
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.security.SecureRandom

/**
 * DB 암호화 키. 랜덤 32바이트를 만들어 Android Keystore 로 보호되는
 * EncryptedSharedPreferences 에 보관한다. 기기를 벗어나지 않는다.
 */
object DbKey {
    private const val PREF = "lifelog_secret"
    private const val KEY = "db_key"

    fun get(context: Context): ByteArray {
        val prefs = EncryptedSharedPreferences.create(
            context.applicationContext,
            PREF,
            MasterKey.Builder(context.applicationContext)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build(),
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
        prefs.getString(KEY, null)?.let { return Base64.decode(it, Base64.NO_WRAP) }
        val key = ByteArray(32).also { SecureRandom().nextBytes(it) }
        prefs.edit().putString(KEY, Base64.encodeToString(key, Base64.NO_WRAP)).apply()
        return key
    }
}
