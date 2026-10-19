package com.lorelog.data

/** 앱이 전면에 올라온 시점 한 건. */
data class AppEvent(
    val id: Long,
    val timestampMs: Long,
    val packageName: String,
    val label: String,
)
