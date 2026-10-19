package com.lorelog.data

/**
 * 대화 한 건(음성 조각을 시간 간격으로 묶은 것).
 * [participants] 참석자 이름(쉼표), [source] "call"|"calendar"|"manual"|"" (어떻게 채웠나),
 * [status] "auto"|"pending"|"tagged", [context] 겹친 일정 제목 등 힌트.
 */
data class Conversation(
    val id: Long,
    val startMs: Long,
    val endMs: Long,
    val participants: String,
    val source: String,
    val status: String,
    val context: String,
)
