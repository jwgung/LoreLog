package com.lifelog.stt

/**
 * 음성 → 텍스트 엔진. 구현을 바꿔 끼울 수 있게 인터페이스로 둔다.
 * 지금은 안드로이드 내장 SpeechRecognizer([AndroidSpeechEngine]).
 * 상시 인식이 부족하면 Vosk 등 온디바이스 연속 엔진으로 교체한다.
 */
interface SttEngine {
    fun start()
    fun stop()

    interface Listener {
        /** 아직 확정 안 된 실시간 텍스트. */
        fun onPartial(text: String)
        /** 확정된 한 조각. */
        fun onFinal(text: String)
        /** 이 기기에서 인식을 쓸 수 없음. */
        fun onUnavailable(reason: String)
    }
}
