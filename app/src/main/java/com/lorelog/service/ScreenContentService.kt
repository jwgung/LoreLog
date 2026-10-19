package com.lorelog.service

import android.accessibilityservice.AccessibilityService
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import com.lorelog.LoggerService
import com.lorelog.collect.CollectorPrefs
import com.lorelog.collect.SensitiveApps
import com.lorelog.data.LogRepository

/**
 * 앱 내 화면 내용 수집. 화면이 바뀔 때(typeWindowStateChanged)만 받아,
 * 잠깐 뒤에(디바운스) 한 번만 화면 텍스트를 읽는다. 고빈도 'typeWindowContentChanged'는
 * 받지 않으므로 영상 재생·내비 같은 화면에서도 부담이 적다.
 * - '기록 중'이고 화면 수집 설정이 켜져 있을 때만.
 * - 제외 목록(은행·결제 등) 앱은 건너뛴다. 사용자가 설정 > 접근성에서 직접 켜야 동작한다.
 */
class ScreenContentService : AccessibilityService() {

    private val main = Handler(Looper.getMainLooper())
    private val labelCache = HashMap<String, String>()
    private var lastPackage = ""
    private var lastText = ""
    private var pendingPackage = ""

    private val captureRunnable = Runnable { capture(pendingPackage) }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        event ?: return
        if (event.eventType != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) return
        val pkg = event.packageName?.toString() ?: return
        if (pkg == packageName) return
        // 화면이 자리 잡을 시간을 조금 준 뒤 한 번만 읽는다(연속 전환은 마지막 것만).
        pendingPackage = pkg
        main.removeCallbacks(captureRunnable)
        main.postDelayed(captureRunnable, CAPTURE_DELAY_MS)
    }

    override fun onInterrupt() {
        main.removeCallbacks(captureRunnable)
    }

    private fun capture(pkg: String) {
        if (pkg.isEmpty() || pkg == packageName) return
        if (!LoggerService.isRunning.value) return
        if (!CollectorPrefs.enabled(this, CollectorPrefs.SCREEN)) return
        if (SensitiveApps.isExcluded(this, pkg)) return

        val root = rootInActiveWindow ?: return
        val text = buildString { collectText(root, this, 0) }.trim().take(MAX_LEN)
        try { root.recycle() } catch (_: Exception) {}
        if (text.isEmpty()) return
        if (pkg == lastPackage && text == lastText) return

        lastPackage = pkg
        lastText = text
        LogRepository.addScreen(pkg, label(pkg), text, System.currentTimeMillis())
    }

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
        private const val CAPTURE_DELAY_MS = 700L
        private const val MAX_LEN = 2000
        private const val MAX_DEPTH = 40
    }
}
