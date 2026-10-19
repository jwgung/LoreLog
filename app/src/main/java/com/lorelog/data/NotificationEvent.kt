package com.lorelog.data

/** 받은 알림 한 건. */
data class NotificationEvent(
    val id: Long,
    val timestampMs: Long,
    val packageName: String,
    val label: String,
    val title: String,
    val text: String,
)
