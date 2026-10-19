package com.lorelog.data

/**
 * 확정된 음성 조각 한 줄. 원본 오디오는 보관하지 않고 텍스트만 남긴다.
 * [speaker] 는 화자 라벨(예: 통화 상대). 일반 녹음 조각은 보통 null.
 */
data class VoiceSegment(
    val id: Long,
    val timestampMs: Long,
    val text: String,
    val speaker: String? = null,
)
