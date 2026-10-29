package com.lorelog

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** 앱 전체의 색·모서리·글자 크기. 폰 배경색(동적 색)을 따르지 않고 목업의 톤으로 고정한다. */
internal val LifeColors = darkColorScheme(
    primary = Color(0xFF9DB7FF), onPrimary = Color(0xFF0E1630),
    primaryContainer = Color(0xFF2A3350), onPrimaryContainer = Color(0xFFD9E3FF),
    secondary = Color(0xFFB8C4E0), onSecondary = Color(0xFF1B2538),
    secondaryContainer = Color(0xFF2A3140), onSecondaryContainer = Color(0xFFDDE3F2),
    tertiary = Color(0xFFE8B86A), onTertiary = Color(0xFF2B1D00),
    background = Color(0xFF14161B), onBackground = Color(0xFFE8EAEE),
    surface = Color(0xFF14161B), onSurface = Color(0xFFE8EAEE),
    surfaceVariant = Color(0xFF1C2027), onSurfaceVariant = Color(0xFF9AA1AD),
    outline = Color(0xFF3A4150), outlineVariant = Color(0xFF2B313B),
    error = Color(0xFFE3A3B8), onError = Color(0xFF3A1522),
    errorContainer = Color(0xFF2E2127), onErrorContainer = Color(0xFFF0C8D3),
)

internal val LifeShapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp),
)

internal val LifeTypography = Typography(
    headlineSmall = TextStyle(fontSize = 22.sp, lineHeight = 30.sp, fontWeight = FontWeight.Medium),
    titleSmall = TextStyle(fontSize = 15.sp, lineHeight = 22.sp, fontWeight = FontWeight.Medium),
    bodyLarge = TextStyle(fontSize = 16.sp, lineHeight = 25.sp),
    bodyMedium = TextStyle(fontSize = 15.sp, lineHeight = 23.sp),
    bodySmall = TextStyle(fontSize = 13.sp, lineHeight = 20.sp),
    labelLarge = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
)
