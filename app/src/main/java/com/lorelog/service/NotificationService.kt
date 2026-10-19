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
 * - 제외 목록(은행·결제 등)·포그라운드 서비스 알림은 건너뛴다.
 * - 상시(ongoing) 알림(충전·음악·내비처럼 자주 갱신)은 버리지 않고 제목 기준으로 길게 스로틀해
 *   도배를 막는다(= 정보는 남되 가끔만). 일반 알림은 동일 내용 반복만 중복 제거.
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
        if (SensitiveApps.isExcluded(this, pkg)) return

        val notification = sbn.notification ?: return
        val flags = notification.flags
        if (flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return
        val ongoing = flags and Notification.FLAG_ONGOING_EVENT != 0

        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty().trim()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty().trim()
        if (title.isEmpty() && text.isEmpty()) return

        val now = System.currentTimeMillis()
        // 상시 알림: 제목만으로 묶어 길게 스로틀(충전 %처럼 본문만 바뀌며 쏟아지는 것 방지).
        // 일반 알림: 동일 내용(제목+본문) 짧게 중복 제거.
        val key = if (ongoing) "$pkg|$title" else "$pkg|$title|$text"
        val window = if (ongoing) ONGOING_THROTTLE_MS else DEDUPE_MS
        val last = recent[key]
        if (last != null && now - last < window) return
        recent[key] = now
        if (recent.size > 200) {
            val cutoff = now - ONGOING_THROTTLE_MS
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
        const val DEDUPE_MS = 60_000L          // 일반 알림 동일 내용 중복 제거 창
        const val ONGOING_THROTTLE_MS = 600_000L // 상시 알림(충전 등) 제목 기준 10분에 1회
    }
}
