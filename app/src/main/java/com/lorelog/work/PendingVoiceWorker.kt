package com.lorelog.work

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lorelog.collect.PendingVoice
import java.util.concurrent.TimeUnit

/** 밀린 음성파일을 네트워크가 있을 때 주기적으로(30분) 금고로 보내 전사한다. 망 조건은 [PushPrefs]. */
class PendingVoiceWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        PendingVoice.transcribePending(applicationContext)
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "pending-voice"

        fun schedule(context: Context) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(PushPrefs.networkType(PushPrefs.wifiOnly(context)))
                .build()
            val request = PeriodicWorkRequestBuilder<PendingVoiceWorker>(30, TimeUnit.MINUTES)
                .setConstraints(constraints)
                .build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.UPDATE, // 망 조건이 바뀌면 바로 반영(다음 실행 시각은 그대로)
                request,
            )
        }
    }
}
