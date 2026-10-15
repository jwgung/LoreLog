package com.lifelog.data

/** 어떤 앱 화면에서 읽은 텍스트 한 건. 민감 앱(제외 목록)은 여기 들어오지 않는다. */
data class ScreenContentEvent(
    val id: Long,
    val timestampMs: Long,
    val packageName: String,
    val label: String,
    val text: String,
)
