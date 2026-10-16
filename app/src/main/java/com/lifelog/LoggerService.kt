package com.lifelog

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.Manifest
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.lifelog.collect.AppUsageCollector
import com.lifelog.collect.CollectorPrefs
import com.lifelog.collect.CalendarCollector
import com.lifelog.collect.CommsCollector
import com.lifelog.collect.LocationCollector
import com.lifelog.collect.VoiceCollector
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * 상시 기록 서비스. 기록 중에는 지울 수 없는 포그라운드 알림을 띄우고 수집기를 돌린다.
 * 멈췄을 때도 '기록 시작' 버튼이 달린 상시 알림을 남겨, 알림창에서 바로 켜고 끌 수 있다.
 */
class LoggerService : Service() {

    private var voice: VoiceCollector? = null
    private var appUsage: AppUsageCollector? = null
    private var location: LocationCollector? = null
    private var comms: CommsCollector? = null
    private var calendar: CalendarCollector? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopLogging()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_REAPPLY) {
            if (_isRunning.value) { stopCollectors(); startCollectors() }
            return START_STICKY
        }
        NotificationManagerCompat.from(this).cancel(IDLE_NOTIF_ID)
        startForeground(NOTIF_ID, buildRecordingNotification())
        _isRunning.value = true
        startCollectors()
        return START_STICKY
    }

    override fun onDestroy() {
        _isRunning.value = false
        stopCollectors()
        super.onDestroy()
    }

    private fun startCollectors() {
        if (CollectorPrefs.enabled(this, CollectorPrefs.VOICE) &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        ) {
            if (voice == null) voice = VoiceCollector(this)
            voice?.start()
        }
        if (CollectorPrefs.enabled(this, CollectorPrefs.APPS)) {
            if (appUsage == null) appUsage = AppUsageCollector(this)
            appUsage?.start()
        }
        if (CollectorPrefs.enabled(this, CollectorPrefs.LOCATION)) {
            if (location == null) location = LocationCollector(this)
            location?.start()
        }
        if (CollectorPrefs.enabled(this, CollectorPrefs.COMMS)) {
            if (comms == null) comms = CommsCollector(this)
            comms?.start()
        }
        if (CollectorPrefs.enabled(this, CollectorPrefs.CALENDAR)) {
            if (calendar == null) calendar = CalendarCollector(this)
            calendar?.start()
        }
    }

    private fun stopCollectors() {
        voice?.stop()
        appUsage?.stop()
        location?.stop()
        comms?.stop()
        calendar?.stop()
    }

    private fun stopLogging() {
        _isRunning.value = false
        stopCollectors()
        stopForeground(STOP_FOREGROUND_REMOVE)
        showIdle(this)
        stopSelf()
    }

    private fun buildRecordingNotification(): Notification {
        val stop = PendingIntent.getService(
            this, 1,
            Intent(this, LoggerService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, LifeLogApp.CHANNEL_LOGGING)
            .setContentTitle(getString(R.string.notif_title))
            .setContentText(getString(R.string.notif_text))
            .setSmallIcon(R.drawable.ic_stat_logging)
            .setOngoing(true)
            .setContentIntent(openAppIntent(this))
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    companion object {
        private const val NOTIF_ID = 1
        private const val IDLE_NOTIF_ID = 2
        const val ACTION_STOP = "com.lifelog.action.STOP"
        const val ACTION_REAPPLY = "com.lifelog.action.REAPPLY"

        private val _isRunning = MutableStateFlow(false)
        val isRunning: StateFlow<Boolean> = _isRunning

        fun start(context: Context) {
            context.startForegroundService(Intent(context, LoggerService::class.java))
        }

        fun stop(context: Context) {
            context.startService(
                Intent(context, LoggerService::class.java).setAction(ACTION_STOP),
            )
        }

        /** 수집 항목 설정이 바뀌면 기록 중인 경우 즉시 반영한다. */
        fun reapply(context: Context) {
            if (!_isRunning.value) return
            context.startService(
                Intent(context, LoggerService::class.java).setAction(ACTION_REAPPLY),
            )
        }

        /** 멈춤 상태용 상시 알림(기록 시작 버튼). 앱 시작·부팅 때 띄워 둔다. 기록 중이면 띄우지 않는다. */
        fun showIdle(context: Context) {
            if (_isRunning.value) return
            val start = PendingIntent.getForegroundService(
                context, 2,
                Intent(context, LoggerService::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
            val notif = NotificationCompat.Builder(context, LifeLogApp.CHANNEL_LOGGING)
                .setContentTitle(context.getString(R.string.notif_idle_title))
                .setContentText(context.getString(R.string.notif_idle_text))
                .setSmallIcon(R.drawable.ic_stat_logging)
                .setOngoing(true)
                .setContentIntent(openAppIntent(context))
                .addAction(0, context.getString(R.string.action_start), start)
                .build()
            try {
                NotificationManagerCompat.from(context).notify(IDLE_NOTIF_ID, notif)
            } catch (_: SecurityException) {
                // POST_NOTIFICATIONS 미허용(Android 13+): 권한 받은 뒤 다시 뜬다.
            }
        }

        private fun openAppIntent(context: Context): PendingIntent =
            PendingIntent.getActivity(
                context, 0,
                Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
    }
}
