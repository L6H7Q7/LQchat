package com.lanchat.app

internal object NotificationAllowedPackages {
    /** Stale saved entries may be removed; newly submitted invalid entries still fail validation. */
    fun reconcile(requested: Set<String>, previous: Set<String>, eligible: Set<String>): Set<String> {
        require(requested.all { it in eligible || it in previous }) {
            "允许列表包含未安装或不可选择的应用"
        }
        return requested.intersect(eligible)
    }
}
