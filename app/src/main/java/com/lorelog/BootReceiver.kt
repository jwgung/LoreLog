package com.lorelog

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** 부팅 후 '멈춤' 상시 알림을 다시 띄워, 알림창에서 늘 기록을 켤 수 있게 한다. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action == Intent.ACTION_BOOT_COMPLETED) {
            LoggerService.showIdle(context)
        }
    }
}
