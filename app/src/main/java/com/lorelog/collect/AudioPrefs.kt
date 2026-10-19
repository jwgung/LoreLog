package com.lorelog.collect

import android.content.Context

/** 녹음 입력 설정. 블루투스 이어폰 마이크로 녹음할지 등. */
object AudioPrefs {
    private const val PREF = "audio"
    private const val KEY_BT = "bt_mic"

    private fun p(context: Context) =
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun btMic(context: Context): Boolean = p(context).getBoolean(KEY_BT, false)
    fun setBtMic(context: Context, v: Boolean) = p(context).edit().putBoolean(KEY_BT, v).apply()
}
