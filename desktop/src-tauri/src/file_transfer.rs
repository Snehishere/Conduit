use base64::{engine::general_purpose::STANDARD as B64, Engine};
use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const CHUNK_SIZE: usize = 64 * 1024; // 64KB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferInfo {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub from_device: String,
    pub to_device: String,
    pub status: String,
    pub chunks_received: u32,
    pub total_chunks: u32,
    pub saved_path: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug)]
#[allow(dead_code)]
struct IncomingTransfer {
    name: String,
    size: u64,
    mime: String,
    from_device: String,
    total_chunks: u32,
    chunks: HashMap<u32, Vec<u8>>,
    received: u32,
    temp_dir: PathBuf,
}

#[derive(Debug)]
#[allow(dead_code)]
struct OutgoingTransfer {
    name: String,
    data: Vec<u8>,
    mime: String,
    to_device: String,
    total_chunks: u32,
    next_chunk: u32,
}

pub struct FileTransferEngine {
    incoming: Arc<RwLock<HashMap<String, IncomingTransfer>>>,
    outgoing: Arc<RwLock<HashMap<String, OutgoingTransfer>>>,
    downloads_dir: PathBuf,
    temp_dir: PathBuf,
}

impl FileTransferEngine {
    pub fn new() -> Self {
        let downloads_dir = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Conduit");
        std::fs::create_dir_all(&downloads_dir).ok();

        let temp_dir = std::env::temp_dir().join("conduit_chunks");
        std::fs::create_dir_all(&temp_dir).ok();

        info!("File downloads directory: {}", downloads_dir.display());
        info!("Chunk temp directory: {}", temp_dir.display());

        FileTransferEngine {
            incoming: Arc::new(RwLock::new(HashMap::new())),
            outgoing: Arc::new(RwLock::new(HashMap::new())),
            downloads_dir,
            temp_dir,
        }
    }

    pub fn get_downloads_path(&self) -> String {
        self.downloads_dir.to_string_lossy().to_string()
    }

    pub async fn start_incoming(
        &self,
        id: &str,
        name: &str,
        size: u64,
        mime: &str,
        from_device: &str,
    ) {
        let total_chunks = (size as usize).div_ceil(CHUNK_SIZE) as u32;
        let transfer_dir = self.temp_dir.join(id);
        std::fs::create_dir_all(&transfer_dir).ok();

        let transfer = IncomingTransfer {
            name: name.to_string(),
            size,
            mime: mime.to_string(),
            from_device: from_device.to_string(),
            total_chunks,
            chunks: HashMap::new(),
            received: 0,
            temp_dir: transfer_dir,
        };
        self.incoming.write().await.insert(id.to_string(), transfer);
        info!("Incoming file transfer started: {} ({} chunks)", name, total_chunks);
    }

    pub async fn receive_chunk(
        &self,
        id: &str,
        index: u32,
        data_b64: &str,
    ) -> Result<u32, String> {
        let data = B64.decode(data_b64).map_err(|e| format!("Invalid base64: {}", e))?;

        let mut incoming = self.incoming.write().await;
        let transfer = incoming
            .get_mut(id)
            .ok_or_else(|| format!("Unknown transfer: {}", id))?;

        // Persist chunk to disk for resume support
        let chunk_path = transfer.temp_dir.join(format!("chunk_{}", index));
        tokio::fs::write(&chunk_path, &data).await.ok();

        transfer.chunks.insert(index, data);
        transfer.received += 1;

        Ok(transfer.received)
    }

    /// Resume an incoming transfer by loading persisted chunks from disk
    pub async fn resume_incoming(
        &self,
        id: &str,
        name: &str,
        size: u64,
        mime: &str,
        from_device: &str,
    ) -> Result<u32, String> {
        let total_chunks = (size as usize).div_ceil(CHUNK_SIZE) as u32;        let transfer_dir = self.temp_dir.join(id);

        let mut received = 0u32;
        let mut chunks = HashMap::new();

        // Load any persisted chunks from disk
        if transfer_dir.exists() {
            for i in 0..total_chunks {
                let chunk_path = transfer_dir.join(format!("chunk_{}", i));
                if let Ok(data) = std::fs::read(&chunk_path) {
                    chunks.insert(i, data);
                    received += 1;
                }
            }
        }

        std::fs::create_dir_all(&transfer_dir).ok();

        let transfer = IncomingTransfer {
            name: name.to_string(),
            size,
            mime: mime.to_string(),
            from_device: from_device.to_string(),
            total_chunks,
            chunks,
            received,
            temp_dir: transfer_dir,
        };
        self.incoming.write().await.insert(id.to_string(), transfer);
        info!("Resumed incoming transfer: {} ({} of {} chunks loaded from disk)", name, received, total_chunks);
        Ok(received)
    }

    pub async fn is_complete(&self, id: &str) -> bool {
        let incoming = self.incoming.read().await;
        if let Some(transfer) = incoming.get(id) {
            transfer.received >= transfer.total_chunks
        } else {
            false
        }
    }

    pub async fn finalize_incoming(&self, id: &str) -> Result<String, String> {
        let mut incoming = self.incoming.write().await;
        let transfer = incoming
            .remove(id)
            .ok_or_else(|| format!("Unknown transfer: {}", id))?;

        let mut file_data = Vec::with_capacity(transfer.size as usize);
        for i in 0..transfer.total_chunks {
            let chunk = transfer
                .chunks
                .get(&i)
                .ok_or_else(|| format!("Missing chunk {}", i))?;
            file_data.extend_from_slice(chunk);
        }

        let safe_name = sanitize_filename(&transfer.name);
        let mut save_path = self.downloads_dir.join(&safe_name);

        // Avoid overwriting existing files
        let mut counter = 1;
        while save_path.exists() {
            let stem = save_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file");
            let ext = save_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| format!(".{}", e))
                .unwrap_or_default();
            save_path = self.downloads_dir.join(format!("{} ({}){}", stem, counter, ext));
            counter += 1;
        }

        tokio::fs::write(&save_path, &file_data)
            .await
            .map_err(|e| format!("Failed to write file: {}", e))?;

        // Clean up temp chunk directory
        let _ = std::fs::remove_dir_all(&transfer.temp_dir);

        let path_str = save_path.to_string_lossy().to_string();
        info!("File saved: {} ({} bytes)", path_str, file_data.len());
        Ok(path_str)
    }

    pub async fn cancel_incoming(&self, id: &str) {
        if let Some(transfer) = self.incoming.write().await.remove(id) {
            // Clean up temp chunk files
            let _ = std::fs::remove_dir_all(&transfer.temp_dir);
            info!("Cancelled incoming transfer: {} (temp files cleaned)", id);
        }
    }

    pub async fn start_outgoing(
        &self,
        id: &str,
        file_path: &str,
        to_device: &str,
    ) -> Result<(u64, String, String, u32), String> {
        let path = PathBuf::from(file_path);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let data = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let size = data.len() as u64;
        let mime = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        let total_chunks = data.len().div_ceil(CHUNK_SIZE) as u32;

        let transfer = OutgoingTransfer {
            name: name.clone(),
            data,
            mime: mime.clone(),
            to_device: to_device.to_string(),
            total_chunks,
            next_chunk: 0,
        };
        self.outgoing.write().await.insert(id.to_string(), transfer);
        info!("Outgoing file transfer started: {} → {} ({} chunks)", name, to_device, total_chunks);

        Ok((size, name, mime, total_chunks))
    }

    pub async fn get_next_chunk(&self, id: &str) -> Option<(u32, u32, String)> {
        let mut outgoing = self.outgoing.write().await;
        let transfer = outgoing.get_mut(id)?;

        if transfer.next_chunk >= transfer.total_chunks {
            return None;
        }

        let idx = transfer.next_chunk;
        let start = (idx as usize) * CHUNK_SIZE;
        let end = std::cmp::min(start + CHUNK_SIZE, transfer.data.len());
        let chunk_data = &transfer.data[start..end];
        let data_b64 = B64.encode(chunk_data);

        transfer.next_chunk += 1;
        Some((idx, transfer.total_chunks, data_b64))
    }

    pub async fn remove_outgoing(&self, id: &str) {
        self.outgoing.write().await.remove(id);
    }
}

fn sanitize_filename(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' || c == '\\' || c == ':' || c == '*' || c == '?' || c == '"' || c == '<' || c == '>' || c == '|' {
            result.push('_');
            i += 1;
        } else if c == '.' {
            let start = i;
            while i < chars.len() && chars[i] == '.' {
                i += 1;
            }
            let run_len = i - start;
            let is_extension = run_len == 1
                && chars.get(i).is_some_and(|next| next.is_alphanumeric());
            if is_extension {
                result.push('.');
            } else {
                result.push('_');
            }
        } else {
            result.push(c);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("../../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("my file (1).png"), "my file (1).png");
    }
}
