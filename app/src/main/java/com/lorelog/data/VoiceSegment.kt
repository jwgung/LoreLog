package com.lorelog.data

/**
 * 확정된 음성 조각 한 줄. 텍스트가 본체이고, [audio] 는 다시 들어볼 수 있게 잠시 보관한
 * 오디오 파일명(voice_kept/ 아래, AAC). 내보낸 뒤 또는 7일 뒤 지워지면 null.
 * [speaker] 는 화자 라벨(예: 통화 상대). 일반 녹음 조각은 보통 null.
 */
data class VoiceSegment(
    val id: Long,
    val timestampMs: Long,
    val text: String,
    val speaker: String? = null,
    val audio: String? = null,
)

/** 아직 전사되지 않은 대기 음성 파일. 타임라인에 "전사 대기"로 보이고 바로 들어볼 수 있다. */
data class PendingAudio(
    val path: String,
    val timestampMs: Long,
    val durationMs: Long,
)
