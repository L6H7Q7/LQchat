package com.lanchat.app

import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.ceil
import kotlin.math.floor

/** Calendar calculations only: no Activity, service, database or Android runtime dependency. */
data class CustomReminder(
    val id: String, val title: String, val content: String,
    val mode: String, val date: String, val time: String,
    val weekday: Int, val monthday: Int, val weekdays: Set<Int>, val monthdays: Set<Int>,
    val countdown: Boolean, val dayGap: Int, val dayTime: String,
    val near: Boolean, val nearStart: Int, val nearGap: Int,
)

data class ReminderOptions(val repeat: Int = 1, val repeatGap: Int = 0)
data class ReminderPush(val reminder: CustomReminder, val eventAt: Long, val at: Long, val kind: String, val index: Int) {
    val key: String get() = "${reminder.id}:$eventAt:$kind:$at:$index"
    fun text(): String = if (kind == "event") reminder.content
        else "距离下次${reminder.title}还有${CustomReminderSchedule.remaining(eventAt, at)}"
}

object CustomReminderSchedule {
    private fun calendar(at: Long, zone: TimeZone) = Calendar.getInstance(zone).apply { timeInMillis = at }
    fun dateKey(at: Long, zone: TimeZone = TimeZone.getDefault()): String =
        SimpleDateFormat("yyyy-MM-dd", Locale.ROOT).apply { timeZone = zone }.format(Date(at))

    fun parseDate(date: String, zone: TimeZone = TimeZone.getDefault()): Long? = runCatching {
        if (!Regex("\\d{4}-\\d{2}-\\d{2}").matches(date)) return null
        val format = SimpleDateFormat("yyyy-MM-dd", Locale.ROOT).apply { timeZone = zone; isLenient = false }
        format.parse(date)?.time
    }.getOrNull()

    fun validTime(time: String) = Regex("(?:[01]\\d|2[0-3]):[0-5]\\d").matches(time)
    private fun atTime(day: Long, time: String, zone: TimeZone): Long {
        val (h, m) = time.split(':').map(String::toInt)
        return calendar(day, zone).apply {
            set(Calendar.HOUR_OF_DAY, h); set(Calendar.MINUTE, m); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
        }.timeInMillis
    }
    private fun addDays(at: Long, count: Int, zone: TimeZone): Long =
        calendar(at, zone).apply { add(Calendar.DAY_OF_MONTH, count) }.timeInMillis

    fun matches(r: CustomReminder, at: Long, zone: TimeZone = TimeZone.getDefault()): Boolean {
        val c = calendar(at, zone)
        val weekday = c.get(Calendar.DAY_OF_WEEK) - 1
        val day = c.get(Calendar.DAY_OF_MONTH)
        return when (r.mode) {
            "daily" -> true
            "weekly" -> weekday == r.weekday
            "monthly" -> day == r.monthday
            "once" -> dateKey(at, zone) == r.date
            "weekdays" -> weekday in r.weekdays
            "monthdays" -> day in r.monthdays
            else -> false
        }
    }

    fun nextEvent(r: CustomReminder, from: Long, zone: TimeZone = TimeZone.getDefault()): Long? {
        if (r.mode == "once") return parseDate(r.date, zone)?.let { atTime(it, r.time, zone) }?.takeIf { it >= from }
        val c = calendar(from, zone)
        repeat(370) {
            val at = atTime(c.timeInMillis, r.time, zone)
            if (at >= from && matches(r, at, zone)) return at
            c.add(Calendar.DAY_OF_MONTH, 1)
        }
        return null
    }

    fun remaining(event: Long, at: Long): String {
        val minutes = ceil((event - at).coerceAtLeast(1) / 60000.0).toLong().coerceAtLeast(1)
        return listOf(minutes / 1440 to "天", minutes % 1440 / 60 to "小时", minutes % 60 to "分钟")
            .filter { it.first > 0 }.joinToString(" ") { "${it.first}${it.second}" }
    }

    /** Includes all ties, so simultaneous reminders are never dropped by a single timestamp cursor. */
    fun next(reminders: List<CustomReminder>, options: ReminderOptions, from: Long,
             delivered: Set<String> = emptySet(), zone: TimeZone = TimeZone.getDefault()): List<ReminderPush> {
        val candidates = mutableListOf<ReminderPush>()
        val span = (options.repeat - 1) * options.repeatGap * 1000L
        fun offer(r: CustomReminder, event: Long, base: Long, kind: String, end: Long = Long.MAX_VALUE) {
            repeat(options.repeat) { i ->
                val at = base + i * options.repeatGap * 1000L
                val p = ReminderPush(r, event, at, kind, i + 1)
                if (at >= from && at < end && p.key !in delivered) candidates.add(p)
            }
        }
        for (r in reminders) {
            // A previous event may still be in its configured repeat sequence.
            var event = nextEvent(r, from - span, zone)
            while (event != null && event < from) {
                offer(r, event, event, "event")
                event = nextEvent(r, event + 1, zone)
            }
            if (event == null) continue
            offer(r, event, event, "event")
            if (!r.countdown) continue
            val nearAt = if (r.near) event - r.nearStart * 60000L else event
            // Anchor day intervals backwards from the event's calendar date, preserving local wall time.
            val day = calendar(event, zone).apply {
                set(Calendar.HOUR_OF_DAY, 12); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
            }
            fun ordinal(at: Long): Long {
                val local = calendar(at, zone)
                return Calendar.getInstance(TimeZone.getTimeZone("UTC")).apply {
                    clear(); set(local.get(Calendar.YEAR), local.get(Calendar.MONTH), local.get(Calendar.DAY_OF_MONTH))
                }.timeInMillis / 86400000L
            }
            // Start close to today even for a one-off event many years away.
            val daysAway = (ordinal(event) - ordinal(from)).coerceAtLeast(0)
            val intervals = (daysAway / r.dayGap).toInt().coerceAtLeast(0)
            day.add(Calendar.DAY_OF_MONTH, -((intervals - 1).coerceAtLeast(0) * r.dayGap))
            var steps = 0
            val earliest = from - span - 2 * 86400000L
            while (day.timeInMillis >= earliest && steps < 4000) {
                val at = atTime(day.timeInMillis, r.dayTime, zone)
                if (at < nearAt) offer(r, event, at, "countdown", nearAt)
                day.add(Calendar.DAY_OF_MONTH, -r.dayGap); steps++
            }
            if (r.near) {
                val interval = r.nearGap * 60000L
                val first = floor((from - span - nearAt).toDouble() / interval).toLong().coerceAtLeast(0)
                val last = ceil((from - nearAt).toDouble() / interval).toLong().coerceAtLeast(0)
                for (i in first..last) {
                    val at = nearAt + i * interval
                    if (at < event) offer(r, event, at, "near", event)
                }
            }
        }
        val earliest = candidates.minOfOrNull { it.at } ?: return emptyList()
        return candidates.filter { it.at == earliest }.distinctBy { it.key }.sortedBy { it.key }
    }
}
