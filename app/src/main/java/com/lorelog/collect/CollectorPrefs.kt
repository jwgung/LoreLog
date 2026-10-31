package com.lorelog.collect

import android.content.Context

/** 수집 항목별 on/off. 리소스를 아끼려면 필요 없는 항목(특히 음성·화면)을 끈다. 기본은 모두 켬. */
object CollectorPrefs {
    private const val PREF = "collectors"

    const val VOICE = "voice"
    const val SCREEN = "screen"
    const val LOCATION = "location"
    const val COMMS = "comms"
    const val CALENDAR = "calendar"
    const val APPS = "apps"
    const val CONTACTS = "contacts"

    val ALL = listOf(VOICE to "음성", SCREEN to "화면 내용", APPS to "앱 사용", LOCATION to "위치", COMMS to "통화·문자", CALENDAR to "캘린더", CONTACTS to "주소록(이름·메일·전화)")

    fun enabled(context: Context, key: String): Boolean =
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE).getBoolean(key, true)

    fun set(context: Context, key: String, value: Boolean) {
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)
            .edit().putBoolean(key, value).apply()
    }
}
