package com.lorelog.work

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.WorkManager
import com.lorelog.collect.CallRecordingImporter
import java.util.concurrent.TimeUnit

/** 지정한 폴더의 새 통화 녹음을 주기적으로(2시간) 서랍으로 보내 전사해 가져온다. 폴더 미지정이면 아무것도 안 함. 망 조건은 [PushPrefs]. */
class CallRecordingWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        CallRecordingImporter.importNew(applicationContext)
        Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "callrec-import"

        fun schedule(context: Context) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(PushPrefs.networkType(PushPrefs.wifiOnly(context)))
                .build()
            val request = PeriodicWorkRequestBuilder<CallRecordingWorker>(2, TimeUnit.HOURS)
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
