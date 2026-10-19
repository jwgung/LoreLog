package com.lorelog

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * 부팅·앱 업데이트 후 복구. 마지막에 기록이 켜져 있었으면 자동 재개를 시도하고,
 * OS 가 백그라운드 포그라운드서비스 시작을 막으면(신형 안드로이드) '멈춤' 상시 알림으로 폴백해
 * 한 번만 눌러 다시 켤 수 있게 한다. 꺼져 있었으면 알림만 띄운다.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED -> {
                if (LoggerService.wasRunning(context)) {
                    try {
                        LoggerService.start(context)
                    } catch (_: Exception) {
                        LoggerService.showIdle(context)
                    }
                } else {
                    LoggerService.showIdle(context)
                }
            }
        }
    }
}
