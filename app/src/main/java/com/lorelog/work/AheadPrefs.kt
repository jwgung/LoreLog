package com.lorelog.work

import android.content.Context

/**
 * 비서가 먼저 말한 것의 기록(#60): 이미 말한 일정, 조직별로 밀어서 지운 횟수, 멈춘 조직, 서랍이 꺼져 있어
 * 아직 못 보낸 버튼 정정.
 */
object AheadPrefs {
    private const val PREF = "ahead"
    private const val KEY_SAID = "said"
    private const val KEY_SWIPES = "swipes"
    private const val KEY_STOPPED = "stopped"
    private const val KEY_PENDING = "pending_corrections"
    private const val SEP = "\u0001"
    private const val DAY_MS = 86_400_000L

    private fun p(c: Context) = c.applicationContext.getSharedPreferences(PREF, Context.MODE_PRIVATE)
    private fun set(c: Context, k: String): Set<String> = p(c).getStringSet(k, emptySet()).orEmpty().toSet()

    /** 이미 말한 일정 열쇠([AheadRules.saidKey]). 하루 넘게 지난 일정은 잊는다(다시 올 수 없다). */
    fun said(c: Context, now: Long = System.currentTimeMillis()): Set<String> =
        set(c, KEY_SAID).filter { (it.substringBefore('|').toLongOrNull() ?: 0L) >= now - DAY_MS }.toSet()

    @Synchronized
    fun markSaid(c: Context, key: String) {
        p(c).edit().putStringSet(KEY_SAID, said(c) + key).apply()
    }

    fun stopped(c: Context): Set<String> = set(c, KEY_STOPPED)

    /** 밀어서 지운 알림 하나. 같은 조직에서 [AheadRules.STOP_AFTER]번이면 그 조직을 멈춘다. */
    @Synchronized
    fun swiped(c: Context, stop: String) {
        if (stop.isBlank()) return
        val counts = set(c, KEY_SWIPES).mapNotNull { e ->
            e.split(SEP).takeIf { it.size == 2 }?.let { (k, n) -> n.toIntOrNull()?.let { k to it } }
        }.toMap()
        val (next, stopNow) = AheadRules.swiped(counts, stop)
        val edit = p(c).edit().putStringSet(KEY_SWIPES, next.map { (k, n) -> "$k$SEP$n" }.toSet())
        if (stopNow) edit.putStringSet(KEY_STOPPED, stopped(c) + stop)
        edit.apply()
    }

    /** 보내지 못한 정정 (wrong, right, ref). */
    fun pending(c: Context): List<Triple<String, String, String>> = set(c, KEY_PENDING).mapNotNull { e ->
        e.split(SEP).takeIf { it.size == 3 }?.let { Triple(it[0], it[1], it[2]) }
    }

    @Synchronized
    fun addPending(c: Context, wrong: String, right: String, ref: String) {
        p(c).edit().putStringSet(KEY_PENDING, set(c, KEY_PENDING) + listOf(wrong, right, ref).joinToString(SEP)).apply()
    }

    @Synchronized
    fun removePending(c: Context, wrong: String, right: String, ref: String) {
        p(c).edit().putStringSet(KEY_PENDING, set(c, KEY_PENDING) - listOf(wrong, right, ref).joinToString(SEP)).apply()
    }
}
