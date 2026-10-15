package com.lifelog.summary

import android.content.Context
import com.google.mlkit.genai.common.FeatureStatus
import com.google.mlkit.genai.common.GenAiException
import com.google.mlkit.genai.summarization.Summarization
import com.google.mlkit.genai.summarization.SummarizationRequest
import com.google.mlkit.genai.summarization.Summarizer as MlSummarizer
import com.google.mlkit.genai.summarization.SummarizerOptions
import kotlinx.coroutines.suspendCancellableCoroutine
import androidx.concurrent.futures.await
import kotlin.coroutines.resume

/**
 * 온디바이스 Gemini Nano(ML Kit GenAI 요약). 기기 안에서만 돌아 데이터가 밖으로 안 나간다.
 * AICore 가 있는 기기(Pixel 8+/Galaxy S24+ 급, Android 14+)에서만 동작한다.
 * 미지원 기기에서는 [Summarizer.SummaryResult.Unavailable] 을 돌려주고 흐름은 그대로 둔다.
 */
class GeminiNanoSummarizer(private val context: Context) : Summarizer {

    override suspend fun summarize(text: String): Summarizer.SummaryResult {
        if (text.isBlank()) return Summarizer.SummaryResult.Unavailable("요약할 기록이 없습니다")
        val options = SummarizerOptions.builder(context)
            .setInputType(SummarizerOptions.InputType.ARTICLE)
            .setOutputType(SummarizerOptions.OutputType.THREE_BULLETS)
            .setLanguage(SummarizerOptions.Language.KOREAN)
            .build()
        val client = Summarization.getClient(options)
        return try {
            when (client.checkFeatureStatus().await()) {
                FeatureStatus.AVAILABLE -> run(client, text)
                FeatureStatus.DOWNLOADABLE, FeatureStatus.DOWNLOADING ->
                    if (download(client)) run(client, text)
                    else Summarizer.SummaryResult.Unavailable("요약 모델 내려받기 실패")
                else -> Summarizer.SummaryResult.Unavailable("이 기기는 온디바이스 Gemini Nano 미지원")
            }
        } catch (e: Exception) {
            Summarizer.SummaryResult.Unavailable(e.message ?: "요약 중 오류")
        } finally {
            runCatching { client.close() }
        }
    }

    private suspend fun run(client: MlSummarizer, text: String): Summarizer.SummaryResult {
        val request = SummarizationRequest.builder(text).build()
        val result = client.runInference(request).await()
        return Summarizer.SummaryResult.Ok(result.summary.trim())
    }

    private suspend fun download(client: MlSummarizer): Boolean =
        suspendCancellableCoroutine { cont ->
            client.downloadFeature(object : com.google.mlkit.genai.common.DownloadCallback {
                override fun onDownloadStarted(bytesToDownload: Long) {}
                override fun onDownloadProgress(totalBytesDownloaded: Long) {}
                override fun onDownloadCompleted() { if (cont.isActive) cont.resume(true) }
                override fun onDownloadFailed(e: GenAiException) { if (cont.isActive) cont.resume(false) }
            })
        }
}
