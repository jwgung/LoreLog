package com.lorelog

import android.content.Context

/** 하루 일기 알림(비서가 먼저 말을 거는 자리) 설정. 기본: 켬, 저녁 9시. */
object DiaryPrefs {
    private const val PREF = "diary"
    private const val KEY_ENABLED = "reminder_enabled"
    private const val KEY_HOUR = "reminder_hour"
    private const val KEY_MINUTE = "reminder_minute"

    private fun p(context: Context) =
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun enabled(context: Context): Boolean = p(context).getBoolean(KEY_ENABLED, true)
    fun setEnabled(context: Context, v: Boolean) = p(context).edit().putBoolean(KEY_ENABLED, v).apply()

    fun hour(context: Context): Int = p(context).getInt(KEY_HOUR, 21)
    fun minute(context: Context): Int = p(context).getInt(KEY_MINUTE, 0)
    fun setTime(context: Context, hour: Int, minute: Int) =
        p(context).edit().putInt(KEY_HOUR, hour).putInt(KEY_MINUTE, minute).apply()

    /**
     * 일기의 "오늘". 새벽 4시 전에는 전날로 본다(밤늦게 쓰다 자정이 지나도 어제 하루를 마무리할 수 있게).
     */
    fun diaryDay(nowMs: Long = System.currentTimeMillis()): String =
        java.text.SimpleDateFormat("yyyy-MM-dd", java.util.Locale.US).format(java.util.Date(nowMs - 4 * 3_600_000L))
}
