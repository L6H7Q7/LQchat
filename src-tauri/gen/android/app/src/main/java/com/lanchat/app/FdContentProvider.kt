package com.lanchat.app

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import java.io.File
import java.io.FileNotFoundException

class FdContentProvider : ContentProvider() {

    companion object {
        @JvmStatic
        external fun nativeGetClonedFd(msgId: Long): Int

        @JvmStatic
        external fun nativeGetFileSize(msgId: Long): Long
    }

    override fun onCreate(): Boolean = true

    /**
     * 兼容早期版本保存在 app data/received_files 下的接收文件。
     * URI 只携带文件名，且 canonical path 必须直属于 received_files，避免路径穿越。
     */
    private fun resolveStoredFile(uri: Uri): File {
        val fileName = uri.pathSegments.getOrNull(1)
            ?: throw FileNotFoundException("Stored file name is missing")
        val appContext = context ?: throw FileNotFoundException("Provider context is unavailable")
        val root = File(appContext.applicationInfo.dataDir, "received_files").canonicalFile
        val file = File(root, fileName).canonicalFile
        if (file.parentFile != root || !file.isFile) {
            throw FileNotFoundException("Stored file does not exist: $fileName")
        }
        return file
    }

    /**
     * 应用自己创建的 outbox 发送副本（app data/outbox）。
     * FileProvider 的 file_paths.xml 只映射 files/cache/external，且 applicationId 在
     * 验收构建中会变化，无法用固定路径映射；这里用运行时 dataDir 解析，只暴露该目录。
     */
    private fun resolveOutboxFile(uri: Uri): File {
        val fileName = uri.pathSegments.getOrNull(1)
            ?: throw FileNotFoundException("Outbox file name is missing")
        val appContext = context ?: throw FileNotFoundException("Provider context is unavailable")
        val root = File(appContext.applicationInfo.dataDir, "outbox").canonicalFile
        val file = File(root, fileName).canonicalFile
        if (file.parentFile != root || !file.isFile) {
            throw FileNotFoundException("Outbox file does not exist: $fileName")
        }
        return file
    }

    /**
     * 第三方 App 读取文件时触发。
     * URI 格式: content://com.lanchat.app.fdprovider/{msg_id}/{file_name}
     * 或 content://com.lanchat.app.fdprovider/outbox/{file_name}
     */
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? {
        val msgIdStr = uri.pathSegments.firstOrNull()
            ?: throw FileNotFoundException("URI path is empty")
        if (msgIdStr == "stored") {
            if (mode != "r") throw FileNotFoundException("Stored files are read-only")
            val file = resolveStoredFile(uri)
            println("[FdContentProvider] open stored file: ${file.name}")
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        }
        if (msgIdStr == "outbox") {
            if (mode != "r") throw FileNotFoundException("Outbox files are read-only")
            val file = resolveOutboxFile(uri)
            println("[FdContentProvider] open outbox file: ${file.name}")
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        }
        val msgId = msgIdStr.toLongOrNull()
            ?: throw FileNotFoundException("Invalid msgId: $msgIdStr")

        println("[FdContentProvider] openFile: msgId=$msgId")

        val fd = nativeGetClonedFd(msgId)
        if (fd < 0) {
            println("[FdContentProvider] FD not found for msgId=$msgId")
            throw FileNotFoundException("File expired or app was restarted")
        }

        println("[FdContentProvider] adoptFd: fd=$fd for msgId=$msgId")
        return ParcelFileDescriptor.adoptFd(fd)
    }

    /**
     * 其他 App（包括 LANChat 自己）查询文件元数据时触发。
     * 从 Rust FD 缓存获取真实文件名和大小。
     */
    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?
    ): Cursor? {
        val msgIdStr = uri.pathSegments.firstOrNull() ?: return null
        val fileName = uri.lastPathSegment ?: "shared_file"
        val fileSize = if (msgIdStr == "stored") {
            try {
                resolveStoredFile(uri).length()
            } catch (_: FileNotFoundException) {
                return null
            }
        } else if (msgIdStr == "outbox") {
            try {
                resolveOutboxFile(uri).length()
            } catch (_: FileNotFoundException) {
                return null
            }
        } else {
            val msgId = msgIdStr.toLongOrNull() ?: return null
            nativeGetFileSize(msgId).also {
                if (it < 0) return null // 缓存已失效
            }
        }

        val proj = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val cursor = MatrixCursor(proj)
        val row = cursor.newRow()

        for (col in proj) {
            when (col) {
                OpenableColumns.DISPLAY_NAME -> row.add(fileName)
                OpenableColumns.SIZE -> row.add(fileSize)
                else -> row.add(null)
            }
        }
        return cursor
    }

    override fun getType(uri: Uri): String? {
        val fileName = uri.lastPathSegment ?: return "*/*"
        val ext = fileName.substringAfterLast('.', "")
        if (ext.isEmpty()) return "*/*"
        return MimeTypeMap.getSingleton()
            .getMimeTypeFromExtension(ext.lowercase()) ?: "*/*"
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0
}
