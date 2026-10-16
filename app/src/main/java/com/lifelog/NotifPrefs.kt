package com.lifelog

import android.content.Context

/** 알림·표시 설정. 기본: 알림 항상 유지(숨기기 off), 플로팅 배지 off. */
object NotifPrefs {
    private const val PREF = "notif"
    private const val KEY_HIDE = "hide_notif"
    private const val KEY_OVERLAY = "overlay_badge"

    private fun p(context: Context) =
        context.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)

    fun hide(context: Context): Boolean = p(context).getBoolean(KEY_HIDE, false)
    fun setHide(context: Context, v: Boolean) = p(context).edit().putBoolean(KEY_HIDE, v).apply()

    fun overlay(context: Context): Boolean = p(context).getBoolean(KEY_OVERLAY, false)
    fun setOverlay(context: Context, v: Boolean) = p(context).edit().putBoolean(KEY_OVERLAY, v).apply()
}
