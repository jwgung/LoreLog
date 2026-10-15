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
import androidx.core.content.ContextCompat
import com.lifelog.collect.AppUsageCollector
import com.lifelog.collect.LocationCollector
import com.lifelog.collect.VoiceCollector
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * 상시 기록 서비스. 1단계에서는 포그라운드 서비스로 떠서 지울 수 없는 알림을 띄우고
 * 시작·정지만 한다. 2단계부터 수집기(음성·앱·위치·알림)를 여기에 붙인다.
 */
class LoggerService : Service() {

    private var voice: VoiceCollector? = null
    private var appUsage: AppUsageCollector? = null
    private var location: LocationCollector? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopLogging()
            return START_NOT_STICKY
        }
        startForeground(NOTIF_ID, buildNotification())
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
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
            == PackageManager.PERMISSION_GRANTED
        ) {
            if (voice == null) voice = VoiceCollector(this)
            voice?.start()
        }
        if (appUsage == null) appUsage = AppUsageCollector(this)
        appUsage?.start()
        if (location == null) location = LocationCollector(this)
        location?.start()
    }

    private fun stopCollectors() {
        voice?.stop()
        appUsage?.stop()
        location?.stop()
    }

    private fun stopLogging() {
        _isRunning.value = false
        stopCollectors()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun buildNotification(): Notification {
        val openApp = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
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
            .setContentIntent(openApp)
            .addAction(0, getString(R.string.action_stop), stop)
            .build()
    }

    companion object {
        private const val NOTIF_ID = 1
        const val ACTION_STOP = "com.lifelog.action.STOP"

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
    }
}
