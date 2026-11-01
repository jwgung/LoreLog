package com.lorelog.work

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.work.NetworkType

/**
 * "와이파이에 연결됐을 때만 보내기". 켜면 기록 전송([PushWorker])과 음성 전송(녹음 직후, 밀린 음성
 * [PendingVoiceWorker], 통화 녹음 [CallRecordingWorker])이 데이터 요금이 없는 망(UNMETERED)에서만 돈다.
 * 기본은 꺼짐(어느 망이든 연결되면 보낸다). 사용자가 직접 누르는 것("지금 전송", "밀린 음성 전사", 말하기)은
 * 이 설정과 상관없이 바로 보낸다.
 */
object PushPrefs {
    private const val PREF = "push"
    private const val KEY_WIFI_ONLY = "wifi_only"

    private fun prefs(c: Context) = c.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun wifiOnly(c: Context): Boolean = prefs(c).getBoolean(KEY_WIFI_ONLY, false)

    /** 저장하고 주기 작업들의 조건을 바로 바꾼다. */
    fun setWifiOnly(c: Context, v: Boolean) {
        prefs(c).edit().putBoolean(KEY_WIFI_ONLY, v).apply()
        PushWorker.schedule(c)
        PendingVoiceWorker.schedule(c)
        CallRecordingWorker.schedule(c)
    }

    fun networkType(wifiOnly: Boolean): NetworkType = if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED

    /** 사용자가 누르지 않은 전송(자동)을 지금 해도 되나. */
    fun mayAutoSend(wifiOnly: Boolean, unmetered: Boolean): Boolean = !wifiOnly || unmetered

    /** [mayAutoSend]를 지금 망으로. */
    fun mayAutoSend(c: Context): Boolean {
        if (!wifiOnly(c)) return true
        val cm = c.applicationContext.getSystemService(ConnectivityManager::class.java) ?: return false
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
    }
}
