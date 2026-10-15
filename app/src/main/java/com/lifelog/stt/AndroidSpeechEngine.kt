package com.lifelog.stt

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.util.Log

/**
 * 안드로이드 내장 구글 음성 인식. 짧은 발화 전제라 상시 인식을 위해
 * 세션이 끝나거나 오류가 나면 바로 다시 연다(재시작 루프).
 * 끊길 때 아직 확정 안 된 마지막 부분 인식은 조각으로 흘려 유실을 막는다.
 * SpeechRecognizer 는 메인 스레드에서만 다룰 수 있어 모든 제어를 메인 핸들러로 보낸다.
 */
class AndroidSpeechEngine(
    private val context: Context,
    private val listener: SttEngine.Listener,
) : SttEngine {

    private val main = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    private var running = false
    private var lastPartial = ""

    override fun start() {
        main.post {
            if (running) return@post
            if (!SpeechRecognizer.isRecognitionAvailable(context)) {
                listener.onUnavailable("이 기기에서 음성 인식을 쓸 수 없습니다")
                return@post
            }
            running = true
            recognizer = SpeechRecognizer.createSpeechRecognizer(context).apply {
                setRecognitionListener(recognitionListener)
            }
            listen()
        }
    }

    override fun stop() {
        main.post {
            running = false
            main.removeCallbacksAndMessages(null)
            try { recognizer?.destroy() } catch (_: Exception) {}
            recognizer = null
            lastPartial = ""
        }
    }

    private fun listen() {
        if (!running) return
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            // 가능하면 오프라인(온디바이스) 우선. 안 되면 시스템이 온라인으로 넘어간다.
            putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
            putExtra(RecognizerIntent.EXTRA_CALLING_PACKAGE, context.packageName)
            // 언어는 기기 기본값을 따른다(명시하지 않음).
        }
        try {
            recognizer?.startListening(intent)
        } catch (e: Exception) {
            Log.w(TAG, "startListening 실패", e)
            restart(RESTART_DELAY_MS)
        }
    }

    private fun restart(delayMs: Long) {
        if (!running) return
        main.postDelayed({
            if (!running) return@postDelayed
            try { recognizer?.cancel() } catch (_: Exception) {}
            listen()
        }, delayMs)
    }

    private fun flushPartialAsFinal() {
        val p = lastPartial.trim()
        lastPartial = ""
        if (p.isNotEmpty()) listener.onFinal(p)
    }

    private val recognitionListener = object : RecognitionListener {
        override fun onReadyForSpeech(params: Bundle?) {}
        override fun onBeginningOfSpeech() {}
        override fun onRmsChanged(rmsdB: Float) {}
        override fun onBufferReceived(buffer: ByteArray?) {}
        override fun onEndOfSpeech() {}

        override fun onPartialResults(partialResults: Bundle?) {
            val text = firstResult(partialResults)
            if (text != null) {
                lastPartial = text
                listener.onPartial(text)
            }
        }

        override fun onResults(results: Bundle?) {
            val text = firstResult(results)
            lastPartial = ""
            listener.onPartial("")
            if (text != null) listener.onFinal(text)
            restart(SHORT_DELAY_MS)
        }

        override fun onError(error: Int) {
            // NO_MATCH·SPEECH_TIMEOUT 는 조용한 구간일 뿐 정상. 그 전 부분 인식은 흘려 보낸다.
            listener.onPartial("")
            flushPartialAsFinal()
            val delay = if (error == SpeechRecognizer.ERROR_RECOGNIZER_BUSY) BUSY_DELAY_MS else RESTART_DELAY_MS
            restart(delay)
        }

        override fun onEvent(eventType: Int, params: Bundle?) {}
    }

    private fun firstResult(bundle: Bundle?): String? =
        bundle?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
            ?.firstOrNull()
            ?.takeIf { it.isNotBlank() }

    companion object {
        private const val TAG = "AndroidSpeechEngine"
        private const val SHORT_DELAY_MS = 200L
        private const val RESTART_DELAY_MS = 600L
        private const val BUSY_DELAY_MS = 1000L
    }
}
