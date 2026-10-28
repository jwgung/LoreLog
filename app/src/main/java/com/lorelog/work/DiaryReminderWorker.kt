package com.lorelog.work

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.lorelog.DiaryPrefs
import com.lorelog.MainActivity
import com.lorelog.R
import com.lorelog.data.LogRepository
import com.lorelog.net.DiaryClient
import kotlinx.coroutines.delay
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.Calendar
import java.util.concurrent.TimeUnit

/** 사용자가 정한 시각에 "오늘의 일기" 알림을 띄우고 다음 날 같은 시각으로 다시 예약한다. */
class DiaryReminderWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        if (DiaryPrefs.enabled(applicationContext)) {
            val ready = runCatching { prepareDraft(applicationContext) }.getOrDefault(false)
            notifyNow(applicationContext, ready)
        }
        schedule(applicationContext)
        return Result.success()
    }

    companion object {
        private const val WORK_NAME = "diary-reminder"
        private const val CHANNEL = "diary"
        private const val NOTIF_ID = 4101
        const val EXTRA_OPEN_DIARY = "open_diary"

        /** 다음 알림 시각까지 기다렸다 한 번 실행하는 작업을 건다(설정이 바뀌면 다시 호출해 교체). */
        fun schedule(context: Context) {
            if (!DiaryPrefs.enabled(context)) {
                WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
                return
            }
            val now = Calendar.getInstance()
            val next = (now.clone() as Calendar).apply {
                set(Calendar.HOUR_OF_DAY, DiaryPrefs.hour(context))
                set(Calendar.MINUTE, DiaryPrefs.minute(context))
                set(Calendar.SECOND, 0)
                set(Calendar.MILLISECOND, 0)
                if (!after(now)) add(Calendar.DAY_OF_YEAR, 1)
            }
            val request = OneTimeWorkRequestBuilder<DiaryReminderWorker>()
                .setInitialDelay(next.timeInMillis - now.timeInMillis, TimeUnit.MILLISECONDS)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(WORK_NAME, ExistingWorkPolicy.REPLACE, request)
        }

        /** 알림을 누르는 순간 바로 보이도록 오늘 정리를 미리 만들어 둔다. 서버에 못 닿으면 false. */
        private suspend fun prepareDraft(context: Context): Boolean {
            LogRepository.pushToHttp(context)
            LogRepository.pushConversations(context)
            val day = SimpleDateFormat("yyyy-MM-dd", Locale.US).format(Date())
            repeat(60) {
                when (DiaryClient.fetchDraft(context, day)) {
                    is DiaryClient.Draft.Ready -> return true
                    is DiaryClient.Draft.Error -> return false
                    DiaryClient.Draft.Generating -> delay(3_000)
                }
            }
            return false
        }

        private fun notifyNow(context: Context, ready: Boolean) {
            val nm = context.getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "일기 알림", NotificationManager.IMPORTANCE_DEFAULT))
            val open = PendingIntent.getActivity(
                context, 0,
                Intent(context, MainActivity::class.java)
                    .putExtra(EXTRA_OPEN_DIARY, true)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val n = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_logging)
                .setContentTitle("오늘의 일기")
                .setContentText(
                    if (ready) "오늘 하루, 이렇게 정리해 봤어요. 틀린 데가 있으면 알려 주세요."
                    else "오늘 하루를 같이 돌아볼까요?",
                )
                .setContentIntent(open)
                .setAutoCancel(true)
                .build()
            try {
                if (NotificationManagerCompat.from(context).areNotificationsEnabled()) {
                    NotificationManagerCompat.from(context).notify(NOTIF_ID, n)
                }
            } catch (_: SecurityException) {
                // POST_NOTIFICATIONS 미허용: 알림 없이 다음 날로 넘어간다.
            }
        }
    }
}
