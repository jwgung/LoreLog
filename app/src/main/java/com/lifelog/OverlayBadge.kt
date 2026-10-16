package com.lifelog

import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.provider.Settings
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.TextView
import kotlin.math.abs

/**
 * 백그라운드로 기록 중일 때 다른 앱 위에 뜨는 작은 "감시중" 배지.
 * 드래그로 옮기고, 탭하면 앱을 연다. '다른 앱 위에 표시' 권한이 있어야 뜬다.
 */
object OverlayBadge {
    private var view: View? = null

    fun canShow(context: Context): Boolean = Settings.canDrawOverlays(context.applicationContext)

    fun show(context: Context) {
        val app = context.applicationContext
        if (view != null) return
        if (!canShow(app)) return
        val wm = app.getSystemService(Context.WINDOW_SERVICE) as WindowManager

        val badge = TextView(app).apply {
            text = app.getString(R.string.overlay_watching)
            setTextColor(Color.WHITE)
            textSize = 12f
            setPadding(28, 16, 28, 16)
            background = GradientDrawable().apply {
                setColor(0xDD1C1C1E.toInt())
                cornerRadius = 60f
            }
        }

        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O)
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE

        val lp = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            type,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = 32
            y = 240
        }

        var downX = 0f; var downY = 0f; var startX = 0; var startY = 0; var moved = false
        badge.setOnTouchListener { _, e ->
            when (e.action) {
                MotionEvent.ACTION_DOWN -> {
                    downX = e.rawX; downY = e.rawY; startX = lp.x; startY = lp.y; moved = false; true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = (e.rawX - downX).toInt(); val dy = (e.rawY - downY).toInt()
                    if (abs(dx) > 12 || abs(dy) > 12) moved = true
                    lp.x = startX + dx; lp.y = startY + dy
                    try { wm.updateViewLayout(badge, lp) } catch (_: Exception) {}
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (!moved) {
                        app.startActivity(
                            Intent(app, MainActivity::class.java)
                                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                        )
                    }
                    true
                }
                else -> false
            }
        }

        try {
            wm.addView(badge, lp)
            view = badge
        } catch (_: Exception) {
            view = null
        }
    }

    fun hide(context: Context) {
        val app = context.applicationContext
        val wm = app.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        view?.let { try { wm.removeView(it) } catch (_: Exception) {} }
        view = null
    }
}
