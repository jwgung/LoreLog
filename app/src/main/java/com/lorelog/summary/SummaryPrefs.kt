package com.lorelog.summary

import android.content.Context

/**
 * 요약 on/off. 요약은 선택 기능이고 기본은 꺼짐.
 * 이유: 지식그래프로는 원본을 그대로 넘기고(폰=충실한 소스), 요약은 KG 쪽에서 하는 게 낫다.
 * 켜면 (1) 주기 요약 워커가 돌아 앱에서 읽을 요약을 쌓고 (2) 내보내기에 summary 가 포함된다.
 */
object SummaryPrefs {
    private const val PREF = "summary"
    private const val KEY = "enabled"
    private fun prefs(c: Context) = c.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)
    fun enabled(c: Context): Boolean = prefs(c).getBoolean(KEY, false)
    fun setEnabled(c: Context, v: Boolean) = prefs(c).edit().putBoolean(KEY, v).apply()
}
