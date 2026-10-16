package com.lifelog

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * 멈춤 알림이 지워지면(deleteIntent) 다시 띄운다. '알림 숨기기'가 켜져 있으면 다시 띄우지 않는다.
 * (showIdle 안에서 숨기기·기록중 여부를 확인한다.)
 */
class NotifReviveReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        LoggerService.showIdle(context)
    }
}
