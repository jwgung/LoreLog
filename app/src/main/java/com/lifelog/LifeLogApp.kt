package com.lifelog

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import com.lifelog.data.LogRepository
import com.lifelog.work.ExportWorker
import com.lifelog.work.CallRecordingWorker
import com.lifelog.work.PendingVoiceWorker
import com.lifelog.work.ConversationWorker
import com.lifelog.work.PruneWorker
import com.lifelog.work.SummaryWorker

/** 앱 전역 초기화. 상시 알림 채널을 한 번 만든다. */
class LifeLogApp : Application() {
    override fun onCreate() {
        super.onCreate()
        LogRepository.init(this)
        ExportWorker.schedule(this)
        SummaryWorker.schedule(this)
        PruneWorker.schedule(this)
        ConversationWorker.schedule(this)
        CallRecordingWorker.schedule(this)
        PendingVoiceWorker.schedule(this)
        val channel = NotificationChannel(
            CHANNEL_LOGGING,
            getString(R.string.channel_logging_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.channel_logging_desc)
            setShowBadge(false)
        }
        val minChannel = NotificationChannel(
            CHANNEL_LOGGING_MIN,
            getString(R.string.channel_logging_min_name),
            NotificationManager.IMPORTANCE_MIN,
        ).apply { setShowBadge(false) }
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(channel)
        nm.createNotificationChannel(minChannel)
        LoggerService.showIdle(this)
    }

    companion object {
        const val CHANNEL_LOGGING = "logging"
        const val CHANNEL_LOGGING_MIN = "logging_min"
    }
}
