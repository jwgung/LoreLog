package com.lorelog.summary

/**
 * 기록 묶음을 한 문단으로 요약하는 엔진. 구현을 바꿔 끼울 수 있게 인터페이스로 둔다.
 * 지금은 온디바이스 Gemini Nano([GeminiNanoSummarizer]).
 */
interface Summarizer {
    suspend fun summarize(text: String): SummaryResult

    sealed interface SummaryResult {
        data class Ok(val summary: String) : SummaryResult
        /** 이 기기에서 요약을 쓸 수 없음(미지원·다운로드 필요·오류). 흐름은 그대로 두고 비워 둔다. */
        /** [retryable] = 다른 모델로 다시 시도할 만한 실패(한도·종료·과부하). */
        data class Unavailable(val reason: String, val retryable: Boolean = false) : SummaryResult
    }
}
