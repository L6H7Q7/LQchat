package com.lanchat.app

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import androidx.annotation.Keep
import java.io.File
import java.io.FileInputStream
import java.net.URLConnection

/**
 * Android 10+ 公共目录写入桥接。
 *
 * 网络核心只写应用专属 staging 文件；这里使用用户通过 ACTION_OPEN_DOCUMENT_TREE
 * 授予的持久化权限，将完整文件复制到目标目录。
 */
@Keep
object AndroidDownloadStore {
    @Volatile
    private var appContext: Context? = null

    @JvmStatic
    fun initialize(context: Context) {
        appContext = context.applicationContext
    }

    @JvmStatic
    @Keep
    fun exportReceivedFile(treeUri: String, sourcePath: String, fileName: String): String {
        return try {
            val context = appContext ?: return "ERROR:Android 文件存储尚未初始化"
            val source = File(sourcePath)
            if (!source.isFile) return "ERROR:待导出的接收文件不存在"

            val resolver = context.contentResolver
            val tree = Uri.parse(treeUri)
            val parentId = DocumentsContract.getTreeDocumentId(tree)
            val parent = DocumentsContract.buildDocumentUriUsingTree(tree, parentId)
            val mimeType = URLConnection.guessContentTypeFromName(fileName)
                ?: "application/octet-stream"
            val target = DocumentsContract.createDocument(resolver, parent, mimeType, fileName)
                ?: return "ERROR:无法在所选文件夹中创建文件"

            try {
                FileInputStream(source).use { input ->
                    resolver.openOutputStream(target, "w")?.use { output ->
                        input.copyTo(output)
                        output.flush()
                    } ?: throw IllegalStateException("无法打开目标文件写入流")
                }
            } catch (error: Exception) {
                runCatching { DocumentsContract.deleteDocument(resolver, target) }
                throw error
            }

            target.toString()
        } catch (error: Exception) {
            "ERROR:${error.message ?: error.javaClass.simpleName}"
        }
    }

    /**
     * 删除本应用导出到用户所选文件夹中的文件（清除聊天数据、迁移后清理旧副本）。
     * 只应在确认该文档确实由 LANChat 创建时调用。
     */
    @JvmStatic
    @Keep
    fun deleteExportedFile(uri: String): String {
        return try {
            val context = appContext ?: return "ERROR:Android 文件存储尚未初始化"
            val target = Uri.parse(uri)
            val deleted = DocumentsContract.deleteDocument(context.contentResolver, target)
            if (deleted) "OK" else "ERROR:系统未删除该文件"
        } catch (error: Exception) {
            "ERROR:${error.message ?: error.javaClass.simpleName}"
        }
    }
}
