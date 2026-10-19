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
 * - 제외 목록(은행·결제 등) 앱과 상시 알림(포그라운드 서비스 알림)은 건너뛴다.
 * 사용자가 설정 > 알림 접근에서 직접 켜야 동작한다.
 */
class NotificationService : NotificationListenerService() {

    private val labelCache = HashMap<String, String>()

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        sbn ?: return
        if (!LoggerService.isRunning.value) return

        val pkg = sbn.packageName ?: return
        if (pkg == packageName) return
        if (SensitiveApps.isExcluded(this, pkg)) return

        val notification = sbn.notification ?: return
        if (notification.flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return

        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty().trim()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty().trim()
        if (title.isEmpty() && text.isEmpty()) return

        LogRepository.addNotification(pkg, label(pkg), title, text, System.currentTimeMillis())
    }

    private fun label(pkg: String): String = labelCache.getOrPut(pkg) {
        try {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
        } catch (e: PackageManager.NameNotFoundException) {
            pkg
        }
    }
}
