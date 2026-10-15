package com.lifelog.data

/** 캘린더 일정 한 건(시작 시각 기준으로 타임라인에 올린다). */
data class CalendarEntry(
    val id: Long,
    val timestampMs: Long,
    val title: String,
    val location: String,
    val endMs: Long,
)
