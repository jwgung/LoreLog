package com.lifelog.collect

import android.content.Context
import com.lifelog.data.LogRepository
import com.lifelog.stt.AndroidSpeechEngine
import com.lifelog.stt.SttEngine

/**
 * 음성 수집기. STT 엔진을 돌려 확정 조각을 저장소에 쌓고,
 * 실시간 부분 인식은 화면 표시용으로만 흘린다(저장 안 함).
 */
class VoiceCollector(context: Context) {

    private val engine: SttEngine = AndroidSpeechEngine(
        context.applicationContext,
        object : SttEngine.Listener {
            override fun onPartial(text: String) {
                LogRepository.setPartial(text)
            }

            override fun onFinal(text: String) {
                LogRepository.addVoice(text)
            }

            override fun onUnavailable(reason: String) {
                LogRepository.setStatus(reason)
            }
        },
    )

    fun start() {
        LogRepository.setStatus("")
        engine.start()
    }

    fun stop() {
        engine.stop()
        LogRepository.setPartial("")
    }
}
