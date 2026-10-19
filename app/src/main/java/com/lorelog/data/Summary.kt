package com.lorelog.data

/** 일정 구간 기록을 요약한 한 건. */
data class Summary(
    val id: Long,
    val timestampMs: Long,
    val periodStartMs: Long,
    val periodEndMs: Long,
    val text: String,
)
