package com.lifelog.data

/** 타임라인 한 줄. 종류가 달라도 시간순으로 함께 그린다. */
sealed interface TimelineItem {
    val timestampMs: Long

    data class Voice(val seg: VoiceSegment) : TimelineItem {
        override val timestampMs: Long get() = seg.timestampMs
    }

    data class App(val event: AppEvent) : TimelineItem {
        override val timestampMs: Long get() = event.timestampMs
    }

    data class Screen(val event: ScreenContentEvent) : TimelineItem {
        override val timestampMs: Long get() = event.timestampMs
    }

    data class Location(val point: LocationPoint) : TimelineItem {
        override val timestampMs: Long get() = point.timestampMs
    }

    data class Notification(val event: NotificationEvent) : TimelineItem {
        override val timestampMs: Long get() = event.timestampMs
    }

    data class Comm(val event: CommEvent) : TimelineItem {
        override val timestampMs: Long get() = event.timestampMs
    }

    data class Calendar(val entry: CalendarEntry) : TimelineItem {
        override val timestampMs: Long get() = entry.timestampMs
    }
}
