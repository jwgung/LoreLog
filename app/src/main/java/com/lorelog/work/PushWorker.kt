package com.lorelog.work

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.lorelog.data.LogRepository
import java.util.concurrent.TimeUnit

/**
 * 15분마다(WorkManager 주기 작업의 최소) 새 기록을 금고로 보낸다. 받는 쪽은 꺼질 수 있는 기기라(데스크톱 내장 금고),
 * 닿지 않으면 기록은 기기 DB에 그대로 있고 전송 커서도 그대로다. 다음 시도는 1분부터 늘려 가며 다시 한다.
 * 오래 못 보냈어도 한 번에 금고 본문 한도를 넘지 않게 나눠 보낸다([LogRepository.pushToHttp]).
 * "와이파이에 연결됐을 때만 보내기"를 켜면 데이터 요금이 없는 망에서만 돈다([PushPrefs]).
 */
class PushWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = try {
        if (LogRepository.pushToHttp(applicationContext) == LogRepository.PUSH_FAILED) Result.retry() else Result.success()
    } catch (e: Exception) {
        Result.retry()
    }

    companion object {
        private const val WORK_NAME = "push-15m"
        const val INTERVAL_MIN = 15L

        /** 등록하거나, 이미 있으면 조건만 바꾼다(UPDATE 는 다음 실행 시각을 그대로 둔다). */
        fun schedule(context: Context) {
            val network = PushPrefs.networkType(PushPrefs.wifiOnly(context))
            val request = PeriodicWorkRequestBuilder<PushWorker>(INTERVAL_MIN, TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(network).build())
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
                .build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.UPDATE, request)
        }
    }
}
