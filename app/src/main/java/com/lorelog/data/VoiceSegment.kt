package com.lorelog.data

/**
 * 확정된 음성 조각 한 줄. 원본 오디오는 보관하지 않고 텍스트만 남긴다.
 * [speaker] 는 화자 식별(나/상대/모름)용으로 2b 단계에서 채운다. 지금은 null.
 */
data class VoiceSegment(
    val id: Long,
    val timestampMs: Long,
    val text: String,
    val speaker: String? = null,
)
