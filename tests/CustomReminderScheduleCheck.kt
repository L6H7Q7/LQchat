package com.lanchat.app

import java.text.SimpleDateFormat
import java.util.TimeZone

class CustomReminderScheduleCheck {
    private fun assertTrue(value: Boolean) = check(value)
    private fun assertFalse(value: Boolean) = check(!value)
    private fun assertNull(value: Any?) = check(value == null) { "Expected null, got $value" }
    private fun assertEquals(expected: Any?, actual: Any?) = check(expected == actual) { "Expected $expected, got $actual" }
    private val zone = TimeZone.getTimeZone("Asia/Shanghai")
    private fun at(value: String): Long = SimpleDateFormat("yyyy-MM-dd HH:mm:ss").apply { timeZone = zone }.parse(value)!!.time
    private fun reminder(id: String = "harvest") = CustomReminder(id, "收菜", "记得收菜", "once", "2026-10-06", "18:00",
        6, 3, setOf(1,3,5), setOf(1,15), true, 1, "09:00", true, 60, 10)
    private fun next(r: CustomReminder, now: String, options: ReminderOptions = ReminderOptions(3, 5)) =
        CustomReminderSchedule.next(listOf(r), options, at(now), zone = zone).firstOrNull()

    fun dateModesAndMonthEnd() {
        val saturday = at("2026-10-03 12:00:00")
        assertTrue(CustomReminderSchedule.matches(reminder().copy(mode="daily"), saturday, zone))
        assertTrue(CustomReminderSchedule.matches(reminder().copy(mode="weekly",weekday=6), saturday, zone))
        assertFalse(CustomReminderSchedule.matches(reminder().copy(mode="weekdays"), saturday, zone))
        assertTrue(CustomReminderSchedule.matches(reminder().copy(mode="monthdays",monthdays=setOf(3)), saturday, zone))
        assertFalse(CustomReminderSchedule.matches(reminder(), saturday, zone))
        val monthly = reminder().copy(mode="monthly",monthday=31,countdown=false)
        assertEquals(at("2026-05-31 18:00:00"), CustomReminderSchedule.nextEvent(monthly,at("2026-04-01 00:00:00"),zone))
        assertNull(CustomReminderSchedule.parseDate("2026-02-30",zone))
        assertEquals(at("2028-02-29 18:00:00"),CustomReminderSchedule.nextEvent(reminder().copy(date="2028-02-29"),saturday,zone))
    }
    fun countdownNearAndEventOrdering() {
        assertEquals("countdown",next(reminder(),"2026-10-03 12:00:00")!!.kind)
        assertEquals(at("2026-10-04 09:00:00"),next(reminder(),"2026-10-03 12:00:00")!!.at)
        assertEquals(at("2026-10-06 17:30:00"),next(reminder(),"2026-10-06 17:23:00")!!.at)
        assertEquals("near",next(reminder(),"2026-10-06 17:23:00")!!.kind)
        assertEquals(at("2026-10-06 18:00:00"),next(reminder(),"2026-10-06 17:59:00")!!.at)
        assertEquals("event",next(reminder(),"2026-10-06 17:59:00")!!.kind)
        assertNull(next(reminder(),"2026-10-06 18:00:11"))
    }
    fun globalRepeatsAndDeduplication() {
        val r=reminder()
        val second=next(r,"2026-10-06 18:00:02")!!
        assertEquals(2,second.index);assertEquals(at("2026-10-06 18:00:05"),second.at)
        val suppressed=CustomReminderSchedule.next(listOf(r),ReminderOptions(3,5),second.at,setOf(second.key),zone).first()
        assertEquals(3,suppressed.index)
        val near=next(r,"2026-10-06 17:20:02")!!
        assertEquals(2,near.index);assertEquals(at("2026-10-06 17:20:05"),near.at)
        assertEquals(1,next(r,"2026-10-06 17:23:00",ReminderOptions(1,30))!!.index)
    }
    fun simultaneousEventsAndRecurrentRollover() {
        val r=reminder().copy(countdown=false)
        val batch=CustomReminderSchedule.next(listOf(r,r.copy(id="other")),ReminderOptions(),at("2026-10-06 17:59:00"),zone=zone)
        assertEquals(2,batch.size)
        val daily=r.copy(mode="daily")
        assertEquals(at("2026-10-07 18:00:00"),next(daily,"2026-10-06 18:01:00")!!.at)
        val twoDays=reminder().copy(dayGap=2,near=false)
        assertEquals(at("2026-10-04 09:00:00"),next(twoDays,"2026-10-03 12:00:00")!!.at)
    }
    fun sharedDefaultsAndLongIntervals() {
        assertEquals(ReminderOptions(1,0), ReminderOptions())
        val r=reminder().copy(countdown=false)
        val event=at("2026-10-06 18:00:00")
        assertEquals(1,CustomReminderSchedule.next(listOf(r),ReminderOptions(),event,zone=zone).single().index)
        assertTrue(CustomReminderSchedule.next(listOf(r),ReminderOptions(),event+1,zone=zone).isEmpty())
        val options=ReminderOptions(20,3600)
        val first=CustomReminderSchedule.next(listOf(r),options,event,zone=zone).single()
        val second=CustomReminderSchedule.next(listOf(r),options,event+1,setOf(first.key),zone).single()
        assertEquals(event+3600000,second.at)
        assertEquals(2,second.index)
    }
    fun timezoneAndDaylightSavingPreserveWallClock() {
        val ny=TimeZone.getTimeZone("America/New_York")
        val f=SimpleDateFormat("yyyy-MM-dd HH:mm:ss").apply{timeZone=ny}
        val r=reminder().copy(mode="daily",time="09:00",countdown=false)
        assertEquals(f.parse("2026-03-08 09:00:00")!!.time,
            CustomReminderSchedule.nextEvent(r,f.parse("2026-03-07 10:00:00")!!.time,ny))
    }

    fun farFutureCountdown() {
        val r=reminder().copy(date="2050-10-06",near=false)
        val next=next(r,"2026-10-03 12:00:00")!!
        assertEquals(at("2026-10-04 09:00:00"),next.at)
    }
    fun forwardingPolicyAndDeviceTitle() {
        assertFalse(CustomReminderForwarding.enabled(false,true,setOf("pc")))
        assertFalse(CustomReminderForwarding.enabled(true,false,setOf("pc")))
        assertFalse(CustomReminderForwarding.enabled(true,true,emptySet()))
        assertFalse(CustomReminderForwarding.enabled(true,true,setOf("  ")))
        assertTrue(CustomReminderForwarding.enabled(true,true,setOf("pc","phone")))
        assertEquals("IQOO · 收菜",CustomReminderForwarding.title(" IQOO "," 收菜 "))
        assertEquals("本机 · 收菜",CustomReminderForwarding.title("  ","收菜"))
        assertEquals("IQOO · 收菜",CustomReminderForwarding.title("IQOO","IQOO · 收菜"))
        assertEquals("收菜",CustomReminderForwarding.remoteTitle("  ","收菜"))
        assertEquals("IQOO · 收菜",CustomReminderForwarding.remoteTitle(" IQOO ","收菜"))
        val push=next(reminder(),"2026-10-03 12:00:00")!!
        assertTrue(push.text().startsWith("距离下次收菜还有"))
    }
}

fun main() {
    CustomReminderScheduleCheck().apply {
        dateModesAndMonthEnd();countdownNearAndEventOrdering();globalRepeatsAndDeduplication()
        simultaneousEventsAndRecurrentRollover();sharedDefaultsAndLongIntervals();timezoneAndDaylightSavingPreserveWallClock();farFutureCountdown();forwardingPolicyAndDeviceTitle()
    }
    println("PASS: six date modes, month-end/leap-day, countdown/near/event order, global repeat intervals, deduplication, simultaneous events, next cycle, DST, far-future countdown, forwarding gate and device-title format.")
}
