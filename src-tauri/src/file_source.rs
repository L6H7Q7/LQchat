//! Provider metadata is a hint; only the opened source defines upload length.
pub fn size_from_metadata(metadata: &std::fs::Metadata, declared: usize) -> Result<usize, String> {
    if !metadata.is_file() {
        return Err("无法确定文件流的实际大小，请先保存到本地再发送".to_string());
    }
    let actual = usize::try_from(metadata.len()).map_err(|_| "文件过大，无法发送".to_string())?;
    if actual == 0 {
        return Err("文件为空，无法发送".to_string());
    }
    if actual != declared {
        eprintln!("[FileSource] 修正文件大小: 登记 {declared} 字节，实际 {actual} 字节");
    }
    Ok(actual)
}

/// Fill a whole chunk even when a provider returns short reads. Check EOF before
/// sending the final chunk, while the receiver has not yet marked it complete.
pub async fn read_upload_chunk<R: tokio::io::AsyncRead + Unpin>(
    source: &mut R,
    expected: usize,
    last: bool,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut bytes = vec![0; expected];
    let mut read = 0;
    while read < expected {
        let n = source
            .read(&mut bytes[read..])
            .await
            .map_err(|error| format!("读取文件失败: {error}"))?;
        if n == 0 {
            return Err(format!("文件读取长度不符: 已读取 {read}/{expected} 字节"));
        }
        read += n;
    }
    if last {
        let mut extra = [0; 1];
        if source
            .read(&mut extra)
            .await
            .map_err(|error| format!("确认文件结尾失败: {error}"))?
            != 0
        {
            return Err("上传期间文件大小发生变化，请重新发送".to_string());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn stale_size_does_not_truncate_or_overread_source() {
        let path = std::env::temp_dir().join(format!("lqchat-size-test-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&[7u8; 2559]).unwrap();
        drop(file);
        for declared in [10476, 100, 0, 2559] {
            let mut source = std::fs::File::open(&path).unwrap();
            let size = size_from_metadata(&source.metadata().unwrap(), declared).unwrap();
            let mut bytes = vec![0; size];
            source.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, vec![7; 2559]);
            assert_eq!(source.read(&mut [0]).unwrap(), 0);
        }
        // A later modification must be reflected when reopening, including an empty source.
        std::fs::write(&path, [9u8; 512]).unwrap();
        assert_eq!(
            size_from_metadata(
                &std::fs::File::open(&path).unwrap().metadata().unwrap(),
                2559
            )
            .unwrap(),
            512
        );
        std::fs::write(&path, []).unwrap();
        assert!(size_from_metadata(&std::fs::metadata(&path).unwrap(), 2559).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn directory_is_not_a_sized_upload_source() {
        assert!(size_from_metadata(&std::fs::metadata(std::env::temp_dir()).unwrap(), 10).is_err());
    }

    #[tokio::test]
    async fn rejects_truncated_or_grown_source_before_final_chunk_is_sent() {
        let mut truncated = std::io::Cursor::new(vec![1; 3]);
        assert!(read_upload_chunk(&mut truncated, 4, true)
            .await
            .unwrap_err()
            .contains("3/4"));
        let mut grown = std::io::Cursor::new(vec![1; 5]);
        assert!(read_upload_chunk(&mut grown, 4, true).await.is_err());
    }

    #[tokio::test]
    async fn short_provider_reads_are_assembled_without_losing_bytes() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, mut reader) = tokio::io::duplex(2);
        let task = tokio::spawn(async move {
            for byte in 0..17u8 {
                writer.write_all(&[byte]).await.unwrap();
            }
        });
        let first = read_upload_chunk(&mut reader, 10, false).await.unwrap();
        let last = read_upload_chunk(&mut reader, 7, true).await.unwrap();
        assert_eq!([first, last].concat(), (0..17u8).collect::<Vec<_>>());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn observed_jpg_sizes_and_multichunk_sources_transfer_complete_bytes() {
        let path =
            std::env::temp_dir().join(format!("lqchat-jpg-regression-{}", std::process::id()));
        let chunk_size = 10 * 1024 * 1024;
        for (declared, actual) in [
            (10468948, 2624909),
            (10476447, 2559098),
            (10, chunk_size + 17),
        ] {
            let contents: Vec<u8> = (0..actual).map(|index| (index % 251) as u8).collect();
            std::fs::write(&path, &contents).unwrap();
            let source = std::fs::File::open(&path).unwrap();
            let size = size_from_metadata(&source.metadata().unwrap(), declared).unwrap();
            let mut source = tokio::fs::File::from_std(source);
            let mut received = Vec::new();
            while received.len() < size {
                let expected = chunk_size.min(size - received.len());
                let last = received.len() + expected == size;
                received.extend(
                    read_upload_chunk(&mut source, expected, last)
                        .await
                        .unwrap(),
                );
            }
            assert_eq!(size, actual);
            assert_eq!(received, contents);
        }
        std::fs::remove_file(path).unwrap();
    }
}
