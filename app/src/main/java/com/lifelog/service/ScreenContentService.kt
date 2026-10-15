package com.lifelog.service

import android.accessibilityservice.AccessibilityService
import android.content.pm.PackageManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.lifelog.LoggerService
import com.lifelog.collect.SensitiveApps
import com.lifelog.data.LogRepository

/**
 * 앱 내 화면 내용 수집. 접근성 서비스로 전면 화면의 텍스트를 읽는다.
 * - '기록 중'일 때만 수집한다(LoggerService 가 꺼져 있으면 아무것도 안 함).
 * - 제외 목록(은행·결제 등)에 든 앱은 건너뛴다.
 * - 같은 앱에서 너무 잦은 중복은 간격·동일 텍스트 검사로 줄인다.
 * 사용자가 설정 > 접근성에서 직접 켜야 동작한다.
 */
class ScreenContentService : AccessibilityService() {

    private val labelCache = HashMap<String, String>()
    private var lastPackage = ""
    private var lastText = ""
    private var lastCaptureMs = 0L

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        event ?: return
        val type = event.eventType
        if (type != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED &&
            type != AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED
        ) return

        if (!LoggerService.isRunning.value) return

        val pkg = event.packageName?.toString() ?: return
        if (pkg == packageName) return
        if (SensitiveApps.isExcluded(this, pkg)) return

        val now = System.currentTimeMillis()
        if (pkg == lastPackage && now - lastCaptureMs < MIN_INTERVAL_MS) return

        val root = rootInActiveWindow ?: return
        val text = buildString { collectText(root, this, 0) }.trim().take(MAX_LEN)
        try { root.recycle() } catch (_: Exception) {}
        if (text.isEmpty()) return
        if (pkg == lastPackage && text == lastText) return

        lastPackage = pkg
        lastText = text
        lastCaptureMs = now
        LogRepository.addScreen(pkg, label(pkg), text, now)
    }

    override fun onInterrupt() {}

    /** 노드 트리를 훑어 보이는 텍스트를 모은다. 노드 수·깊이를 제한한다. */
    private fun collectText(node: AccessibilityNodeInfo?, sb: StringBuilder, depth: Int) {
        if (node == null || depth > MAX_DEPTH || sb.length > MAX_LEN) return
        val piece = node.text?.toString()?.trim().takeUnless { it.isNullOrEmpty() }
            ?: node.contentDescription?.toString()?.trim().takeUnless { it.isNullOrEmpty() }
        if (piece != null && !sb.contains(piece)) {
            if (sb.isNotEmpty()) sb.append(' ')
            sb.append(piece)
        }
        val count = node.childCount
        for (i in 0 until count) {
            val child = node.getChild(i)
            collectText(child, sb, depth + 1)
            try { child?.recycle() } catch (_: Exception) {}
        }
    }

    private fun label(pkg: String): String = labelCache.getOrPut(pkg) {
        try {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
        } catch (e: PackageManager.NameNotFoundException) {
            pkg
        }
    }

    companion object {
        private const val MIN_INTERVAL_MS = 1500L
        private const val MAX_LEN = 2000
        private const val MAX_DEPTH = 40
    }
}
