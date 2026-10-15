package com.lifelog.data

/**
 * 소통 한 건(통화 또는 문자).
 * [kind] "call"|"sms", [direction] "in"|"out"|"missed",
 * [party] 번호, [partyName] 연락처 이름(없으면 null),
 * [content] 통화는 길이(초) 문자열, 문자는 본문.
 */
data class CommEvent(
    val id: Long,
    val timestampMs: Long,
    val kind: String,
    val direction: String,
    val party: String,
    val partyName: String?,
    val content: String,
)
