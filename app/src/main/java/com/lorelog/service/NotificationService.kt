package com.lorelog.service

import android.app.Notification
import android.content.pm.PackageManager
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import com.lorelog.LoggerService
import com.lorelog.collect.SensitiveApps
import com.lorelog.data.LogRepository

/**
 * 받은 알림 수집. NotificationListenerService 로 올라오는 알림의 제목·본문을 기록한다.
 * - '기록 중'일 때만 수집.
 * - 제외 목록(은행·결제 등)·시스템 UI(충전·배터리 등)·상시(ongoing)·포그라운드 서비스 알림은 건너뛴다.
 * - 짧은 시간 내 같은 알림 반복은 중복 제거.
 * 사용자가 설정 > 알림 접근에서 직접 켜야 동작한다.
 */
class NotificationService : NotificationListenerService() {

    private val labelCache = HashMap<String, String>()
    private val recent = HashMap<String, Long>()

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        sbn ?: return
        if (!LoggerService.isRunning.value) return

        val pkg = sbn.packageName ?: return
        if (pkg == packageName) return
        if (pkg in SYSTEM_PKGS) return
        if (SensitiveApps.isExcluded(this, pkg)) return

        val notification = sbn.notification ?: return
        val flags = notification.flags
        if (flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return
        // 상시 알림(충전·음악·내비 등)은 자주 갱신돼 스팸이라 제외
        if (flags and Notification.FLAG_ONGOING_EVENT != 0) return

        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty().trim()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty().trim()
        if (title.isEmpty() && text.isEmpty()) return

        // 짧은 시간 내 동일 알림(제목+본문) 반복 제거
        val now = System.currentTimeMillis()
        val key = "$pkg|$title|$text"
        val last = recent[key]
        if (last != null && now - last < DEDUPE_MS) return
        recent[key] = now
        if (recent.size > 200) {
            val cutoff = now - DEDUPE_MS
            recent.entries.removeAll { it.value < cutoff }
        }

        LogRepository.addNotification(pkg, label(pkg), title, text, now)
    }

    private fun label(pkg: String): String = labelCache.getOrPut(pkg) {
        try {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
        } catch (e: PackageManager.NameNotFoundException) {
            pkg
        }
    }

    private companion object {
        val SYSTEM_PKGS = setOf("com.android.systemui", "android")
        const val DEDUPE_MS = 60_000L
    }
}
