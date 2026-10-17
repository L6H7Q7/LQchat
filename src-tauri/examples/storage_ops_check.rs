//! 存储清理/迁移的可运行验证（只使用隔离的临时目录与临时数据库）。
//!
//! 用法：`cargo run --example storage_ops_check`
//! 本机 `cargo test` 的测试二进制无法启动（0xC0000139），因此用示例程序跑同一批断言。

use lanchat::{db, storage_ops};
use std::path::{Path, PathBuf};

fn temp_dir(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("lqchat-check-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

async fn fixture_pool(root: &Path) -> sqlx::Pool<sqlx::Sqlite> {
    db::init_db_with_path(root.to_path_buf())
        .await
        .expect("初始化临时数据库失败")
}

async fn insert_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender: &str,
    receiver: &str,
    path: &Path,
) {
    sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
         VALUES (?, ?, 'file.bin', 'file', unixepoch(), ?, 'accepted')",
    )
    .bind(sender)
    .bind(receiver)
    .bind(path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .unwrap();
}

struct Harness {
    passed: usize,
    failed: usize,
}

impl Harness {
    fn new() -> Self {
        Self { passed: 0, failed: 0 }
    }
    fn check(&mut self, name: &str, ok: bool, detail: String) {
        if ok {
            self.passed += 1;
            println!("PASS {name} :: {detail}");
        } else {
            self.failed += 1;
            println!("FAIL {name} :: {detail}");
        }
    }
}

#[tokio::main]
async fn main() {
    let mut harness = Harness::new();

    // 1) 迁移：应用管理的文件搬到下载位置，记录同步更新，原文件删除
    {
        let root = temp_dir("migrate-app");
        let staging = root.join("staging");
        let target = temp_dir("migrate-target");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("report.bin");
        std::fs::write(&source, b"lqchat-migration-payload").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "migration_moves_file",
            report.failures.is_empty()
                && report.migrated == 1
                && !source.exists()
                && std::fs::read(target.join("report.bin")).unwrap() == b"lqchat-migration-payload"
                && PathBuf::from(&stored) == target.join("report.bin"),
            format!("migrated={} failures={:?}", report.migrated, report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // 2) 迁移：同名文件自动改名，绝不覆盖已有文件
    {
        let root = temp_dir("migrate-conflict");
        let staging = root.join("staging");
        let target = temp_dir("migrate-conflict-target");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("photo.png");
        std::fs::write(&source, b"new-content").unwrap();
        std::fs::write(target.join("photo.png"), b"existing-content").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        harness.check(
            "migration_conflict_renames",
            report.failures.is_empty()
                && std::fs::read(target.join("photo.png")).unwrap() == b"existing-content"
                && std::fs::read(target.join("photo (1).png")).unwrap() == b"new-content"
                && !source.exists(),
            format!("failures={:?}", report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // 3) 迁移失败：原文件与记录都保留，可重试
    {
        let root = temp_dir("migrate-fail");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("keep.bin");
        std::fs::write(&source, b"must-not-be-lost").unwrap();
        let blocked = root.join("blocked-target");
        std::fs::write(&blocked, b"x").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &blocked.to_string_lossy())
                .await;
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "migration_failure_keeps_source",
            !report.failures.is_empty()
                && report.migrated == 0
                && source.exists()
                && PathBuf::from(stored) == source,
            format!("failures={:?}", report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // 4) 迁移：上次遗留的原文件在下次运行时重试清理
    {
        let root = temp_dir("migrate-pending");
        let staging = root.join("staging");
        let target = temp_dir("migrate-pending-target");
        std::fs::create_dir_all(&staging).unwrap();
        let leftover = staging.join("leftover.bin");
        std::fs::write(&leftover, b"leftover").unwrap();
        std::fs::write(target.join("leftover.bin"), b"leftover").unwrap();
        let pool = fixture_pool(&root).await;
        db::add_pending_cleanup(&pool, &leftover.to_string_lossy()).await.unwrap();
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        harness.check(
            "migration_retries_pending_cleanup",
            report.failures.is_empty()
                && report.pending_cleaned == 1
                && !leftover.exists()
                && db::get_pending_cleanup(&pool).await.is_empty(),
            format!("pending_cleaned={} failures={:?}", report.pending_cleaned, report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // 5) 清理：接收文件删除；发送源与共享路径保留
    {
        let root = temp_dir("cleanup-app");
        let staging = root.join("staging");
        let download = temp_dir("cleanup-download");
        std::fs::create_dir_all(&staging).unwrap();
        let received = download.join("received.apk");
        std::fs::write(&received, b"received").unwrap();
        let sent_dir = temp_dir("cleanup-user-source");
        let user_file = sent_dir.join("original.apk");
        std::fs::write(&user_file, b"user-source").unwrap();
        let shared = staging.join("shared.bin");
        std::fs::write(&shared, b"shared").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &received).await;
        insert_message(&pool, "me", "iqoo", &user_file).await;
        insert_message(&pool, "iqoo", "me", &shared).await;
        insert_message(&pool, "other", "me", &shared).await;
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_scope",
            outcome.report.failures.is_empty()
                && !received.exists()
                && user_file.exists()
                && shared.exists()
                && outcome.report.deleted == 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&download).ok();
        std::fs::remove_dir_all(&sent_dir).ok();
    }

    // 6) 清理：位于下载目录的“发送源”也不能删
    {
        let root = temp_dir("cleanup-source-dir");
        let staging = root.join("staging");
        let download = temp_dir("cleanup-source-download");
        std::fs::create_dir_all(&staging).unwrap();
        let picked = download.join("picked-by-user.apk");
        std::fs::write(&picked, b"user-picked").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "iqoo", &picked).await;
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_protects_send_source_in_download",
            picked.exists() && outcome.report.deleted == 0,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&download).ok();
    }

    // 7) 清理：大小写/别名相同的路径仍算共享引用
    {
        let root = temp_dir("cleanup-normalized");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let shared = staging.join("Shared-Case.bin");
        std::fs::write(&shared, b"shared").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &shared).await;
        let alias = shared.to_string_lossy().to_uppercase();
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('other', 'me', 'file.bin', 'file', unixepoch(), ?, 'accepted')",
        )
        .bind(&alias)
        .execute(&pool)
        .await
        .unwrap();
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_shared_normalized",
            shared.exists() && outcome.report.deleted == 0,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // 8) 清理：没有导出凭据的 content:// 文档保留
    {
        let root = temp_dir("cleanup-content");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let pool = fixture_pool(&root).await;
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('iqoo', 'me', 'doc.pdf', 'file', unixepoch(), 'content://user/docs/1', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_keeps_unknown_content_uri",
            outcome.report.failures.is_empty() && outcome.report.deleted == 0 && outcome.report.kept == 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // 9) 清理失败：如实上报失败，不返回“清理成功”
    {
        let root = temp_dir("cleanup-fail");
        let staging = root.join("staging");
        let download = temp_dir("cleanup-fail-download");
        std::fs::create_dir_all(&staging).unwrap();
        let blocked = download.join("blocked.apk");
        std::fs::create_dir_all(&blocked).unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &blocked).await;
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_reports_failure",
            outcome.report.failed() && outcome.report.deleted == 0 && blocked.exists(),
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&download).ok();
    }

    // 10) 数据库查询失败必须报错，而不是当成“没有文件”返回成功
    {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let result = storage_ops::cleanup_chat_files(&pool, "me", "iqoo", None, None, None).await;
        harness.check("cleanup_surfaces_db_error", result.is_err(), format!("is_err={}", result.is_err()));
        pool.close().await;
    }

    // 11) 迁移只更新一次即可覆盖所有引用同一路径的记录
    {
        let root = temp_dir("caption-app");
        let pool = fixture_pool(&root).await;
        let path = root.join("shared.bin");
        std::fs::write(&path, b"x").unwrap();
        insert_message(&pool, "me", "iqoo", &path).await;
        insert_message(&pool, "iqoo", "me", &path).await;
        let updated = db::update_file_paths_by_old_path(&pool, &path.to_string_lossy(), "/new/shared.bin")
            .await
            .unwrap();
        db::set_message_caption(&pool, 1, "一起发送的文字").await.unwrap();
        let caption: Option<String> = sqlx::query_scalar("SELECT caption FROM messages WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "caption_and_path_updates",
            updated == 2 && caption.as_deref() == Some("一起发送的文字"),
            format!("updated={updated} caption={caption:?}"),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 审查修正专项（1/8 组）：共享 content:// URI 不得进入释放列表 ──
    {
        let root = temp_dir("uri-shared");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let pool = fixture_pool(&root).await;
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('iqoo', 'me', 'doc.pdf', 'file', unixepoch(), 'content://tree-shared/docs/1', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO persisted_uris (uri, msg_id, created_at) VALUES ('content://tree-shared/docs/1', 1, unixepoch())")
            .execute(&pool)
            .await
            .unwrap();
        // 另一条聊天仍引用同一个 URI
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('other', 'me', 'doc.pdf', 'file', unixepoch(), 'content://tree-shared/docs/1', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "shared_content_uri_not_released",
            outcome.persisted_uris.is_empty() && outcome.report.deleted == 0,
            format!("release_list={:?} report={:?}", outcome.persisted_uris, outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 审查修正专项（1/8 组）：独占 URI 才进入释放列表（记录删除后才释放） ──
    {
        let root = temp_dir("uri-exclusive");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let pool = fixture_pool(&root).await;
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('iqoo', 'me', 'doc.pdf', 'file', unixepoch(), 'content://tree-exclusive/docs/9', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO persisted_uris (uri, msg_id, created_at) VALUES ('content://tree-exclusive/docs/9', 1, unixepoch())")
            .execute(&pool)
            .await
            .unwrap();
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "exclusive_content_uri_queued_for_release",
            outcome.persisted_uris.len() == 1
                && outcome.persisted_uris[0].1 == "content://tree-exclusive/docs/9"
                && outcome.report.deleted == 0,
            format!("release_list={:?}", outcome.persisted_uris),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 审查修正专项（1/8 组）：用户“发送”的 content:// 原始文档必须保留 ──
    {
        let root = temp_dir("uri-sent-source");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let pool = fixture_pool(&root).await;
        db::remember_download_tree(&pool, "content://tree-a").await.unwrap();
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('me', 'iqoo', 'doc.pdf', 'file', unixepoch(), 'content://tree-a/docs/user-original', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "sent_content_uri_source_is_kept",
            outcome.report.deleted == 0 && outcome.report.kept == 1 && outcome.report.failures.is_empty(),
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 审查修正专项（5/6/7 组）：下载目录历史（含旧目录）作为导出凭据 ──
    {
        let root = temp_dir("tree-history");
        let pool = fixture_pool(&root).await;
        db::update_download_path(&pool, "content://tree-old".to_string()).await.unwrap();
        db::update_download_path(&pool, "content://tree-new".to_string()).await.unwrap();
        db::update_download_path(&pool, "content://tree-new".to_string()).await.unwrap();
        let trees = db::get_download_trees(&pool).await;
        harness.check(
            "download_tree_history_keeps_old_and_dedups",
            trees.contains(&"content://tree-old".to_string())
                && trees.iter().filter(|item| *item == "content://tree-new").count() == 1,
            format!("trees={trees:?}"),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 审查修正专项（5/6/7 组）：正在传输的文件不会被迁移 ──
    {
        let root = temp_dir("migrate-inflight");
        let staging = root.join("staging");
        let target = temp_dir("migrate-inflight-target");
        std::fs::create_dir_all(&staging).unwrap();
        let in_flight_file = staging.join("sending.bin");
        std::fs::write(&in_flight_file, b"in-flight").unwrap();
        let pool = fixture_pool(&root).await;
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('me', 'iqoo', 'file.bin', 'file', unixepoch(), ?, 'uploading')",
        )
        .bind(in_flight_file.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .unwrap();
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "migration_skips_in_flight_files",
            report.migrated == 0
                && report.skipped >= 1
                && in_flight_file.exists()
                && PathBuf::from(stored) == in_flight_file,
            format!("migrated={} skipped={} failures={:?}", report.migrated, report.skipped, report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // ── 审查修正专项（9 组）：caption 与路径更新写入数据库并可读回 ──
    {
        let root = temp_dir("caption-chain");
        let pool = fixture_pool(&root).await;
        let path = root.join("chain.bin");
        std::fs::write(&path, b"x").unwrap();
        insert_message(&pool, "me", "iqoo", &path).await;
        insert_message(&pool, "iqoo", "me", &path).await;
        let updated = db::update_file_paths_by_old_path(&pool, &path.to_string_lossy(), "/moved/chain.bin")
            .await
            .unwrap();
        db::set_message_caption(&pool, 1, "图片下方的文字").await.unwrap();
        let caption: Option<String> = sqlx::query_scalar("SELECT caption FROM messages WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "caption_chain_persisted",
            updated == 2 && caption.as_deref() == Some("图片下方的文字"),
            format!("updated={updated} caption={caption:?}"),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 迁移来源判定（审查边界）：私有目录里的“用户发送源”不得被迁移/删除 ──
    {
        let root = temp_dir("migrate-user-source");
        let staging = root.join("staging");
        let target = temp_dir("migrate-user-source-target");
        std::fs::create_dir_all(&staging).unwrap();
        // 用户把原图放进应用私有目录，然后作为发送源使用（与 codex-qa-source.png 场景一致）。
        let user_source = root.join("codex-qa-source.png");
        std::fs::write(&user_source, b"user-original").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "iqoo", &user_source).await;
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "migration_skips_user_source_in_private_dir",
            report.migrated == 0
                && report.skipped >= 1
                && user_source.exists()
                && PathBuf::from(stored) == user_source,
            format!("migrated={} skipped={} failures={:?}", report.migrated, report.skipped, report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // ── 迁移来源判定：应用自己的 outbox 发送副本可以迁移并被清理 ──
    {
        let root = temp_dir("migrate-outbox-copy");
        let outbox = root.join("outbox");
        let staging = root.join("staging");
        let target = temp_dir("migrate-outbox-target");
        std::fs::create_dir_all(&outbox).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let copy = outbox.join("uuid_小游戏.apk");
        std::fs::write(&copy, b"apk-copy").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "iqoo", &copy).await;
        let report =
            storage_ops::migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy())
                .await;
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        harness.check(
            "migration_moves_outbox_copy",
            report.failures.is_empty()
                && report.migrated == 1
                && !copy.exists()
                && std::fs::read(target.join("uuid_小游戏.apk")).unwrap() == b"apk-copy"
                && PathBuf::from(&stored) == target.join("uuid_小游戏.apk"),
            format!("migrated={} failures={:?}", report.migrated, report.failures),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    // ── content:// 来源谓词（清理与迁移共用） ──
    {
        let trees = vec!["content://tree-a".to_string()];
        let exported = vec!["content://tree-a/docs/migrated".to_string()];
        let cases = [
            ("received_under_tree", "content://tree-a/docs/received", false, true, true),
            ("sent_user_document_in_tree", "content://tree-a/docs/user", true, false, false),
            ("exported_by_app_sent_only", "content://tree-a/docs/migrated", true, false, true),
            ("outside_any_tree", "content://other/docs/x", false, true, false),
        ];
        for (name, path, sent, received, expected) in cases {
            let actual = storage_ops::content_uri_is_app_owned(path, sent, received, &trees, &exported);
            harness.check(
                &format!("content_uri_ownership_{name}"),
                actual == expected,
                format!("actual={actual} expected={expected}"),
            );
        }
    }

    // ── outbox FileProvider 映射（静态/隔离证据） ──
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let main_activity = std::fs::read_to_string(
            manifest_dir.join("gen/android/app/src/main/java/com/lanchat/app/MainActivity.kt"),
        )
        .unwrap_or_default();
        let provider = std::fs::read_to_string(
            manifest_dir.join("gen/android/app/src/main/java/com/lanchat/app/FdContentProvider.kt"),
        )
        .unwrap_or_default();
        let file_paths = std::fs::read_to_string(
            manifest_dir.join("gen/android/app/src/main/res/xml/file_paths.xml"),
        )
        .unwrap_or_default();
        let resolve_uses_outbox = main_activity.contains("outboxFileUri(file)")
            && main_activity
                .find("storedReceivedFileUri(file)")
                .zip(main_activity.find("FileProvider.getUriForFile"))
                .map(|(stored, fallback)| {
                    main_activity
                        .find("?: outboxFileUri(file)")
                        .map(|outbox| stored < outbox && outbox < fallback)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        let outbox_root_is_data_dir = main_activity.contains("File(applicationInfo.dataDir, \"outbox\")")
            && provider.contains("File(appContext.applicationInfo.dataDir, \"outbox\")");
        let provider_is_restricted = provider.contains("if (msgIdStr == \"outbox\")")
            && provider.contains("Outbox files are read-only")
            && provider.contains("if (file.parentFile != root || !file.isFile)")
            && !file_paths.contains("root-path")
            && !file_paths.contains("path=\".\" /><root");
        harness.check(
            "outbox_provider_mapping_restricted",
            resolve_uses_outbox && outbox_root_is_data_dir && provider_is_restricted,
            format!(
                "resolve={} root={} restricted={} file_paths_has_root_path={}",
                resolve_uses_outbox,
                outbox_root_is_data_dir,
                provider_is_restricted,
                file_paths.contains("root-path")
            ),
        );
    }

    // ── review04：应用 private 目录里的“原始发送源”不得被清理（F1/F8） ──
    {
        let root = temp_dir("cleanup-private-source");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        // 与设备复现一致：/data/user/0/<pkg>/files/codex-qa-original-source.bin（这里用临时 app data 根模拟）
        let private_source = root.join("codex-qa-original-source.bin");
        std::fs::write(&private_source, b"independent-user-source-37-bytes!").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "codex-qa-20261017", &private_source).await;
        let outcome = storage_ops::cleanup_chat_files(
            &pool,
            "me",
            "codex-qa-20261017",
            Some(&root),
            Some(&staging),
            Some(&staging),
        )
        .await
        .expect("清理失败");
        harness.check(
            "cleanup_keeps_private_original_source",
            private_source.exists() && outcome.report.deleted == 0 && outcome.report.kept >= 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── review04：删除设备同样保留 private 原始发送源（F8 共用同一清理入口） ──
    {
        let root = temp_dir("cleanup-private-source-device");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let private_source = root.join("device-qa-original.bin");
        std::fs::write(&private_source, b"device-user-source").unwrap();
        let received = staging.join("from-phone.bin");
        std::fs::write(&received, b"received-copy").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "codex-qa-delete", &private_source).await;
        insert_message(&pool, "codex-qa-delete", "me", &received).await;
        let outcome = storage_ops::cleanup_chat_files(
            &pool,
            "me",
            "codex-qa-delete",
            Some(&root),
            Some(&staging),
            Some(&staging),
        )
        .await
        .expect("清理失败");
        harness.check(
            "delete_device_keeps_private_sent_source",
            private_source.exists()
                && !received.exists()
                && outcome.report.deleted == 1
                && outcome.report.kept >= 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── review04：应用自建 outbox 副本仍然会被清理（不能过度保护） ──
    {
        let root = temp_dir("cleanup-outbox-copy");
        let staging = root.join("staging");
        let outbox = root.join("outbox");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&outbox).unwrap();
        let copy = outbox.join("uuid_failed-send.apk");
        std::fs::write(&copy, b"app-made-copy").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "iqoo", &copy).await;
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_removes_outbox_sent_copy",
            !copy.exists() && outcome.report.deleted == 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── review04：应用数据目录内的旧接收文件（received_files）仍需清理 ──
    {
        let root = temp_dir("cleanup-legacy-received");
        let staging = root.join("staging");
        let legacy = root.join("received_files");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        let received = legacy.join("old-received.apk");
        std::fs::write(&received, b"legacy-received").unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &received).await;
        let outcome =
            storage_ops::cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .expect("清理失败");
        harness.check(
            "cleanup_removes_legacy_received_in_app_data",
            !received.exists() && outcome.report.deleted == 1,
            format!("report={:?}", outcome.report),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    // ── 清除离线：复用单设备删除语义（先清文件、后删记录；失败则整台保留） ──
    {
        let root = temp_dir("clear-offline");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        // 设备 A：文件可删 → 记录应被清空。
        let ok_file = staging.join("offline-a.apk");
        std::fs::write(&ok_file, b"offline-a").unwrap();
        // 设备 B：目标是一个目录（删除必然失败）→ 记录与文件都应保留。
        let blocked = staging.join("offline-b-blocked.apk");
        std::fs::create_dir_all(&blocked).unwrap();
        let pool = fixture_pool(&root).await;
        insert_message(&pool, "offline-a", "me", &ok_file).await;
        insert_message(&pool, "offline-b", "me", &blocked).await;

        let mut cleared = Vec::new();
        let mut kept = Vec::new();
        for peer in ["offline-a", "offline-b"] {
            let outcome = storage_ops::cleanup_peer_chat_files(&pool, "me", peer).await.unwrap();
            if outcome.report.failed() {
                kept.push(peer.to_string());
                continue;
            }
            db::delete_user_and_history(&pool, "me", peer).await.unwrap();
            cleared.push(peer.to_string());
        }
        let remaining_a: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE from_id = 'offline-a' OR to_id = 'offline-a'")
            .fetch_one(&pool).await.unwrap();
        let remaining_b: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE from_id = 'offline-b' OR to_id = 'offline-b'")
            .fetch_one(&pool).await.unwrap();
        harness.check(
            "clear_offline_cleans_files_and_records",
            cleared == vec!["offline-a".to_string()]
                && kept == vec!["offline-b".to_string()]
                && remaining_a.0 == 0
                && remaining_b.0 > 0
                && !ok_file.exists()
                && blocked.exists(),
            format!("cleared={:?} kept={:?} a={} b={}", cleared, kept, remaining_a.0, remaining_b.0),
        );
        pool.close().await;
        std::fs::remove_dir_all(&root).ok();
    }

    println!();
    println!("合计 {} 项，通过 {} 项，失败 {} 项", harness.passed + harness.failed, harness.passed, harness.failed);
    if harness.failed > 0 {
        std::process::exit(1);
    }
}
