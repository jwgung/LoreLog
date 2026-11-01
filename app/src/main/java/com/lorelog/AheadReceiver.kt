package com.lorelog

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationManagerCompat
import com.lorelog.net.DiaryClient
import com.lorelog.work.AheadPrefs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * 비서가 먼저 말한 알림(#60)의 버튼과 밀어서 지우기.
 * - 「이미 했어요」: 그 요청은 답한 것이라는 정정(#10, right "답함").
 * - 「이건 아니에요」: 그 요청은 요청이 아니라는 정정(right "-").
 *   서랍에 닿지 않으면 남겨 두었다가 다음 15분 전송 때 보낸다.
 * - 밀어서 지움: 그 조직의 횟수를 센다. 세 번이면 그 조직은 멈춘다.
 */
class AheadReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        val i = intent ?: return
        val stop = i.getStringExtra(EXTRA_STOP).orEmpty()
        val right = when (i.action) {
            ACTION_DONE -> ANSWERED
            ACTION_NOT_THIS -> "-"
            ACTION_SWIPED -> { AheadPrefs.swiped(context, stop); return }
            else -> return
        }
        NotificationManagerCompat.from(context).cancel(i.getIntExtra(EXTRA_ID, 0))
        val wrong = i.getStringExtra(EXTRA_WRONG).orEmpty()
        val edge = i.getStringExtra(EXTRA_EDGE).orEmpty()
        if (wrong.isBlank() || edge.isBlank()) return
        val app = context.applicationContext
        val done = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                if (DiaryClient.correct(app, wrong, right, edge) == null) AheadPrefs.addPending(app, wrong, right, edge)
            } finally {
                done.finish()
            }
        }
    }

    companion object {
        const val ACTION_DONE = "com.lorelog.ahead.DONE"
        const val ACTION_NOT_THIS = "com.lorelog.ahead.NOT_THIS"
        const val ACTION_SWIPED = "com.lorelog.ahead.SWIPED"
        const val EXTRA_ID = "id"
        const val EXTRA_WRONG = "wrong"
        const val EXTRA_EDGE = "edge"
        const val EXTRA_STOP = "stop"
        /** 서랍의 corrections::ANSWERED 와 같은 말 */
        const val ANSWERED = "답함"
    }
}
