package com.lorelog

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import com.lorelog.collect.PendingVoice
import com.lorelog.data.LogRepository
import com.lorelog.work.ExportWorker
import com.lorelog.work.CallRecordingWorker
import com.lorelog.work.PendingVoiceWorker
import com.lorelog.work.ConversationWorker
import com.lorelog.work.DiaryReminderWorker
import com.lorelog.work.PruneWorker
import com.lorelog.work.PushWorker
import com.lorelog.work.SummaryWorker

/** 앱 전역 초기화. 상시 알림 채널을 한 번 만든다. */
class LifeLogApp : Application() {
    override fun onCreate() {
        super.onCreate()
        LogRepository.init(this)
        Thread { PendingVoice.refresh(this) }.start()
        ExportWorker.schedule(this)
        PushWorker.schedule(this)
        SummaryWorker.schedule(this)
        PruneWorker.schedule(this)
        ConversationWorker.schedule(this)
        DiaryReminderWorker.schedule(this)
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
