package com.lorelog.work

import java.util.Calendar
import java.util.TimeZone

/**
 * 비서가 일정 직전에 먼저 말할 때의 규칙(#60). 기기와 시각을 받지 않는 순수 함수라 JVM 테스트로 확인한다.
 * - 일정 시작 10분 전에, 일정이 시작하기 전까지만.
 * - 밤 22시~아침 7시에는 말하지 않는다.
 * - 일정 하나에 한 번. 같은 (일정, 건)도 다시 말하지 않는다(일정 하나에 한 번이 이를 포함한다).
 * - 사용자가 멈춘 조직(같은 조직의 알림을 세 번 밀어서 지움)은 말하지 않는다. 하루 상한은 두지 않는다.
 */
object AheadRules {
    const val STOP_AFTER = 3
    private const val QUIET_FROM = 22
    private const val QUIET_UNTIL = 7

    /** 서랍이 준 한 줄. 시각은 epoch ms. */
    data class Line(
        val event: String,
        val start: Long,
        val at: Long,
        val title: String,
        val text: String,
        val question: String,
        val stop: String,
        val edge: String,
        val wrong: String,
        val matter: String,
    )

    sealed class Decision {
        data object Say : Decision()
        data object Drop : Decision()
        data class Wait(val ms: Long) : Decision()
    }

    fun quiet(ms: Long, tz: TimeZone): Boolean {
        val h = Calendar.getInstance(tz).apply { timeInMillis = ms }.get(Calendar.HOUR_OF_DAY)
        return h >= QUIET_FROM || h < QUIET_UNTIL
    }

    /** 이미 말한 일정의 열쇠(일정 하나에 한 번). */
    fun saidKey(l: Line): String = "${l.start}|${l.event}"

    /** 예약 작업 이름: 같은 (일정, 건)은 한 번만 예약된다. */
    fun workName(l: Line): String = "ahead-" + Integer.toHexString("${l.start}|${l.event}|${l.matter}".hashCode())

    fun decide(l: Line, now: Long, tz: TimeZone, said: Set<String>, stopped: Set<String>): Decision = when {
        now >= l.start -> Decision.Drop
        quiet(maxOf(l.at, now), tz) -> Decision.Drop
        saidKey(l) in said -> Decision.Drop
        l.stop in stopped -> Decision.Drop
        now < l.at -> Decision.Wait(l.at - now)
        else -> Decision.Say
    }

    /** 밀어서 지운 알림 하나를 센다. -> (새 횟수, 이번에 그 조직을 멈췄는가) */
    fun swiped(counts: Map<String, Int>, stop: String): Pair<Map<String, Int>, Boolean> {
        val n = (counts[stop] ?: 0) + 1
        return (counts + (stop to n)) to (n >= STOP_AFTER)
    }
}
