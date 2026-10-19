package com.lorelog.work

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lorelog.data.LogRepository
import java.util.concurrent.TimeUnit

/**
 * 음성 조각을 대화로 묶고 통화·회의로 참석자를 자동 연결한다(15분 주기).
 * 자동으로 안 채워진 대화는 검토 큐(pending)로 남는다.
 */
class ConversationWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        LogRepository.buildConversations(applicationContext)
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "conversation-build"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<ConversationWorker>(15, TimeUnit.MINUTES).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request,
            )
        }
    }
}
