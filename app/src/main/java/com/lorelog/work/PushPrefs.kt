package com.lorelog.work

import android.content.Context
import androidx.work.NetworkType

/**
 * 주기 전송([PushWorker])의 "와이파이에 연결됐을 때만 보내기". 켜면 데이터 요금이 없는 망(UNMETERED)에서만 보낸다.
 * 기본은 꺼짐(어느 망이든 연결되면 보낸다). 사용자가 누르는 "지금 전송"은 이 설정과 상관없이 바로 보낸다.
 */
object PushPrefs {
    private const val PREF = "push"
    private const val KEY_WIFI_ONLY = "wifi_only"

    private fun prefs(c: Context) = c.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun wifiOnly(c: Context): Boolean = prefs(c).getBoolean(KEY_WIFI_ONLY, false)

    /** 저장하고 주기 전송의 조건을 바로 바꾼다. */
    fun setWifiOnly(c: Context, v: Boolean) {
        prefs(c).edit().putBoolean(KEY_WIFI_ONLY, v).apply()
        PushWorker.schedule(c)
    }

    fun networkType(wifiOnly: Boolean): NetworkType = if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED
}
