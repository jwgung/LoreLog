package com.lorelog.work

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.lorelog.AheadReceiver
import com.lorelog.MainActivity
import com.lorelog.R
import com.lorelog.collect.UpcomingEvents
import com.lorelog.net.DiaryClient
import com.lorelog.net.EndpointConfig
import java.util.TimeZone
import java.util.concurrent.TimeUnit

/**
 * 비서가 다음 행동의 그 자리에서 먼저 말한다(#60). 15분 전송([PushWorker]) 때 앞으로 2시간의 일정을 금고에 묻고,
 * 금고가 준 한 줄마다 일정 시작 10분 전에 도는 일회성 작업을 건다. 그 시각에는 금고가 꺼져 있어도 뜬다.
 * 규칙은 [AheadRules]. 알림은 "비서" 채널이라 OS 설정에서 따로 끈다.
 */
class AheadWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val l = lineOf(inputData) ?: return Result.success()
        val c = applicationContext
        when (val d = AheadRules.decide(l, System.currentTimeMillis(), TimeZone.getDefault(), AheadPrefs.said(c), AheadPrefs.stopped(c))) {
            AheadRules.Decision.Say -> if (notifyNow(c, l)) AheadPrefs.markSaid(c, AheadRules.saidKey(l))
            is AheadRules.Decision.Wait -> enqueue(c, l, d.ms, ExistingWorkPolicy.REPLACE) // 일찍 깨어났다
            AheadRules.Decision.Drop -> {}
        }
        return Result.success()
    }

    companion object {
        const val CHANNEL = "assistant"
        const val EXTRA_ASK = "ask"
        private const val WINDOW_MS = 2 * 3_600_000L

        /** 금고에 앞으로 2시간을 묻고 예약한다. 와이파이 전용이 켜져 있고 데이터 요금 망이면 묻지 않는다. */
        suspend fun ask(context: Context) {
            if (!EndpointConfig.isConfigured(context) || !PushPrefs.mayAutoSend(context)) return
            for ((w, r, ref) in AheadPrefs.pending(context)) {
                if (DiaryClient.correct(context, w, r, ref) != null) AheadPrefs.removePending(context, w, r, ref)
            }
            val now = System.currentTimeMillis()
            val events = UpcomingEvents.between(context, now, now + WINDOW_MS)
            val lines = DiaryClient.ahead(context, events, AheadPrefs.stopped(context)) ?: return
            for (l in lines) {
                when (val d = AheadRules.decide(l, now, TimeZone.getDefault(), AheadPrefs.said(context), AheadPrefs.stopped(context))) {
                    is AheadRules.Decision.Wait -> enqueue(context, l, d.ms, ExistingWorkPolicy.KEEP)
                    AheadRules.Decision.Say -> enqueue(context, l, 0, ExistingWorkPolicy.KEEP)
                    AheadRules.Decision.Drop -> {}
                }
            }
        }

        private fun enqueue(context: Context, l: AheadRules.Line, delayMs: Long, policy: ExistingWorkPolicy) {
            val request = OneTimeWorkRequestBuilder<AheadWorker>()
                .setInitialDelay(delayMs, TimeUnit.MILLISECONDS)
                .setInputData(dataOf(l))
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(AheadRules.workName(l), policy, request)
        }

        private fun dataOf(l: AheadRules.Line): Data = Data.Builder()
            .putString("event", l.event).putLong("start", l.start).putLong("at", l.at)
            .putString("title", l.title).putString("text", l.text).putString("question", l.question)
            .putString("stop", l.stop).putString("edge", l.edge).putString("wrong", l.wrong).putString("matter", l.matter)
            .build()

        private fun lineOf(d: Data): AheadRules.Line? {
            val s = { k: String -> d.getString(k).orEmpty() }
            if (s("event").isEmpty() || s("edge").isEmpty()) return null
            return AheadRules.Line(
                s("event"), d.getLong("start", 0), d.getLong("at", 0), s("title"), s("text"), s("question"),
                s("stop"), s("edge"), s("wrong"), s("matter"),
            )
        }

        /** 한 줄을 띄운다. 알림을 못 띄우면 false(말한 것으로 치지 않는다). */
        private fun notifyNow(context: Context, l: AheadRules.Line): Boolean {
            val nm = context.getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "비서", NotificationManager.IMPORTANCE_DEFAULT))
            val id = AheadRules.workName(l).hashCode()
            val open = PendingIntent.getActivity(
                context, id,
                Intent(context, MainActivity::class.java)
                    .putExtra(EXTRA_ASK, l.question)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            fun action(what: String, code: Int): PendingIntent = PendingIntent.getBroadcast(
                context, id * 4 + code,
                Intent(context, AheadReceiver::class.java).setAction(what)
                    .putExtra(AheadReceiver.EXTRA_ID, id).putExtra(AheadReceiver.EXTRA_WRONG, l.wrong)
                    .putExtra(AheadReceiver.EXTRA_EDGE, l.edge).putExtra(AheadReceiver.EXTRA_STOP, l.stop),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val n = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_logging)
                .setContentTitle(l.title)
                .setContentText(l.text)
                .setStyle(NotificationCompat.BigTextStyle().bigText(l.text))
                .setContentIntent(open)
                .setAutoCancel(true)
                .setTimeoutAfter(maxOf(l.start - System.currentTimeMillis(), 60_000L) + 30 * 60_000L)
                .addAction(0, "이미 했어요", action(AheadReceiver.ACTION_DONE, 1))
                .addAction(0, "이건 아니에요", action(AheadReceiver.ACTION_NOT_THIS, 2))
                .setDeleteIntent(action(AheadReceiver.ACTION_SWIPED, 3))
                // 잠금 화면에는 누가 무엇을 요청했는지 보이지 않게
                .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                .setPublicVersion(
                    NotificationCompat.Builder(context, CHANNEL).setSmallIcon(R.drawable.ic_stat_logging)
                        .setContentTitle("비서").setContentText("곧 시작하는 일정에 대해 알려 드릴 것이 있어요").build(),
                )
                .build()
            return try {
                val mgr = NotificationManagerCompat.from(context)
                if (!mgr.areNotificationsEnabled()) return false
                mgr.notify(id, n)
                true
            } catch (_: SecurityException) {
                false // POST_NOTIFICATIONS 미허용
            }
        }
    }
}
