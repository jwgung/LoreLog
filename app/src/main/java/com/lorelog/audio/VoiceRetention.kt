package com.lorelog.audio

import com.lorelog.data.Conversation

/**
 * 다시 들어볼 보관 오디오를 언제 지우나. 대화 확인 카드에서 들어 보고 답하도록, 서랍으로 보냈는지와 상관없이 그 대화를
 * 확인할 때까지 남긴다. 그 음성이 든 대화가 답을 받았으면(누구와였는지, 넘김, 내 대화 아님, 자동으로 정해짐) 지우고,
 * 아직 어느 대화에도 들지 않았거나 확인을 기다리는 음성은 남긴다. 어느 쪽이든 [VoiceAudio.RETENTION_MS](7일)보다
 * 오래 두지 않는다. 서랍은 전사할 때 받은 소리를 남기지 않고 글만 둔다.
 */
object VoiceRetention {
    /** 답을 받은 대화의 상태. 확인을 기다리는 것은 "pending". */
    private val ANSWERED = setOf("tagged", "skipped", "ambient", "auto")

    /** [voices] (id, 녹음 시각)와 대화들에서, 오디오를 지울 음성의 id. */
    fun toDrop(voices: List<Pair<Long, Long>>, conversations: List<Conversation>, nowMs: Long): List<Long> {
        val cutoff = nowMs - VoiceAudio.RETENTION_MS
        val answered = conversations.filter { it.status in ANSWERED }
        return voices.filter { (_, ts) -> ts < cutoff || answered.any { ts in it.startMs..it.endMs } }.map { it.first }
    }
}
