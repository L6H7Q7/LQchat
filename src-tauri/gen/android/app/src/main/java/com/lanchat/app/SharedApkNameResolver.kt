package com.lanchat.app

import android.content.Context
import android.net.Uri
import java.io.File

/** Resolve the label from the shared APK itself, without trusting launcher-specific extras. */
internal object SharedApkNameResolver {
    fun resolve(context: Context, uri: Uri, originalName: String, mimeType: String): String {
        if (!originalName.endsWith(".apk", ignoreCase = true) &&
            !mimeType.equals("application/vnd.android.package-archive", ignoreCase = true)) {
            return originalName
        }

        // FileProvider descriptors usually point at ordinary files. Try this before copying an APK.
        val directName = runCatching {
            context.contentResolver.openFileDescriptor(uri, "r")?.use { descriptor ->
                readLabel(context, "/proc/self/fd/${descriptor.fd}")
            }
        }.getOrNull()
        if (directName != null) return directName

        // Providers backed by pipes or virtual documents need a seekable private cache file.
        var temporary: File? = null
        return try {
            val archive = File.createTempFile("shared-apk-label-", ".apk", context.cacheDir)
            temporary = archive
            context.contentResolver.openInputStream(uri)?.use { input ->
                archive.outputStream().use { output -> input.copyTo(output) }
            } ?: return originalName
            readLabel(context, archive.absolutePath) ?: originalName
        } catch (error: Exception) {
            android.util.Log.w("SharedApkNameResolver", "无法解析分享 APK 名称，保留原文件名", error)
            originalName
        } finally {
            temporary?.let { file ->
                if (!file.delete()) android.util.Log.w("SharedApkNameResolver", "未能清理 APK 名称解析缓存")
            }
        }
    }

    private fun readLabel(context: Context, path: String): String? {
        @Suppress("DEPRECATION")
        val archive = context.packageManager.getPackageArchiveInfo(path, 0) ?: return null
        val info = archive.applicationInfo ?: return null
        info.sourceDir = path
        info.publicSourceDir = path
        // An APK without a label falls back to its package ID; preserve the original filename then.
        if (info.labelRes == 0 && info.nonLocalizedLabel.isNullOrBlank()) return null
        return SharedApkFileName.fromLabel(context.packageManager.getApplicationLabel(info).toString())
    }
}
