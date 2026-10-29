package com.lorelog

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/**
 * Lucide 아이콘(https://lucide.dev, ISC 라이선스). 원본 SVG(lucide-static 0.468.0)를 그대로 옮긴 것이고 직접 그린 그림이 아니다.
 * 선 두께는 앱의 가는 선 톤에 맞춰 1.75 로 쓴다. 색은 Icon(tint)로 정한다.
 */
internal object Lucide {
    private const val STROKE = 1.75f

    private fun icon(name: String, vararg paths: String): ImageVector =
        ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
            for (d in paths) {
                addPath(
                    pathData = PathParser().parsePathString(d).toNodes(),
                    stroke = SolidColor(Color.Black),
                    strokeLineWidth = STROKE,
                    strokeLineCap = StrokeCap.Round,
                    strokeLineJoin = StrokeJoin.Round,
                )
            }
        }.build()

    val Settings: ImageVector by lazy {
        icon("Settings",
            "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
            "M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0")
    }

    val Archive: ImageVector by lazy {
        icon("Archive",
            "M3 3h18a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-18a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1Z",
            "M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8",
            "M10 12h4")
    }

    val Chat: ImageVector by lazy {
        icon("Chat",
            "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
            "M13 8H7",
            "M17 12H7")
    }

    val Back: ImageVector by lazy {
        icon("Back",
            "m12 19-7-7 7-7",
            "M19 12H5")
    }

    val Mic: ImageVector by lazy {
        icon("Mic",
            "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z",
            "M19 10v2a7 7 0 0 1-14 0v-2",
            "M12 19L12 22")
    }

    val Up: ImageVector by lazy {
        icon("Up",
            "m5 12 7-7 7 7",
            "M12 19V5")
    }

    val Stop: ImageVector by lazy {
        icon("Stop",
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z")
    }


    val Todo: ImageVector by lazy {
        icon("Todo",
            "m3 17 2 2 4-4",
            "m3 7 2 2 4-4",
            "M13 6h8",
            "M13 12h8",
            "M13 18h8")
    }

    val Close: ImageVector by lazy {
        icon("Close",
            "M18 6 6 18",
            "m6 6 12 12")
    }

    val Check: ImageVector by lazy {
        icon("Check",
            "M20 6 9 17l-5-5")
    }
}
